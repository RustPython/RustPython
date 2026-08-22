//! Cross-interpreter object sharing and exception snapshots (XI data).
//!
//! Interpreters do not share `PyObject` graphs. Shareable values are converted
//! to an interpreter-neutral payload and rebuilt in the destination.

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyBytes, PyCode, PyFloat, PyFunction, PyInt, PyMemoryView, PyStr,
        PyTuple,
    },
    bytecode::{BorrowedConstant, Constant, Instruction, oparg::OpArgState},
    protocol::PyBuffer,
};
use malachite_bigint::BigInt;
use rustpython_common::wtf8::Wtf8Buf;

/// Unbound-item ops (`UNBOUND_REMOVE` / `UNBOUND_ERROR` / `UNBOUND_REPLACE`).
pub const UNBOUND_REMOVE: i32 = 1;
pub const UNBOUND_ERROR: i32 = 2;
pub const UNBOUND_REPLACE: i32 = 3;

/// `xidata_fallback_t`: what to do with an object that has no XI data support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// `_PyXIDATA_XIDATA_ONLY`: raise `NotShareableError`.
    XidataOnly = 0,
    /// `_PyXIDATA_FULL_FALLBACK`: try stateless functions, then pickle.
    Full = 1,
}

impl Fallback {
    #[must_use]
    pub const fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::XidataOnly,
            1 => Self::Full,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Interpreter-neutral payload that can be materialized in any interpreter
/// sharing the process-wide [`crate::Context`].
#[derive(Clone)]
pub enum SharedValue {
    None,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Bytes(Vec<u8>),
    Str(Wtf8Buf),
    Tuple(Vec<Self>),
    Channel {
        cid: i64,
        end: i32,
    },
    #[cfg(feature = "threading")]
    Buffer(PyBuffer),
    #[cfg(not(feature = "threading"))]
    Buffer(Vec<u8>),
    /// Marshalled code object.
    Code(Vec<u8>),
    /// Marshalled code of a stateless function; rebuilt against `__main__`.
    Function(Vec<u8>),
    /// `pickle.dumps` output, used by [`Fallback::Full`].
    Pickled(Vec<u8>),
}

impl SharedValue {
    pub fn from_object(obj: &PyObject, fallback: Fallback, vm: &VirtualMachine) -> PyResult<Self> {
        match Self::basic_from_object(obj, fallback, vm) {
            Ok(v) => Ok(v),
            Err(exc) => {
                if fallback == Fallback::XidataOnly {
                    return Err(exc);
                }
                if obj.downcastable::<PyFunction>()
                    && let Ok(v) = Self::from_function(obj, vm)
                {
                    return Ok(v);
                }
                // We could try marshal here but we don't for now.
                match pickle_dumps(obj, vm) {
                    Ok(data) => Ok(Self::Pickled(data)),
                    // Raise the original exception.
                    Err(_) => Err(exc),
                }
            }
        }
    }

    /// The registered "getdata" functions, i.e. no pickle fallback.
    fn basic_from_object(
        obj: &PyObject,
        fallback: Fallback,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        if vm.is_none(obj) {
            return Ok(Self::None);
        }
        let cls = obj.class();
        if cls.is(vm.ctx.types.bool_type) {
            return Ok(Self::Bool(obj.to_owned().is_true(vm)?));
        }
        if cls.is(vm.ctx.types.int_type) {
            let n = obj.downcast_ref::<PyInt>().unwrap();
            return Ok(Self::Int(n.as_bigint().clone()));
        }
        if cls.is(vm.ctx.types.float_type) {
            let f = obj.downcast_ref::<PyFloat>().unwrap();
            return Ok(Self::Float(f.to_f64()));
        }
        if cls.is(vm.ctx.types.bytes_type) {
            let b = obj.downcast_ref::<PyBytes>().unwrap();
            return Ok(Self::Bytes(b.as_bytes().to_vec()));
        }
        if cls.is(vm.ctx.types.str_type) {
            let s = obj.downcast_ref::<PyStr>().unwrap();
            return Ok(Self::Str(s.as_wtf8().to_owned()));
        }
        if cls.is(vm.ctx.types.tuple_type) {
            let t = obj.downcast_ref::<PyTuple>().unwrap();
            let mut items = Vec::with_capacity(t.len());
            for item in t {
                items.push(Self::from_object(item, fallback, vm)?);
            }
            return Ok(Self::Tuple(items));
        }
        // Registered by the `_interpreters` module for the builtin memoryview.
        if cls.is(vm.ctx.types.memoryview_type) {
            return Self::from_buffer_object(obj, vm);
        }
        // Registered by the `_interpchannels` module for ChannelID.
        if let Some(ch) = crate::stdlib::_interpchannels::channel_id_parts(obj) {
            return Ok(Self::Channel {
                cid: ch.0,
                end: ch.1,
            });
        }
        Err(not_shareable(vm, obj))
    }

