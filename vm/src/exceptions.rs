use crate::common::cell::PyRwLock;
use crate::function::PyFuncArgs;
use crate::obj::objnone::PyNone;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtraceback::PyTracebackRef;
use crate::obj::objtuple::{PyTuple, PyTupleRef};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::py_io::{self, Write};
use crate::pyobject::{
    BorrowValue, PyClassDef, PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult,
    PyValue, TryFromObject, TypeProtocol,
};
use crate::types::create_type_with_slots;
use crate::VirtualMachine;
use crate::{py_serde, sysmodule};

use itertools::Itertools;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader};

use crossbeam_utils::atomic::AtomicCell;

#[pyclass(module = false, name = "BaseException")]
pub struct PyBaseException {
    traceback: PyRwLock<Option<PyTracebackRef>>,
    cause: PyRwLock<Option<PyBaseExceptionRef>>,
    context: PyRwLock<Option<PyBaseExceptionRef>>,
    suppress_context: AtomicCell<bool>,
    args: PyRwLock<PyTupleRef>,
}

impl fmt::Debug for PyBaseException {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("PyBaseException")
    }
}

pub type PyBaseExceptionRef = PyRef<PyBaseException>;

pub trait IntoPyException {
    fn into_pyexception(self, vm: &VirtualMachine) -> PyBaseExceptionRef;
}

impl PyValue for PyBaseException {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.exceptions.base_exception_type.clone()
    }
}

