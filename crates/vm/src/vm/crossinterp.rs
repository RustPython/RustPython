//! Cross-interpreter object sharing and exception snapshots (CPython XI data).
//!
//! Interpreters do not share `PyObject` graphs. Shareable values are converted
//! to an interpreter-neutral payload and rebuilt in the destination.

use crate::{
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyBytes, PyCode, PyFloat, PyFunction, PyInt, PyMemoryView, PyStr,
        PyTuple,
    },
    bytecode::{BorrowedConstant, Constant, Instruction, oparg::OpArgState},
    protocol::PyBuffer,
};
use malachite_bigint::BigInt;
use rustpython_common::wtf8::Wtf8Buf;

/// CPython unbound-item ops (`UNBOUND_REMOVE` / `UNBOUND_ERROR` / `UNBOUND`).
pub const UNBOUND_REMOVE: i32 = 1;
pub const UNBOUND_ERROR: i32 = 2;
pub const UNBOUND_REPLACE: i32 = 3;

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
}

impl SharedValue {
    pub fn from_object(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        if vm.is_none(obj) {
            return Ok(Self::None);
        }
        if obj.class().is(vm.ctx.types.bool_type) {
            return Ok(Self::Bool(obj.to_owned().is_true(vm)?));
        }
        if obj.class().is(vm.ctx.types.int_type) {
            let n = obj
                .downcast_ref::<PyInt>()
                .ok_or_else(|| not_shareable(vm, obj))?;
            return Ok(Self::Int(n.as_bigint().clone()));
        }
        if obj.class().is(vm.ctx.types.float_type) {
            let f = obj
                .downcast_ref::<PyFloat>()
                .ok_or_else(|| not_shareable(vm, obj))?;
            return Ok(Self::Float(f.to_f64()));
        }
        if obj.class().is(vm.ctx.types.bytes_type) {
            let b = obj
                .downcast_ref::<PyBytes>()
                .ok_or_else(|| not_shareable(vm, obj))?;
            return Ok(Self::Bytes(b.as_bytes().to_vec()));
        }
        if obj.class().is(vm.ctx.types.str_type) {
            let s = obj
                .downcast_ref::<PyStr>()
                .ok_or_else(|| not_shareable(vm, obj))?;
            return Ok(Self::Str(s.as_wtf8().to_owned()));
        }
        if obj.class().is(vm.ctx.types.tuple_type) {
            let t = obj
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| not_shareable(vm, obj))?;
            let mut items = Vec::with_capacity(t.len());
            for item in t {
                items.push(Self::from_object(item, vm)?);
            }
            return Ok(Self::Tuple(items));
        }
        if let Some(ch) = crate::stdlib::_interpchannels::channel_id_parts(obj) {
            return Ok(Self::Channel {
                cid: ch.0,
                end: ch.1,
            });
        }
        Err(not_shareable(vm, obj))
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
                #[cfg(feature = "threading")]
                {
                    let mv = PyMemoryView::from_buffer(buffer, vm)?;
                    Ok(mv.into_pyobject(vm))
                }
                #[cfg(not(feature = "threading"))]
                {
                    let mv = PyMemoryView::from_buffer(PyBuffer::from_byte_vector(buffer, vm), vm)?;
                    Ok(mv.into_pyobject(vm))
                }
            }
        }
    }
}

fn not_shareable(vm: &VirtualMachine, obj: &PyObject) -> crate::builtins::PyBaseExceptionRef {
    let msg = format!("{} is not shareable", obj.class());
    crate::stdlib::_interpreters::not_shareable_error(vm, msg)
}

/// Exact-type shareable check used by `_interpreters.is_shareable`.
pub fn is_shareable(obj: &PyObject, vm: &VirtualMachine) -> bool {
    if vm.is_none(obj) {
        return true;
    }
    let cls = obj.class();
    if cls.is(vm.ctx.types.bool_type)
        || cls.is(vm.ctx.types.int_type)
        || cls.is(vm.ctx.types.float_type)
        || cls.is(vm.ctx.types.bytes_type)
        || cls.is(vm.ctx.types.str_type)
    {
        return true;
    }
    if cls.is(vm.ctx.types.tuple_type)
        && let Some(t) = obj.downcast_ref::<PyTuple>()
    {
        return t.iter().all(|item| is_shareable(item, vm));
    }
    crate::stdlib::_interpchannels::channel_id_parts(obj).is_some()
}

/// Encode a dict key as UTF-8, rejecting lone surrogates like CPython.
pub fn utf8_key<'a>(key: &'a PyObject, vm: &'a VirtualMachine) -> PyResult<&'a str> {
    let s = key
        .downcast_ref::<PyStr>()
        .ok_or_else(|| vm.new_type_error("attribute name must be a string"))?;
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
        let mut errdisplay = String::new();
        let _ = vm.write_exception(&mut errdisplay, exc);
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

/// Verify a code object is a valid `_interpreters` script.
pub fn verify_script(code: &PyCode, vm: &VirtualMachine) -> PyResult<()> {
    if !code.freevars.is_empty() || !code.cellvars.is_empty() {
        return Err(vm.new_value_error("code with a closure is not a script"));
    }
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

/// Extract a script code object from a str / function / code object.
pub fn script_code(obj: &PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyCode>> {
    if let Ok(code) = obj.to_owned().downcast::<PyCode>() {
        verify_script(&code, vm)?;
        return Ok(code);
    }
    if let Ok(func) = obj.to_owned().downcast::<PyFunction>() {
        let code = (*func.code).to_owned();
        if func.closure.is_some() {
            return Err(vm.new_value_error("code with a closure is not a script"));
        }
        verify_script(&code, vm)?;
        return Ok(code);
    }
    if let Ok(s) = obj.to_owned().downcast::<PyStr>() {
        // Compile in the caller so SyntaxError is raised here, matching CPython.
        let src = s.as_wtf8().as_str().map_err(|_| {
            vm.new_exception_msg(
                vm.ctx.exceptions.unicode_encode_error.to_owned(),
                "surrogates not allowed".to_owned().into(),
            )
        })?;
        let code = vm
            .compile(src, crate::compiler::Mode::Exec, "<script>")
            .map_err(|err| err.into_pyexception(vm, Some(src)))?;
        return Ok(code);
    }
    Err(vm.new_type_error(format!("unsupported script {}", obj.class())))
}

pub fn apply_shared_ns(
    ns: &crate::builtins::PyDictRef,
    shared: &crate::builtins::PyDict,
    vm: &VirtualMachine,
) -> PyResult<()> {
    for (key, value) in shared {
        let name = utf8_key(&key, vm)?;
        let shared = SharedValue::from_object(&value, vm)?;
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

    if !state
        .running_main
        .compare_exchange(
            false,
            true,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
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