    /// `_PyFunction_GetXIData`: only stateless functions are shareable.
    fn from_function(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        let func = obj.downcast_ref::<PyFunction>().ok_or_else(|| {
            not_shareable_error(
                vm,
                format!("expected a function, got {}", render_repr(obj, vm)),
            )
        })?;
        verify_stateless_function(func, vm)
            .map_err(|_| not_shareable_error(vm, "only stateless functions are shareable"))?;
        let code = (*func.code).to_owned();
        Ok(Self::Function(marshal_dumps(code.as_object(), vm)?))
    }

    /// `_PyCode_GetXIData`.
    pub fn from_code(code: &Py<PyCode>, vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self::Code(marshal_dumps(code.as_object(), vm)?))
    }

    pub fn from_buffer_object(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        let view = PyMemoryView::from_object(obj, vm)?;
        #[cfg(feature = "threading")]
        {
            Ok(Self::Buffer(view.clone_buffer()))
        }
        #[cfg(not(feature = "threading"))]
        {
            let buf = view.clone_buffer();
            let bytes = buf.as_contiguous().map(|b| b.to_vec()).unwrap_or_default();
            Ok(Self::Buffer(bytes))
        }
    }

    pub fn into_object(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::None => Ok(vm.ctx.none()),
            Self::Bool(b) => Ok(vm.ctx.new_bool(b).into()),
            Self::Int(n) => Ok(vm.ctx.new_int(n).into()),
            Self::Float(f) => Ok(vm.ctx.new_float(f).into()),
            Self::Bytes(b) => Ok(vm.ctx.new_bytes(b).into()),
            Self::Str(s) => Ok(vm.ctx.new_str(s).into()),
            Self::Tuple(items) => {
                let mut els = Vec::with_capacity(items.len());
                for item in items {
                    els.push(item.into_object(vm)?);
                }
                Ok(vm.ctx.new_tuple(els).into())
            }
            Self::Channel { cid, end } => {
                crate::stdlib::_interpchannels::channel_id_from_parts(cid, end, false, false, vm)
            }
            Self::Buffer(buffer) => {
                #[cfg(not(feature = "threading"))]
                let buffer = PyBuffer::from_byte_vector(buffer, vm);
                // _memoryview_from_xid
                let view = crate::stdlib::_interpreters::xibufferview_from_buffer(buffer, vm);
                let mv = PyMemoryView::from_object(&view, vm)?;
                Ok(mv.into_pyobject(vm))
            }
            Self::Code(data) => marshal_loads(&data, vm),
            Self::Function(data) => {
                let code = marshal_loads(&data, vm)?
                    .downcast::<PyCode>()
                    .map_err(|_| vm.new_type_error("expected code"))?;
                // Stateless functions have no globals, so `__main__` is used,
                // just like for builtins such as exec().
                let globals = vm.main_namespace()?;
                Ok(PyFunction::new(code, globals, vm)?.into_pyobject(vm))
            }
            Self::Pickled(data) => pickle_loads(&data, vm),
        }
    }
}

fn marshal_dumps(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let dumps = vm.import("marshal", 0)?.get_attr("dumps", vm)?;
    let bytes = dumps.call((obj.to_owned(),), vm)?;
    let bytes = bytes
        .downcast::<PyBytes>()
        .map_err(|_| vm.new_type_error("marshal.dumps() did not return bytes"))?;
    Ok(bytes.as_bytes().to_vec())
}