#[pyimpl(flags(BASETYPE, HAS_DICT))]
impl PyBaseException {
    pub(crate) fn new(args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyBaseException {
        PyBaseException {
            traceback: PyRwLock::new(None),
            cause: PyRwLock::new(None),
            context: PyRwLock::new(None),
            suppress_context: AtomicCell::new(false),
            args: PyRwLock::new(PyTuple::from(args).into_ref(vm)),
        }
    }

    #[pyslot]
    fn tp_new(cls: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyBaseException::new(args.args, vm).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__init__")]
    fn init(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        *self.args.write() = PyTuple::from(args.args).into_ref(vm);
        Ok(())
    }

    #[pyproperty]
    pub fn args(&self) -> PyTupleRef {
        self.args.read().clone()
    }

    #[pyproperty(setter)]
    fn set_args(&self, args: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let args = args.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
        *self.args.write() = PyTuple::from(args).into_ref(vm);
        Ok(())
    }

    #[pyproperty(name = "__traceback__")]
    pub fn traceback(&self) -> Option<PyTracebackRef> {
        self.traceback.read().clone()
    }

    #[pyproperty(name = "__traceback__", setter)]
    pub fn set_traceback(&self, traceback: Option<PyTracebackRef>) {
        *self.traceback.write() = traceback;
    }

    #[pyproperty(name = "__cause__")]
    pub fn cause(&self) -> Option<PyBaseExceptionRef> {
        self.cause.read().clone()
    }

    #[pyproperty(name = "__cause__", setter)]
    pub fn set_cause(&self, cause: Option<PyBaseExceptionRef>) {
        *self.cause.write() = cause;
    }

    #[pyproperty(name = "__context__")]
    pub fn context(&self) -> Option<PyBaseExceptionRef> {
        self.context.read().clone()
    }

    #[pyproperty(name = "__context__", setter)]
    pub fn set_context(&self, context: Option<PyBaseExceptionRef>) {
        *self.context.write() = context;
    }

    #[pyproperty(name = "__suppress_context__")]
    fn get_suppress_context(&self) -> bool {
        self.suppress_context.load()
    }

    #[pyproperty(name = "__suppress_context__", setter)]
    fn set_suppress_context(&self, suppress_context: bool) {
        self.suppress_context.store(suppress_context);
    }

    #[pymethod]
    fn with_traceback(zelf: PyRef<Self>, tb: Option<PyTracebackRef>) -> PyResult {
        *zelf.traceback.write() = tb;
        Ok(zelf.as_object().clone())
    }

    #[pymethod(name = "__str__")]
    fn str(&self, vm: &VirtualMachine) -> PyStringRef {
        let str_args = exception_args_as_string(vm, self.args(), true);
        match str_args.into_iter().exactly_one() {
            Err(i) if i.len() == 0 => PyString::from("").into_ref(vm),
            Ok(s) => s,
            Err(i) => PyString::from(format!("({})", i.format(", "))).into_ref(vm),
        }
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> String {
        let repr_args = exception_args_as_string(vm, zelf.args(), false);
        let cls = zelf.lease_class();
        format!("{}({})", cls.name, repr_args.iter().format(", "))
    }
}

/// Print exception chain by calling sys.excepthook
pub fn print_exception(vm: &VirtualMachine, exc: PyBaseExceptionRef) {
    let write_fallback = |exc, errstr| {
        if let Ok(stderr) = sysmodule::get_stderr(vm) {
            let mut stderr = py_io::PyWriter(stderr, vm);
            // if this fails stderr might be closed -- ignore it
            let _ = writeln!(stderr, "{}", errstr);
            let _ = write_exception(&mut stderr, vm, exc);
        } else {
            eprintln!("{}\nlost sys.stderr", errstr);
            let _ = write_exception(&mut io::stderr(), vm, exc);
        }
    };
    if let Ok(excepthook) = vm.get_attribute(vm.sys_module.clone(), "excepthook") {
        let (exc_type, exc_val, exc_tb) = split(exc, vm);
        if let Err(eh_exc) = vm.invoke(&excepthook, vec![exc_type, exc_val, exc_tb]) {
            write_fallback(&eh_exc, "Error in sys.excepthook:");
            write_fallback(&eh_exc, "Original exception was:");
        }
    } else {
        write_fallback(&exc, "missing sys.excepthook");
    }
}

pub fn write_exception<W: Write>(
    output: &mut W,
    vm: &VirtualMachine,
    exc: &PyBaseExceptionRef,
) -> Result<(), W::Error> {
    if let Some(cause) = exc.cause() {
        write_exception(output, vm, &cause)?;
        writeln!(
            output,
            "\nThe above exception was the direct cause of the following exception:\n"
        )?;
    } else if let Some(context) = exc.context() {
        write_exception(output, vm, &context)?;
        writeln!(
            output,
            "\nDuring handling of the above exception, another exception occurred:\n"
        )?;
    }

    write_exception_inner(output, vm, exc)
}

fn print_source_line<W: Write>(
    output: &mut W,
    filename: &str,
    lineno: usize,
) -> Result<(), W::Error> {
    // TODO: use io.open() method instead, when available, according to https://github.com/python/cpython/blob/master/Python/traceback.c#L393
    // TODO: support different encodings
    let file = match File::open(filename) {
        Ok(file) => file,
        Err(_) => return Ok(()),
    };
    let file = BufReader::new(file);

    for (i, line) in file.lines().enumerate() {
        if i + 1 == lineno {
            if let Ok(line) = line {
                // Indented with 4 spaces
                writeln!(output, "    {}", line.trim_start())?;
            }
            return Ok(());
        }
    }

    Ok(())
}

/// Print exception occurrence location from traceback element
fn write_traceback_entry<W: Write>(
    output: &mut W,
    tb_entry: &PyTracebackRef,
) -> Result<(), W::Error> {
    let filename = tb_entry.frame.code.source_path.to_owned();
    writeln!(
        output,
        r##"  File "{}", line {}, in {}"##,
        filename, tb_entry.lineno, tb_entry.frame.code.obj_name
    )?;
    print_source_line(output, &filename, tb_entry.lineno)?;

    Ok(())
}

/// Print exception with traceback
pub fn write_exception_inner<W: Write>(
    output: &mut W,
    vm: &VirtualMachine,
    exc: &PyBaseExceptionRef,
) -> Result<(), W::Error> {
    if let Some(tb) = exc.traceback.read().clone() {
        writeln!(output, "Traceback (most recent call last):")?;
        for tb in tb.iter() {
            write_traceback_entry(output, &tb)?;
        }
    }

    let varargs = exc.args();
    let args_repr = exception_args_as_string(vm, varargs, true);

    let exc_name = exc.lease_class().name.clone();
    match args_repr.len() {
        0 => writeln!(output, "{}", exc_name),
        1 => writeln!(output, "{}: {}", exc_name, args_repr[0]),
        _ => writeln!(
            output,
            "{}: ({})",
            exc_name,
            args_repr.into_iter().format(", ")
        ),
    }
}

fn exception_args_as_string(
    vm: &VirtualMachine,
    varargs: PyTupleRef,
    str_single: bool,
) -> Vec<PyStringRef> {
    let varargs = varargs.borrow_value();
    match varargs.len() {
        0 => vec![],
        1 => {
            let args0_repr = if str_single {
                vm.to_str(&varargs[0])
                    .unwrap_or_else(|_| PyString::from("<element str() failed>").into_ref(vm))
            } else {
                vm.to_repr(&varargs[0])
                    .unwrap_or_else(|_| PyString::from("<element repr() failed>").into_ref(vm))
            };
            vec![args0_repr]
        }
        _ => varargs
            .iter()
            .map(|vararg| {
                vm.to_repr(vararg)
                    .unwrap_or_else(|_| PyString::from("<element repr() failed>").into_ref(vm))
            })
            .collect(),
    }
}

#[derive(Clone)]
pub enum ExceptionCtor {
    Class(PyClassRef),
    Instance(PyBaseExceptionRef),
}

impl TryFromObject for ExceptionCtor {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast::<PyClass>()
            .and_then(|cls| {
                if objtype::issubclass(&cls, &vm.ctx.exceptions.base_exception_type) {
                    Ok(Self::Class(cls))
                } else {
                    Err(cls.into_object())
                }
            })
            .or_else(|obj| obj.downcast::<PyBaseException>().map(Self::Instance))
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "exceptions must be classes or instances deriving from BaseException, not {}",
                    obj.lease_class().name
                ))
            })
    }
}