fn marshal_loads(data: &[u8], vm: &VirtualMachine) -> PyResult {
    let loads = vm.import("marshal", 0)?.get_attr("loads", vm)?;
    loads.call((vm.ctx.new_bytes(data.to_vec()),), vm)
}

fn pickle_dumps(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let dumps = vm.import("pickle", 0)?.get_attr("dumps", vm)?;
    let bytes = dumps.call((obj.to_owned(),), vm)?;
    let bytes = bytes
        .downcast::<PyBytes>()
        .map_err(|_| vm.new_type_error("pickle.dumps() did not return bytes"))?;
    Ok(bytes.as_bytes().to_vec())
}

fn pickle_loads(data: &[u8], vm: &VirtualMachine) -> PyResult {
    let loads = vm.import("pickle", 0)?.get_attr("loads", vm)?;
    loads
        .call((vm.ctx.new_bytes(data.to_vec()),), vm)
        .map_err(|_| not_shareable_error(vm, "object could not be unpickled"))
}

fn not_shareable_error(vm: &VirtualMachine, msg: impl Into<String>) -> PyBaseExceptionRef {
    crate::stdlib::_interpreters::not_shareable_error(vm, msg)
}

fn not_shareable(vm: &VirtualMachine, obj: &PyObject) -> PyBaseExceptionRef {
    let rendered = obj
        .str(vm)
        .map_or_else(|_| obj.class().name().to_string(), |s| s.to_string());
    not_shareable_error(
        vm,
        format!("{rendered} does not support cross-interpreter data"),
    )
}

fn render_repr(obj: &PyObject, vm: &VirtualMachine) -> String {
    obj.repr(vm)
        .map_or_else(|_| obj.class().name().to_string(), |s| s.to_string())
}

/// Exact-type shareable check used by `_interpreters.is_shareable`.
///
/// This mirrors `_PyObject_CheckXIData`: only the type's registration is
/// consulted, so a tuple is "shareable" even when its items are not.
pub fn is_shareable(obj: &PyObject, vm: &VirtualMachine) -> bool {
    if vm.is_none(obj) {
        return true;
    }
    let cls = obj.class();
    cls.is(vm.ctx.types.bool_type)
        || cls.is(vm.ctx.types.int_type)
        || cls.is(vm.ctx.types.float_type)
        || cls.is(vm.ctx.types.bytes_type)
        || cls.is(vm.ctx.types.str_type)
        || cls.is(vm.ctx.types.tuple_type)
        || cls.is(vm.ctx.types.memoryview_type)
        || crate::stdlib::_interpchannels::channel_id_parts(obj).is_some()
}

/// Encode a namespace key as UTF-8, rejecting non-strings and lone surrogates.
pub fn utf8_key<'a>(key: &'a PyObject, vm: &'a VirtualMachine) -> PyResult<&'a str> {
    let s = key
        .downcast_ref::<PyStr>()
        .ok_or_else(|| vm.new_type_error("bad argument type for built-in operation"))?;
    s.as_wtf8().as_str().map_err(|_| {
        vm.new_unicode_encode_error_real(
            vm.ctx.new_str("utf-8"),
            s.to_owned(),
            0,
            s.char_len(),
            vm.ctx.new_str("surrogates not allowed"),
        )
    })
}

/// `_convert_exc_to_TracebackException` followed by `_format_TracebackException`.
fn format_traceback_exception(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> PyResult<String> {
    let create = vm
        .import("traceback", 0)?
        .get_attr("TracebackException", vm)?
        .get_attr("from_exception", vm)?;
    let kwargs: crate::function::KwArgs = [
        (
            "save_exc_type".to_owned(),
            vm.ctx.false_value.clone().into(),
        ),
        ("lookup_lines".to_owned(), vm.ctx.false_value.clone().into()),
    ]
    .into_iter()
    .collect();
    let tbexc = create.call(
        crate::function::FuncArgs::new(vec![exc.as_object().to_owned()], kwargs),
        vm,
    )?;
    let lines = vm.call_method(&tbexc, "format", ())?;
    let empty: PyObjectRef = vm.ctx.empty_str.to_owned().into();
    let mut formatted = vm
        .call_method(&empty, "join", (lines,))?
        .str(vm)?
        .to_string();
    // Remove a trailing newline if needed.
    if formatted.ends_with('\n') {
        formatted.pop();
    }
    Ok(formatted)
}

/// Snapshot of an exception that can be rebuilt in another interpreter.
pub struct ExcInfo {
    pub type_name: String,
    pub type_qualname: String,
    pub type_module: String,
    pub msg: Option<String>,
    pub formatted: String,
    pub errdisplay: String,
}

impl ExcInfo {
    pub fn capture(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> Self {
        let cls = exc.class();
        let type_name = cls.name().to_owned();
        let type_qualname = cls
            .as_object()
            .get_attr("__qualname__", vm)
            .ok()
            .and_then(|o| o.str(vm).ok())
            .map_or_else(|| type_name.clone(), |s| s.to_string());
        let type_module = cls
            .as_object()
            .get_attr("__module__", vm)
            .ok()
            .and_then(|o| o.str(vm).ok())
            .map_or_else(|| "builtins".to_owned(), |s| s.to_string());
        let msg = exc.as_object().str(vm).ok().map(|s| s.to_string());
        let msg = msg.filter(|m| !m.is_empty());
        let formatted = match &msg {
            Some(m) => format!("{type_name}: {m}"),
            None => type_name.clone(),
        };
        let errdisplay = format_traceback_exception(exc, vm).unwrap_or_else(|_| {
            let mut buf = String::new();
            let _ = vm.write_exception(&mut buf, exc);
            buf
        });
        Self {
            type_name,
            type_qualname,
            type_module,
            msg,
            formatted,
            errdisplay,
        }
    }

    pub fn into_namespace(self, vm: &VirtualMachine) -> PyObjectRef {
        let type_ns = crate::py_namespace!(vm, {
            "__name__" => vm.ctx.new_str(self.type_name),
            "__qualname__" => vm.ctx.new_str(self.type_qualname),
            "__module__" => vm.ctx.new_str(self.type_module),
        });
        let msg_obj = match self.msg {
            Some(m) => vm.ctx.new_str(m).into(),
            None => vm.ctx.none(),
        };
        crate::py_namespace!(vm, {
            "type" => type_ns,
            "msg" => msg_obj,
            "formatted" => vm.ctx.new_str(self.formatted),
            "errdisplay" => vm.ctx.new_str(self.errdisplay),
        })
        .into()
    }
}

/// `_PyCode_VerifyStateless` for the non-pure case: reject closures.
fn verify_stateless(code: &PyCode, vm: &VirtualMachine) -> PyResult<()> {
    if !code.freevars.is_empty() {
        return Err(vm.new_value_error("closures not supported"));
    }
    Ok(())
}

fn verify_stateless_function(func: &PyFunction, vm: &VirtualMachine) -> PyResult<()> {
    if func.closure.is_some() {
        return Err(vm.new_value_error("closures not supported"));
    }
    verify_stateless(&func.code, vm)
}

/// `verify_script`: a script takes no arguments and returns only None.
pub fn verify_script(code: &PyCode, vm: &VirtualMachine) -> PyResult<()> {
    verify_stateless(code, vm)?;
    if code.arg_count > 0
        || code.posonlyarg_count > 0
        || code.kwonlyarg_count > 0
        || code.flags.contains(crate::bytecode::CodeFlags::VARARGS)
        || code.flags.contains(crate::bytecode::CodeFlags::VARKEYWORDS)
    {
        return Err(vm.new_value_error("code with args not supported"));
    }
    if !code_returns_only_none(code) {
        return Err(vm.new_value_error("code that returns a value is not a script"));
    }
    Ok(())
}

fn code_returns_only_none(code: &PyCode) -> bool {
    let mut arg_state = OpArgState::default();
    let mut prev_none = false;
    for unit in code.instructions.iter().copied() {
        let (op, arg) = arg_state.get(unit);
        match op {
            Instruction::LoadConst { consti } => {
                let cidx = consti.get(arg);
                prev_none = matches!(
                    code.constants[cidx].borrow_constant(),
                    BorrowedConstant::None
                );
            }
            Instruction::ReturnValue | Instruction::InstrumentedReturnValue => {
                if !prev_none {
                    return false;
                }
            }
            Instruction::ExtendedArg => {}
            _ => prev_none = false,
        }
    }
    true
}

/// Extract a script code object from source text / function / code object.
pub fn script_code(obj: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyCode>> {
    let code = if let Ok(code) = obj.to_owned().downcast::<PyCode>() {
        code
    } else if let Some(func) = obj.downcast_ref::<PyFunction>() {
        (*func.code).to_owned()
    } else {
        let src = source_as_string(obj, vm)?;
        // Compile in the caller so SyntaxError is raised here.
        vm.compile(&src, crate::compiler::Mode::Exec, "<script>")
            .map_err(|err| err.into_pyexception(vm, Some(src.as_str())))?
    };
    verify_script(&code, vm)?;
    Ok(code)
}

/// `_Py_SourceAsString` for the objects `_PyObject_SupportedAsScript` accepts:
/// `str`, `bytes`, and other readable buffers.
fn source_as_string(obj: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
    if let Some(s) = obj.downcast_ref::<PyStr>() {
        return s
            .as_wtf8()
            .as_str()
            .map(str::to_owned)
            .map_err(|_| unsupported_script(vm, obj));
    }
    let buf = crate::function::ArgBytesLike::try_from_borrowed_object(vm, obj)
        .map_err(|_| unsupported_script(vm, obj))?;
    buf.with_ref(|bytes| {
        core::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| unsupported_script(vm, obj))
    })
}

fn unsupported_script(vm: &VirtualMachine, obj: &PyObject) -> PyBaseExceptionRef {
    vm.new_type_error(format!("unsupported script {}", render_repr(obj, vm)))
}

pub fn apply_shared_ns(
    ns: &crate::builtins::PyDictRef,
    shared: &crate::builtins::PyDict,
    vm: &VirtualMachine,
) -> PyResult<()> {
    for (key, value) in shared {
        let name = utf8_key(&key, vm)?;
        let shared = SharedValue::from_object(&value, Fallback::XidataOnly, vm)?;
        ns.set_item(name, shared.into_object(vm)?, vm)?;
    }
    Ok(())
}

/// Run `f` in interpreter `id` with exclusive `__main__` occupancy.
#[cfg(feature = "threading")]
pub fn with_interpreter<F, R>(id: i64, caller: &VirtualMachine, f: F) -> PyResult<R>
where
    F: FnOnce(&VirtualMachine) -> PyResult<R>,
{
    let state = crate::vm::runtime::lookup_interpreter(id)
        .ok_or_else(|| crate::stdlib::_interpreters::interpreter_not_found(caller, id))?;
    if !state.ready.load(core::sync::atomic::Ordering::Acquire) {
        return Err(crate::stdlib::_interpreters::interpreter_error(
            caller,
            format!("cannot exec interpreter {id} (not ready)"),
        ));
    }

    if state
        .running_main
        .compare_exchange(
            false,
            true,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        return Err(crate::stdlib::_interpreters::interpreter_error(
            caller,
            "interpreter already running",
        ));
    }

    struct RunningGuard<'a>(&'a crate::vm::PyGlobalState);
    impl Drop for RunningGuard<'_> {
        fn drop(&mut self) {
            self.0
                .running_main
                .store(false, core::sync::atomic::Ordering::Release);
        }
    }
    let _guard = RunningGuard(&state);

    let tvm = crate::vm::runtime::owned_new_thread(id)
        .ok_or_else(|| crate::stdlib::_interpreters::interpreter_not_found(caller, id))?;
    tvm.run(f)
}

#[must_use]
pub fn is_running(id: i64) -> bool {
    crate::vm::runtime::lookup_interpreter(id)
        .is_some_and(|s| s.is_main || s.running_main.load(core::sync::atomic::Ordering::Acquire))
}