pub fn invoke(
    cls: PyClassRef,
    args: Vec<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<PyBaseExceptionRef> {
    // TODO: fast-path built-in exceptions by directly instantiating them? Is that really worth it?
    let res = vm.invoke(cls.as_object(), args)?;
    PyBaseExceptionRef::try_from_object(vm, res)
}

impl ExceptionCtor {
    pub fn instantiate(self, vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        match self {
            Self::Class(cls) => invoke(cls, vec![], vm),
            Self::Instance(exc) => Ok(exc),
        }
    }

    pub fn instantiate_value(
        self,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyBaseExceptionRef> {
        let exc_inst = value.clone().downcast::<PyBaseException>().ok();
        match (self, exc_inst) {
            // both are instances; which would we choose?
            (Self::Instance(_exc_a), Some(_exc_b)) => {
                Err(vm
                    .new_type_error("instance exception may not have a separate value".to_owned()))
            }
            // if the "type" is an instance and the value isn't, use the "type"
            (Self::Instance(exc), None) => Ok(exc),
            // if the value is an instance of the type, use the instance value
            (Self::Class(cls), Some(exc)) if objtype::isinstance(&exc, &cls) => Ok(exc),
            // otherwise; construct an exception of the type using the value as args
            (Self::Class(cls), _) => {
                let args = match_class!(match value {
                    PyNone => vec![],
                    tup @ PyTuple => tup.borrow_value().to_vec(),
                    exc @ PyBaseException => exc.args().borrow_value().to_vec(),
                    obj => vec![obj],
                });
                invoke(cls, args, vm)
            }
        }
    }
}

pub fn split(
    exc: PyBaseExceptionRef,
    vm: &VirtualMachine,
) -> (PyObjectRef, PyObjectRef, PyObjectRef) {
    let tb = exc.traceback().map_or(vm.get_none(), |tb| tb.into_object());
    (exc.class().into_object(), exc.into_object(), tb)
}

/// Similar to PyErr_NormalizeException in CPython
pub fn normalize(
    exc_type: PyObjectRef,
    exc_val: PyObjectRef,
    exc_tb: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyBaseExceptionRef> {
    let ctor = ExceptionCtor::try_from_object(vm, exc_type)?;
    let exc = ctor.instantiate_value(exc_val, vm)?;
    if let Some(tb) = Option::<PyTracebackRef>::try_from_object(vm, exc_tb)? {
        exc.set_traceback(Some(tb));
    }
    Ok(exc)
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub base_exception_type: PyClassRef,
    pub system_exit: PyClassRef,
    pub keyboard_interrupt: PyClassRef,
    pub generator_exit: PyClassRef,
    pub exception_type: PyClassRef,
    pub stop_iteration: PyClassRef,
    pub stop_async_iteration: PyClassRef,
    pub arithmetic_error: PyClassRef,
    pub floating_point_error: PyClassRef,
    pub overflow_error: PyClassRef,
    pub zero_division_error: PyClassRef,
    pub assertion_error: PyClassRef,
    pub attribute_error: PyClassRef,
    pub buffer_error: PyClassRef,
    pub eof_error: PyClassRef,
    pub import_error: PyClassRef,
    pub module_not_found_error: PyClassRef,
    pub lookup_error: PyClassRef,
    pub index_error: PyClassRef,
    pub key_error: PyClassRef,
    pub memory_error: PyClassRef,
    pub name_error: PyClassRef,
    pub unbound_local_error: PyClassRef,
    pub os_error: PyClassRef,
    pub blocking_io_error: PyClassRef,
    pub child_process_error: PyClassRef,
    pub connection_error: PyClassRef,
    pub broken_pipe_error: PyClassRef,
    pub connection_aborted_error: PyClassRef,
    pub connection_refused_error: PyClassRef,
    pub connection_reset_error: PyClassRef,
    pub file_exists_error: PyClassRef,
    pub file_not_found_error: PyClassRef,
    pub interrupted_error: PyClassRef,
    pub is_a_directory_error: PyClassRef,
    pub not_a_directory_error: PyClassRef,
    pub permission_error: PyClassRef,
    pub process_lookup_error: PyClassRef,
    pub timeout_error: PyClassRef,
    pub reference_error: PyClassRef,
    pub runtime_error: PyClassRef,
    pub not_implemented_error: PyClassRef,
    pub recursion_error: PyClassRef,
    pub syntax_error: PyClassRef,
    pub target_scope_error: PyClassRef,
    pub indentation_error: PyClassRef,
    pub tab_error: PyClassRef,
    pub system_error: PyClassRef,
    pub type_error: PyClassRef,
    pub value_error: PyClassRef,
    pub unicode_error: PyClassRef,
    pub unicode_decode_error: PyClassRef,
    pub unicode_encode_error: PyClassRef,
    pub unicode_translate_error: PyClassRef,

    #[cfg(feature = "jit")]
    pub jit_error: PyClassRef,

    pub warning: PyClassRef,
    pub deprecation_warning: PyClassRef,
    pub pending_deprecation_warning: PyClassRef,
    pub runtime_warning: PyClassRef,
    pub syntax_warning: PyClassRef,
    pub user_warning: PyClassRef,
    pub future_warning: PyClassRef,
    pub import_warning: PyClassRef,
    pub unicode_warning: PyClassRef,
    pub bytes_warning: PyClassRef,
    pub resource_warning: PyClassRef,
}

impl ExceptionZoo {
    pub fn new(type_type: &PyClassRef, object_type: &PyClassRef) -> Self {
        let create_exception_type = |name: &str, base: &PyClassRef| {
            create_type_with_slots(name, type_type, base.clone(), PyBaseException::make_slots())
        };
        // Sorted By Hierarchy then alphabetized.
        let base_exception_type = create_exception_type(PyBaseExceptionRef::NAME, &object_type);
        let system_exit = create_exception_type("SystemExit", &base_exception_type);
        let keyboard_interrupt = create_exception_type("KeyboardInterrupt", &base_exception_type);
        let generator_exit = create_exception_type("GeneratorExit", &base_exception_type);

        let exception_type = create_exception_type("Exception", &base_exception_type);
        let stop_iteration = create_exception_type("StopIteration", &exception_type);
        let stop_async_iteration = create_exception_type("StopAsyncIteration", &exception_type);
        let arithmetic_error = create_exception_type("ArithmeticError", &exception_type);
        let floating_point_error = create_exception_type("FloatingPointError", &arithmetic_error);
        let overflow_error = create_exception_type("OverflowError", &arithmetic_error);
        let zero_division_error = create_exception_type("ZeroDivisionError", &arithmetic_error);
        let assertion_error = create_exception_type("AssertionError", &exception_type);
        let attribute_error = create_exception_type("AttributeError", &exception_type);
        let buffer_error = create_exception_type("BufferError", &exception_type);
        let eof_error = create_exception_type("EOFError", &exception_type);
        let import_error = create_exception_type("ImportError", &exception_type);
        let module_not_found_error = create_exception_type("ModuleNotFoundError", &import_error);
        let lookup_error = create_exception_type("LookupError", &exception_type);
        let index_error = create_exception_type("IndexError", &lookup_error);
        let key_error = create_exception_type("KeyError", &lookup_error);
        let memory_error = create_exception_type("MemoryError", &exception_type);
        let name_error = create_exception_type("NameError", &exception_type);
        let unbound_local_error = create_exception_type("UnboundLocalError", &name_error);

        // os errors
        let os_error = create_exception_type("OSError", &exception_type);
        let blocking_io_error = create_exception_type("BlockingIOError", &os_error);
        let child_process_error = create_exception_type("ChildProcessError", &os_error);
        let connection_error = create_exception_type("ConnectionError", &os_error);
        let connection_aborted_error =
            create_exception_type("ConnectionAbortedError", &connection_error);
        let connection_refused_error =
            create_exception_type("ConnectionRefusedError", &connection_error);
        let connection_reset_error =
            create_exception_type("ConnectionResetError", &connection_error);
        let file_exists_error = create_exception_type("FileExistsError", &os_error);
        let file_not_found_error = create_exception_type("FileNotFoundError", &os_error);
        let interrupted_error = create_exception_type("InterruptedError", &os_error);
        let is_a_directory_error = create_exception_type("IsADirectoryError", &os_error);
        let not_a_directory_error = create_exception_type("NotADirectoryError", &os_error);
        let broken_pipe_error = create_exception_type("BrokenPipeError", &connection_error);
        let permission_error = create_exception_type("PermissionError", &os_error);
        let process_lookup_error = create_exception_type("ProcessLookupError", &os_error);
        let timeout_error = create_exception_type("TimeoutError", &os_error);

        let reference_error = create_exception_type("ReferenceError", &exception_type);
        let runtime_error = create_exception_type("RuntimeError", &exception_type);
        let not_implemented_error = create_exception_type("NotImplementedError", &runtime_error);
        let recursion_error = create_exception_type("RecursionError", &runtime_error);
        let syntax_error = create_exception_type("SyntaxError", &exception_type);
        let indentation_error = create_exception_type("IndentationError", &syntax_error);
        let tab_error = create_exception_type("TabError", &indentation_error);
        let target_scope_error = create_exception_type("TargetScopeError", &syntax_error);
        let system_error = create_exception_type("SystemError", &exception_type);
        let type_error = create_exception_type("TypeError", &exception_type);
        let value_error = create_exception_type("ValueError", &exception_type);
        let unicode_error = create_exception_type("UnicodeError", &value_error);
        let unicode_decode_error = create_exception_type("UnicodeDecodeError", &unicode_error);
        let unicode_encode_error = create_exception_type("UnicodeEncodeError", &unicode_error);
        let unicode_translate_error =
            create_exception_type("UnicodeTranslateError", &unicode_error);

        #[cfg(feature = "jit")]
        let jit_error = create_exception_type("JitError", &exception_type);

        let warning = create_exception_type("Warning", &exception_type);
        let deprecation_warning = create_exception_type("DeprecationWarning", &warning);
        let pending_deprecation_warning =
            create_exception_type("PendingDeprecationWarning", &warning);
        let runtime_warning = create_exception_type("RuntimeWarning", &warning);
        let syntax_warning = create_exception_type("SyntaxWarning", &warning);
        let user_warning = create_exception_type("UserWarning", &warning);
        let future_warning = create_exception_type("FutureWarning", &warning);
        let import_warning = create_exception_type("ImportWarning", &warning);
        let unicode_warning = create_exception_type("UnicodeWarning", &warning);
        let bytes_warning = create_exception_type("BytesWarning", &warning);
        let resource_warning = create_exception_type("ResourceWarning", &warning);

        ExceptionZoo {
            base_exception_type,
            system_exit,
            keyboard_interrupt,
            generator_exit,
            exception_type,
            stop_iteration,
            stop_async_iteration,
            arithmetic_error,
            floating_point_error,
            overflow_error,
            zero_division_error,
            assertion_error,
            attribute_error,
            buffer_error,
            eof_error,
            import_error,
            module_not_found_error,
            lookup_error,
            index_error,
            key_error,
            memory_error,
            name_error,
            unbound_local_error,
            os_error,
            blocking_io_error,
            child_process_error,
            connection_error,
            broken_pipe_error,
            connection_aborted_error,
            connection_refused_error,
            connection_reset_error,
            file_exists_error,
            file_not_found_error,
            interrupted_error,
            is_a_directory_error,
            not_a_directory_error,
            permission_error,
            process_lookup_error,
            timeout_error,
            reference_error,
            runtime_error,
            not_implemented_error,
            recursion_error,
            syntax_error,
            target_scope_error,
            indentation_error,
            tab_error,
            system_error,
            type_error,
            value_error,
            unicode_error,
            unicode_decode_error,
            unicode_encode_error,
            unicode_translate_error,

            #[cfg(feature = "jit")]
            jit_error,

            warning,
            deprecation_warning,
            pending_deprecation_warning,
            runtime_warning,
            syntax_warning,
            user_warning,
            future_warning,
            import_warning,
            unicode_warning,
            bytes_warning,
            resource_warning,
        }
    }
}

fn import_error_init(exc_self: PyObjectRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<()> {
    vm.set_attr(
        &exc_self,
        "name",
        args.kwargs
            .get("name")
            .cloned()
            .unwrap_or_else(|| vm.get_none()),
    )?;
    vm.set_attr(
        &exc_self,
        "path",
        args.kwargs
            .get("path")
            .cloned()
            .unwrap_or_else(|| vm.get_none()),
    )?;
    Ok(())
}

fn none_getter(_obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.get_none()
}

fn make_arg_getter(idx: usize) -> impl Fn(PyBaseExceptionRef, &VirtualMachine) -> PyObjectRef {
    move |exc, vm| {
        exc.args
            .read()
            .borrow_value()
            .get(idx)
            .cloned()
            .unwrap_or_else(|| vm.get_none())
    }
}

fn key_error_str(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyStringRef {
    let args = exc.args();
    if args.borrow_value().len() == 1 {
        exception_args_as_string(vm, args, false)
            .into_iter()
            .exactly_one()
            .unwrap()
    } else {
        exc.str(vm)
    }
}

fn system_exit_code(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyObjectRef {
    match exc.args.read().borrow_value().first() {
        Some(code) => match_class!(match code {
            ref tup @ PyTuple => match tup.borrow_value() {
                [x] => x.clone(),
                _ => code.clone(),
            },
            other => other.clone(),
        }),
        None => vm.get_none(),
    }
}

pub fn init(ctx: &PyContext) {
    let excs = &ctx.exceptions;

    PyBaseException::extend_class(ctx, &excs.base_exception_type);

    extend_class!(ctx, &excs.syntax_error, {
        "msg" => ctx.new_readonly_getset("msg", make_arg_getter(0)),
        // TODO: members
        "filename" => ctx.none(),
        "lineno" => ctx.none(),
        "offset" => ctx.none(),
        "text" => ctx.none(),
    });

    extend_class!(ctx, &excs.system_exit, {
        "code" => ctx.new_readonly_getset("code", system_exit_code),
    });

    extend_class!(ctx, &excs.import_error, {
        "__init__" => ctx.new_method(import_error_init),
        "msg" => ctx.new_readonly_getset("msg", make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.stop_iteration, {
        "value" => ctx.new_readonly_getset("value", make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.key_error, {
        "__str__" => ctx.new_method(key_error_str),
    });

    extend_class!(ctx, &excs.unicode_decode_error, {
        "encoding" => ctx.new_readonly_getset("encoding", make_arg_getter(0)),
        "object" => ctx.new_readonly_getset("object", make_arg_getter(1)),
        "start" => ctx.new_readonly_getset("start", make_arg_getter(2)),
        "end" => ctx.new_readonly_getset("end", make_arg_getter(3)),
        "reason" => ctx.new_readonly_getset("reason", make_arg_getter(4)),
    });

    extend_class!(ctx, &excs.unicode_encode_error, {
        "encoding" => ctx.new_readonly_getset("encoding", make_arg_getter(0)),
        "object" => ctx.new_readonly_getset("object", make_arg_getter(1)),
        "start" => ctx.new_readonly_getset("start", make_arg_getter(2)),
        "end" => ctx.new_readonly_getset("end", make_arg_getter(3)),
        "reason" => ctx.new_readonly_getset("reason", make_arg_getter(4)),
    });

    extend_class!(ctx, &excs.unicode_translate_error, {
        "encoding" => ctx.new_readonly_getset("encoding", none_getter),
        "object" => ctx.new_readonly_getset("object", make_arg_getter(0)),
        "start" => ctx.new_readonly_getset("start", make_arg_getter(1)),
        "end" => ctx.new_readonly_getset("end", make_arg_getter(2)),
        "reason" => ctx.new_readonly_getset("reason", make_arg_getter(3)),
    });
}

pub struct SerializeException<'s> {
    vm: &'s VirtualMachine,
    exc: &'s PyBaseExceptionRef,
}

impl<'s> SerializeException<'s> {
    pub fn new(vm: &'s VirtualMachine, exc: &'s PyBaseExceptionRef) -> Self {
        SerializeException { vm, exc }
    }
}

impl serde::Serialize for SerializeException<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::*;

        let mut struc = s.serialize_struct("PyBaseException", 7)?;
        struc.serialize_field("exc_type", &self.exc.lease_class().name)?;
        let tbs = {
            struct Tracebacks(PyTracebackRef);
            impl serde::Serialize for Tracebacks {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    let mut s = s.serialize_seq(None)?;
                    for tb in self.0.iter() {
                        s.serialize_element(&*tb)?;
                    }
                    s.end()
                }
            }
            self.exc.traceback().map(Tracebacks)
        };
        struc.serialize_field("traceback", &tbs)?;
        struc.serialize_field(
            "cause",
            &self.exc.cause().as_ref().map(|e| Self::new(self.vm, e)),
        )?;
        struc.serialize_field(
            "context",
            &self.exc.context().as_ref().map(|e| Self::new(self.vm, e)),
        )?;
        struc.serialize_field("suppress_context", &self.exc.suppress_context.load())?;

        let args = {
            struct Args<'vm>(&'vm VirtualMachine, PyTupleRef);
            impl serde::Serialize for Args<'_> {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.collect_seq(
                        self.1
                            .borrow_value()
                            .iter()
                            .map(|arg| py_serde::PyObjectSerializer::new(self.0, arg)),
                    )
                }
            }
            Args(self.vm, self.exc.args())
        };
        struc.serialize_field("args", &args)?;

        let rendered = {
            let mut rendered = Vec::<u8>::new();
            write_exception(&mut rendered, self.vm, &self.exc).map_err(S::Error::custom)?;
            String::from_utf8(rendered).map_err(S::Error::custom)?
        };
        struc.serialize_field("rendered", &rendered)?;

        struc.end()
    }
}
