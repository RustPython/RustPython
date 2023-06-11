use self::types::{PyBaseException, PyBaseExceptionRef};
use crate::common::lock::PyRwLock;
use crate::object::{Traverse, TraverseFn};
use crate::{
    builtins::{
        traceback::PyTracebackRef, PyNone, PyStr, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef,
    },
    class::{PyClassImpl, StaticType},
    convert::{ToPyException, ToPyObject},
    function::{ArgIterable, FuncArgs, IntoFuncArgs},
    py_io::{self, Write},
    stdlib::sys,
    suggestion::offer_suggestions,
    types::{Callable, Constructor, Initializer, Representable},
    AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use itertools::Itertools;
use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader},
};

unsafe impl Traverse for PyBaseException {
    fn traverse(&self, tracer_fn: &mut TraverseFn) {
        self.traceback.traverse(tracer_fn);
        self.cause.traverse(tracer_fn);
        self.context.traverse(tracer_fn);
        self.args.traverse(tracer_fn);
    }
}

impl std::fmt::Debug for PyBaseException {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("PyBaseException")
    }
}

impl PyPayload for PyBaseException {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.exceptions.base_exception_type
    }
}

impl VirtualMachine {
    // Why `impl VirtualMachine?
    // These functions are natively free function in CPython - not methods of PyException

    /// Print exception chain by calling sys.excepthook
    pub fn print_exception(&self, exc: PyBaseExceptionRef) {
        let vm = self;
        let write_fallback = |exc, errstr| {
            if let Ok(stderr) = sys::get_stderr(vm) {
                let mut stderr = py_io::PyWriter(stderr, vm);
                // if this fails stderr might be closed -- ignore it
                let _ = writeln!(stderr, "{errstr}");
                let _ = self.write_exception(&mut stderr, exc);
            } else {
                eprintln!("{errstr}\nlost sys.stderr");
                let _ = self.write_exception(&mut py_io::IoWriter(io::stderr()), exc);
            }
        };
        if let Ok(excepthook) = vm.sys_module.get_attr("excepthook", vm) {
            let (exc_type, exc_val, exc_tb) = vm.split_exception(exc.clone());
            if let Err(eh_exc) = excepthook.call((exc_type, exc_val, exc_tb), vm) {
                write_fallback(&eh_exc, "Error in sys.excepthook:");
                write_fallback(&exc, "Original exception was:");
            }
        } else {
            write_fallback(&exc, "missing sys.excepthook");
        }
    }

    pub fn write_exception<W: Write>(
        &self,
        output: &mut W,
        exc: &PyBaseExceptionRef,
    ) -> Result<(), W::Error> {
        let seen = &mut HashSet::<usize>::new();
        self.write_exception_recursive(output, exc, seen)
    }

    fn write_exception_recursive<W: Write>(
        &self,
        output: &mut W,
        exc: &PyBaseExceptionRef,
        seen: &mut HashSet<usize>,
    ) -> Result<(), W::Error> {
        // This function should not be called directly,
        // use `wite_exception` as a public interface.
        // It is similar to `print_exception_recursive` from `CPython`.
        seen.insert(exc.get_id());

        #[allow(clippy::manual_map)]
        if let Some((cause_or_context, msg)) = if let Some(cause) = exc.cause() {
            // This can be a special case: `raise e from e`,
            // we just ignore it and treat like `raise e` without any extra steps.
            Some((
                cause,
                "\nThe above exception was the direct cause of the following exception:\n",
            ))
        } else if let Some(context) = exc.context() {
            // This can be a special case:
            //   e = ValueError('e')
            //   e.__context__ = e
            // In this case, we just ignore
            // `__context__` part from going into recursion.
            Some((
                context,
                "\nDuring handling of the above exception, another exception occurred:\n",
            ))
        } else {
            None
        } {
            if !seen.contains(&cause_or_context.get_id()) {
                self.write_exception_recursive(output, &cause_or_context, seen)?;
                writeln!(output, "{msg}")?;
            } else {
                seen.insert(cause_or_context.get_id());
            }
        }

        self.write_exception_inner(output, exc)
    }

    /// Print exception with traceback
    pub fn write_exception_inner<W: Write>(
        &self,
        output: &mut W,
        exc: &PyBaseExceptionRef,
    ) -> Result<(), W::Error> {
        let vm = self;
        if let Some(tb) = exc.traceback.read().clone() {
            writeln!(output, "Traceback (most recent call last):")?;
            for tb in tb.iter() {
                write_traceback_entry(output, &tb)?;
            }
        }

        let varargs = exc.args();
        let args_repr = vm.exception_args_as_string(varargs, true);

        let exc_class = exc.class();
        let exc_name = exc_class.name();
        match args_repr.len() {
            0 => write!(output, "{exc_name}"),
            1 => write!(output, "{}: {}", exc_name, args_repr[0]),
            _ => write!(
                output,
                "{}: ({})",
                exc_name,
                args_repr.into_iter().format(", ")
            ),
        }?;

        match offer_suggestions(exc, vm) {
            Some(suggestions) => writeln!(output, ". Did you mean: '{suggestions}'?"),
            None => writeln!(output),
        }
    }

    fn exception_args_as_string(&self, varargs: PyTupleRef, str_single: bool) -> Vec<PyStrRef> {
        let vm = self;
        match varargs.len() {
            0 => vec![],
            1 => {
                let args0_repr = if str_single {
                    varargs[0]
                        .str(vm)
                        .unwrap_or_else(|_| PyStr::from("<element str() failed>").into_ref(&vm.ctx))
                } else {
                    varargs[0].repr(vm).unwrap_or_else(|_| {
                        PyStr::from("<element repr() failed>").into_ref(&vm.ctx)
                    })
                };
                vec![args0_repr]
            }
            _ => varargs
                .iter()
                .map(|vararg| {
                    vararg.repr(vm).unwrap_or_else(|_| {
                        PyStr::from("<element repr() failed>").into_ref(&vm.ctx)
                    })
                })
                .collect(),
        }
    }

    pub fn split_exception(
        &self,
        exc: PyBaseExceptionRef,
    ) -> (PyObjectRef, PyObjectRef, PyObjectRef) {
        let tb = exc.traceback().to_pyobject(self);
        let class = exc.class().to_owned();
        (class.into(), exc.into(), tb)
    }

    /// Similar to PyErr_NormalizeException in CPython
    pub fn normalize_exception(
        &self,
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
    ) -> PyResult<PyBaseExceptionRef> {
        let ctor = ExceptionCtor::try_from_object(self, exc_type)?;
        let exc = ctor.instantiate_value(exc_val, self)?;
        if let Some(tb) = Option::<PyTracebackRef>::try_from_object(self, exc_tb)? {
            exc.set_traceback(Some(tb));
        }
        Ok(exc)
    }

    pub fn invoke_exception(
        &self,
        cls: PyTypeRef,
        args: Vec<PyObjectRef>,
    ) -> PyResult<PyBaseExceptionRef> {
        // TODO: fast-path built-in exceptions by directly instantiating them? Is that really worth it?
        let res = PyType::call(&cls, args.into_args(self), self)?;
        PyBaseExceptionRef::try_from_object(self, res)
    }
}

fn print_source_line<W: Write>(
    output: &mut W,
    filename: &str,
    lineno: usize,
) -> Result<(), W::Error> {
    // TODO: use io.open() method instead, when available, according to https://github.com/python/cpython/blob/main/Python/traceback.c#L393
    // TODO: support different encodings
    let file = match std::fs::File::open(filename) {
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
    let filename = tb_entry.frame.code.source_path.as_str();
    writeln!(
        output,
        r##"  File "{}", line {}, in {}"##,
        filename, tb_entry.lineno, tb_entry.frame.code.obj_name
    )?;
    print_source_line(output, filename, tb_entry.lineno.to_usize())?;

    Ok(())
}

#[derive(Clone)]
pub enum ExceptionCtor {
    Class(PyTypeRef),
    Instance(PyBaseExceptionRef),
}

impl TryFromObject for ExceptionCtor {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast::<PyType>()
            .and_then(|cls| {
                if cls.fast_issubclass(vm.ctx.exceptions.base_exception_type) {
                    Ok(Self::Class(cls))
                } else {
                    Err(cls.into())
                }
            })
            .or_else(|obj| obj.downcast::<PyBaseException>().map(Self::Instance))
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "exceptions must be classes or instances deriving from BaseException, not {}",
                    obj.class().name()
                ))
            })
    }
}

impl ExceptionCtor {
    pub fn instantiate(self, vm: &VirtualMachine) -> PyResult<PyBaseExceptionRef> {
        match self {
            Self::Class(cls) => vm.invoke_exception(cls, vec![]),
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
            (Self::Class(cls), Some(exc)) if exc.fast_isinstance(&cls) => Ok(exc),
            // otherwise; construct an exception of the type using the value as args
            (Self::Class(cls), _) => {
                let args = match_class!(match value {
                    PyNone => vec![],
                    tup @ PyTuple => tup.to_vec(),
                    exc @ PyBaseException => exc.args().to_vec(),
                    obj => vec![obj],
                });
                vm.invoke_exception(cls, args)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExceptionZoo {
    pub base_exception_type: &'static Py<PyType>,
    pub base_exception_group: &'static Py<PyType>,
    pub system_exit: &'static Py<PyType>,
    pub keyboard_interrupt: &'static Py<PyType>,
    pub generator_exit: &'static Py<PyType>,
    pub exception_type: &'static Py<PyType>,
    pub stop_iteration: &'static Py<PyType>,
    pub stop_async_iteration: &'static Py<PyType>,
    pub arithmetic_error: &'static Py<PyType>,
    pub floating_point_error: &'static Py<PyType>,
    pub overflow_error: &'static Py<PyType>,
    pub zero_division_error: &'static Py<PyType>,
    pub assertion_error: &'static Py<PyType>,
    pub attribute_error: &'static Py<PyType>,
    pub buffer_error: &'static Py<PyType>,
    pub eof_error: &'static Py<PyType>,
    pub import_error: &'static Py<PyType>,
    pub module_not_found_error: &'static Py<PyType>,
    pub lookup_error: &'static Py<PyType>,
    pub index_error: &'static Py<PyType>,
    pub key_error: &'static Py<PyType>,
    pub memory_error: &'static Py<PyType>,
    pub name_error: &'static Py<PyType>,
    pub unbound_local_error: &'static Py<PyType>,
    pub os_error: &'static Py<PyType>,
    pub blocking_io_error: &'static Py<PyType>,
    pub child_process_error: &'static Py<PyType>,
    pub connection_error: &'static Py<PyType>,
    pub broken_pipe_error: &'static Py<PyType>,
    pub connection_aborted_error: &'static Py<PyType>,
    pub connection_refused_error: &'static Py<PyType>,
    pub connection_reset_error: &'static Py<PyType>,
    pub file_exists_error: &'static Py<PyType>,
    pub file_not_found_error: &'static Py<PyType>,
    pub interrupted_error: &'static Py<PyType>,
    pub is_a_directory_error: &'static Py<PyType>,
    pub not_a_directory_error: &'static Py<PyType>,
    pub permission_error: &'static Py<PyType>,
    pub process_lookup_error: &'static Py<PyType>,
    pub timeout_error: &'static Py<PyType>,
    pub reference_error: &'static Py<PyType>,
    pub runtime_error: &'static Py<PyType>,
    pub not_implemented_error: &'static Py<PyType>,
    pub recursion_error: &'static Py<PyType>,
    pub syntax_error: &'static Py<PyType>,
    pub indentation_error: &'static Py<PyType>,
    pub tab_error: &'static Py<PyType>,
    pub system_error: &'static Py<PyType>,
    pub type_error: &'static Py<PyType>,
    pub value_error: &'static Py<PyType>,
    pub unicode_error: &'static Py<PyType>,
    pub unicode_decode_error: &'static Py<PyType>,
    pub unicode_encode_error: &'static Py<PyType>,
    pub unicode_translate_error: &'static Py<PyType>,

    #[cfg(feature = "jit")]
    pub jit_error: &'static Py<PyType>,

    pub warning: &'static Py<PyType>,
    pub deprecation_warning: &'static Py<PyType>,
    pub pending_deprecation_warning: &'static Py<PyType>,
    pub runtime_warning: &'static Py<PyType>,
    pub syntax_warning: &'static Py<PyType>,
    pub user_warning: &'static Py<PyType>,
    pub future_warning: &'static Py<PyType>,
    pub import_warning: &'static Py<PyType>,
    pub unicode_warning: &'static Py<PyType>,
    pub bytes_warning: &'static Py<PyType>,
    pub resource_warning: &'static Py<PyType>,
    pub encoding_warning: &'static Py<PyType>,
}

macro_rules! extend_exception {
    (
        $exc_struct:ident,
        $ctx:expr,
        $class:expr
    ) => {
        extend_exception!($exc_struct, $ctx, $class, {});
    };
    (
        $exc_struct:ident,
        $ctx:expr,
        $class:expr,
        { $($name:expr => $value:expr),* $(,)* }
    ) => {
        $exc_struct::extend_class($ctx, $class);
        extend_class!($ctx, $class, {
            $($name => $value,)*
        });
    };
}

impl PyBaseException {
    pub(crate) fn new(args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyBaseException {
        PyBaseException {
            traceback: PyRwLock::new(None),
            cause: PyRwLock::new(None),
            context: PyRwLock::new(None),
            suppress_context: AtomicCell::new(false),
            args: PyRwLock::new(PyTuple::new_ref(args, &vm.ctx)),
        }
    }

    pub fn get_arg(&self, idx: usize) -> Option<PyObjectRef> {
        self.args.read().get(idx).cloned()
    }
}

#[pyclass(
    with(Constructor, Initializer, Representable),
    flags(BASETYPE, HAS_DICT)
)]
impl PyBaseException {
    #[pygetset]
    pub fn args(&self) -> PyTupleRef {
        self.args.read().clone()
    }

    #[pygetset(setter)]
    fn set_args(&self, args: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
        let args = args.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
        *self.args.write() = PyTuple::new_ref(args, &vm.ctx);
        Ok(())
    }

    #[pygetset(magic)]
    pub fn traceback(&self) -> Option<PyTracebackRef> {
        self.traceback.read().clone()
    }

    #[pygetset(magic, setter)]
    pub fn set_traceback(&self, traceback: Option<PyTracebackRef>) {
        *self.traceback.write() = traceback;
    }

    #[pygetset(magic)]
    pub fn cause(&self) -> Option<PyRef<Self>> {
        self.cause.read().clone()
    }

    #[pygetset(magic, setter)]
    pub fn set_cause(&self, cause: Option<PyRef<Self>>) {
        let mut c = self.cause.write();
        self.set_suppress_context(true);
        *c = cause;
    }

    #[pygetset(magic)]
    pub fn context(&self) -> Option<PyRef<Self>> {
        self.context.read().clone()
    }

    #[pygetset(magic, setter)]
    pub fn set_context(&self, context: Option<PyRef<Self>>) {
        *self.context.write() = context;
    }

    #[pygetset(name = "__suppress_context__")]
    pub(super) fn get_suppress_context(&self) -> bool {
        self.suppress_context.load()
    }

    #[pygetset(name = "__suppress_context__", setter)]
    fn set_suppress_context(&self, suppress_context: bool) {
        self.suppress_context.store(suppress_context);
    }

    #[pymethod]
    fn with_traceback(zelf: PyRef<Self>, tb: Option<PyTracebackRef>) -> PyResult<PyRef<Self>> {
        *zelf.traceback.write() = tb;
        Ok(zelf)
    }

    #[pymethod(magic)]
    pub(super) fn str(&self, vm: &VirtualMachine) -> PyStrRef {
        let str_args = vm.exception_args_as_string(self.args(), true);
        match str_args.into_iter().exactly_one() {
            Err(i) if i.len() == 0 => vm.ctx.empty_str.to_owned(),
            Ok(s) => s,
            Err(i) => PyStr::from(format!("({})", i.format(", "))).into_ref(&vm.ctx),
        }
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
        if let Some(dict) = zelf.as_object().dict().filter(|x| !x.is_empty()) {
            vm.new_tuple((zelf.class().to_owned(), zelf.args(), dict))
        } else {
            vm.new_tuple((zelf.class().to_owned(), zelf.args()))
        }
    }
}

impl Constructor for PyBaseException {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        PyBaseException::new(args.args, vm)
            .into_ref_with_type(vm, cls)
            .map(Into::into)
    }
}

impl Initializer for PyBaseException {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        *zelf.args.write() = PyTuple::new_ref(args.args, &vm.ctx);
        Ok(())
    }
}

impl Representable for PyBaseException {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let repr_args = vm.exception_args_as_string(zelf.args(), false);
        let cls = zelf.class();
        Ok(format!("{}({})", cls.name(), repr_args.iter().format(", ")))
    }
}

impl ExceptionZoo {
    pub(crate) fn init() -> Self {
        use self::types::*;

        let base_exception_type = PyBaseException::init_builtin_type();

        // Sorted By Hierarchy then alphabetized.
        let base_exception_group = PyBaseExceptionGroup::init_builtin_type();
        let system_exit = PySystemExit::init_builtin_type();
        let keyboard_interrupt = PyKeyboardInterrupt::init_builtin_type();
        let generator_exit = PyGeneratorExit::init_builtin_type();

        let exception_type = PyException::init_builtin_type();
        let stop_iteration = PyStopIteration::init_builtin_type();
        let stop_async_iteration = PyStopAsyncIteration::init_builtin_type();
        let arithmetic_error = PyArithmeticError::init_builtin_type();
        let floating_point_error = PyFloatingPointError::init_builtin_type();
        let overflow_error = PyOverflowError::init_builtin_type();
        let zero_division_error = PyZeroDivisionError::init_builtin_type();

        let assertion_error = PyAssertionError::init_builtin_type();
        let attribute_error = PyAttributeError::init_builtin_type();
        let buffer_error = PyBufferError::init_builtin_type();
        let eof_error = PyEOFError::init_builtin_type();

        let import_error = PyImportError::init_builtin_type();
        let module_not_found_error = PyModuleNotFoundError::init_builtin_type();

        let lookup_error = PyLookupError::init_builtin_type();
        let index_error = PyIndexError::init_builtin_type();
        let key_error = PyKeyError::init_builtin_type();

        let memory_error = PyMemoryError::init_builtin_type();

        let name_error = PyNameError::init_builtin_type();
        let unbound_local_error = PyUnboundLocalError::init_builtin_type();

        // os errors
        let os_error = PyOSError::init_builtin_type();
        let blocking_io_error = PyBlockingIOError::init_builtin_type();
        let child_process_error = PyChildProcessError::init_builtin_type();

        let connection_error = PyConnectionError::init_builtin_type();
        let broken_pipe_error = PyBrokenPipeError::init_builtin_type();
        let connection_aborted_error = PyConnectionAbortedError::init_builtin_type();
        let connection_refused_error = PyConnectionRefusedError::init_builtin_type();
        let connection_reset_error = PyConnectionResetError::init_builtin_type();

        let file_exists_error = PyFileExistsError::init_builtin_type();
        let file_not_found_error = PyFileNotFoundError::init_builtin_type();
        let interrupted_error = PyInterruptedError::init_builtin_type();
        let is_a_directory_error = PyIsADirectoryError::init_builtin_type();
        let not_a_directory_error = PyNotADirectoryError::init_builtin_type();
        let permission_error = PyPermissionError::init_builtin_type();
        let process_lookup_error = PyProcessLookupError::init_builtin_type();
        let timeout_error = PyTimeoutError::init_builtin_type();

        let reference_error = PyReferenceError::init_builtin_type();

        let runtime_error = PyRuntimeError::init_builtin_type();
        let not_implemented_error = PyNotImplementedError::init_builtin_type();
        let recursion_error = PyRecursionError::init_builtin_type();

        let syntax_error = PySyntaxError::init_builtin_type();
        let indentation_error = PyIndentationError::init_builtin_type();
        let tab_error = PyTabError::init_builtin_type();

        let system_error = PySystemError::init_builtin_type();
        let type_error = PyTypeError::init_builtin_type();
        let value_error = PyValueError::init_builtin_type();
        let unicode_error = PyUnicodeError::init_builtin_type();
        let unicode_decode_error = PyUnicodeDecodeError::init_builtin_type();
        let unicode_encode_error = PyUnicodeEncodeError::init_builtin_type();
        let unicode_translate_error = PyUnicodeTranslateError::init_builtin_type();

        #[cfg(feature = "jit")]
        let jit_error = PyJitError::init_builtin_type();

        let warning = PyWarning::init_builtin_type();
        let deprecation_warning = PyDeprecationWarning::init_builtin_type();
        let pending_deprecation_warning = PyPendingDeprecationWarning::init_builtin_type();
        let runtime_warning = PyRuntimeWarning::init_builtin_type();
        let syntax_warning = PySyntaxWarning::init_builtin_type();
        let user_warning = PyUserWarning::init_builtin_type();
        let future_warning = PyFutureWarning::init_builtin_type();
        let import_warning = PyImportWarning::init_builtin_type();
        let unicode_warning = PyUnicodeWarning::init_builtin_type();
        let bytes_warning = PyBytesWarning::init_builtin_type();
        let resource_warning = PyResourceWarning::init_builtin_type();
        let encoding_warning = PyEncodingWarning::init_builtin_type();

        Self {
            base_exception_type,
            base_exception_group,
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
            encoding_warning,
        }
    }

    // TODO: remove it after fixing `errno` / `winerror` problem
    #[allow(clippy::redundant_clone)]
    pub fn extend(ctx: &Context) {
        use self::types::*;

        let excs = &ctx.exceptions;

        PyBaseException::extend_class(ctx, excs.base_exception_type);

        // Sorted By Hierarchy then alphabetized.
        extend_exception!(PyBaseExceptionGroup, ctx, excs.base_exception_group, {
            "message" => ctx.new_readonly_getset("message", excs.base_exception_group, make_arg_getter(0)),
            "exceptions" => ctx.new_readonly_getset("exceptions", excs.base_exception_group, make_arg_getter(1)),
        });
        extend_exception!(PySystemExit, ctx, excs.system_exit, {
            "code" => ctx.new_readonly_getset("code", excs.system_exit, system_exit_code),
        });
        extend_exception!(PyKeyboardInterrupt, ctx, excs.keyboard_interrupt);
        extend_exception!(PyGeneratorExit, ctx, excs.generator_exit);

        extend_exception!(PyException, ctx, excs.exception_type);

        extend_exception!(PyStopIteration, ctx, excs.stop_iteration, {
            "value" => ctx.none(),
        });
        extend_exception!(PyStopAsyncIteration, ctx, excs.stop_async_iteration);

        extend_exception!(PyArithmeticError, ctx, excs.arithmetic_error);
        extend_exception!(PyFloatingPointError, ctx, excs.floating_point_error);
        extend_exception!(PyOverflowError, ctx, excs.overflow_error);
        extend_exception!(PyZeroDivisionError, ctx, excs.zero_division_error);

        extend_exception!(PyAssertionError, ctx, excs.assertion_error);
        extend_exception!(PyAttributeError, ctx, excs.attribute_error, {
            "name" => ctx.none(),
            "obj" => ctx.none(),
        });
        extend_exception!(PyBufferError, ctx, excs.buffer_error);
        extend_exception!(PyEOFError, ctx, excs.eof_error);

        extend_exception!(PyImportError, ctx, excs.import_error, {
            "msg" => ctx.new_readonly_getset("msg", excs.import_error, make_arg_getter(0)),
            "name" => ctx.none(),
            "path" => ctx.none(),
        });
        extend_exception!(PyModuleNotFoundError, ctx, excs.module_not_found_error);

        extend_exception!(PyLookupError, ctx, excs.lookup_error);
        extend_exception!(PyIndexError, ctx, excs.index_error);

        extend_exception!(PyKeyError, ctx, excs.key_error);

        extend_exception!(PyMemoryError, ctx, excs.memory_error);
        extend_exception!(PyNameError, ctx, excs.name_error, {
            "name" => ctx.none(),
        });
        extend_exception!(PyUnboundLocalError, ctx, excs.unbound_local_error);

        // os errors:
        let errno_getter =
            ctx.new_readonly_getset("errno", excs.os_error, |exc: PyBaseExceptionRef| {
                let args = exc.args();
                args.get(0)
                    .filter(|_| args.len() > 1 && args.len() <= 5)
                    .cloned()
            });
        let strerror_getter =
            ctx.new_readonly_getset("strerror", excs.os_error, |exc: PyBaseExceptionRef| {
                let args = exc.args();
                args.get(1)
                    .filter(|_| args.len() >= 2 && args.len() <= 5)
                    .cloned()
            });
        extend_exception!(PyOSError, ctx, excs.os_error, {
            // POSIX exception code
            "errno" => errno_getter.clone(),
            // exception strerror
            "strerror" => strerror_getter.clone(),
            // exception filename
            "filename" => ctx.none(),
            // second exception filename
            "filename2" => ctx.none(),
        });
        // TODO: this isn't really accurate
        #[cfg(windows)]
        excs.os_error
            .set_str_attr("winerror", errno_getter.clone(), ctx);

        extend_exception!(PyBlockingIOError, ctx, excs.blocking_io_error);
        extend_exception!(PyChildProcessError, ctx, excs.child_process_error);

        extend_exception!(PyConnectionError, ctx, excs.connection_error);
        extend_exception!(PyBrokenPipeError, ctx, excs.broken_pipe_error);
        extend_exception!(PyConnectionAbortedError, ctx, excs.connection_aborted_error);
        extend_exception!(PyConnectionRefusedError, ctx, excs.connection_refused_error);
        extend_exception!(PyConnectionResetError, ctx, excs.connection_reset_error);

        extend_exception!(PyFileExistsError, ctx, excs.file_exists_error);
        extend_exception!(PyFileNotFoundError, ctx, excs.file_not_found_error);
        extend_exception!(PyInterruptedError, ctx, excs.interrupted_error);
        extend_exception!(PyIsADirectoryError, ctx, excs.is_a_directory_error);
        extend_exception!(PyNotADirectoryError, ctx, excs.not_a_directory_error);
        extend_exception!(PyPermissionError, ctx, excs.permission_error);
        extend_exception!(PyProcessLookupError, ctx, excs.process_lookup_error);
        extend_exception!(PyTimeoutError, ctx, excs.timeout_error);

        extend_exception!(PyReferenceError, ctx, excs.reference_error);
        extend_exception!(PyRuntimeError, ctx, excs.runtime_error);
        extend_exception!(PyNotImplementedError, ctx, excs.not_implemented_error);
        extend_exception!(PyRecursionError, ctx, excs.recursion_error);

        extend_exception!(PySyntaxError, ctx, excs.syntax_error, {
            "msg" => ctx.new_readonly_getset("msg", excs.syntax_error, make_arg_getter(0)),
            // TODO: members
            "filename" => ctx.none(),
            "lineno" => ctx.none(),
            "end_lineno" => ctx.none(),
            "offset" => ctx.none(),
            "end_offset" => ctx.none(),
            "text" => ctx.none(),
        });
        extend_exception!(PyIndentationError, ctx, excs.indentation_error);
        extend_exception!(PyTabError, ctx, excs.tab_error);

        extend_exception!(PySystemError, ctx, excs.system_error);
        extend_exception!(PyTypeError, ctx, excs.type_error);
        extend_exception!(PyValueError, ctx, excs.value_error);
        extend_exception!(PyUnicodeError, ctx, excs.unicode_error, {
            "encoding" => ctx.new_readonly_getset("encoding", excs.unicode_error, make_arg_getter(0)),
            "object" => ctx.new_readonly_getset("object", excs.unicode_error, make_arg_getter(1)),
            "start" => ctx.new_readonly_getset("start", excs.unicode_error, make_arg_getter(2)),
            "end" => ctx.new_readonly_getset("end", excs.unicode_error, make_arg_getter(3)),
            "reason" => ctx.new_readonly_getset("reason", excs.unicode_error, make_arg_getter(4)),
        });
        extend_exception!(PyUnicodeDecodeError, ctx, excs.unicode_decode_error);
        extend_exception!(PyUnicodeEncodeError, ctx, excs.unicode_encode_error);
        extend_exception!(PyUnicodeTranslateError, ctx, excs.unicode_translate_error, {
            "encoding" => ctx.new_readonly_getset("encoding", excs.unicode_translate_error, none_getter),
            "object" => ctx.new_readonly_getset("object", excs.unicode_translate_error, make_arg_getter(0)),
            "start" => ctx.new_readonly_getset("start", excs.unicode_translate_error, make_arg_getter(1)),
            "end" => ctx.new_readonly_getset("end", excs.unicode_translate_error, make_arg_getter(2)),
            "reason" => ctx.new_readonly_getset("reason", excs.unicode_translate_error, make_arg_getter(3)),
        });

        #[cfg(feature = "jit")]
        extend_exception!(PyJitError, ctx, excs.jit_error);

        extend_exception!(PyWarning, ctx, excs.warning);
        extend_exception!(PyDeprecationWarning, ctx, excs.deprecation_warning);
        extend_exception!(
            PyPendingDeprecationWarning,
            ctx,
            excs.pending_deprecation_warning
        );
        extend_exception!(PyRuntimeWarning, ctx, excs.runtime_warning);
        extend_exception!(PySyntaxWarning, ctx, excs.syntax_warning);
        extend_exception!(PyUserWarning, ctx, excs.user_warning);
        extend_exception!(PyFutureWarning, ctx, excs.future_warning);
        extend_exception!(PyImportWarning, ctx, excs.import_warning);
        extend_exception!(PyUnicodeWarning, ctx, excs.unicode_warning);
        extend_exception!(PyBytesWarning, ctx, excs.bytes_warning);
        extend_exception!(PyResourceWarning, ctx, excs.resource_warning);
        extend_exception!(PyEncodingWarning, ctx, excs.encoding_warning);
    }
}

fn none_getter(_obj: PyObjectRef, vm: &VirtualMachine) -> PyRef<PyNone> {
    vm.ctx.none.clone()
}

fn make_arg_getter(idx: usize) -> impl Fn(PyBaseExceptionRef) -> Option<PyObjectRef> {
    move |exc| exc.get_arg(idx)
}

fn system_exit_code(exc: PyBaseExceptionRef) -> Option<PyObjectRef> {
    exc.args.read().first().map(|code| {
        match_class!(match code {
            ref tup @ PyTuple => match tup.as_slice() {
                [x] => x.clone(),
                _ => code.clone(),
            },
            other => other.clone(),
        })
    })
}

#[cfg(feature = "serde")]
pub struct SerializeException<'vm, 's> {
    vm: &'vm VirtualMachine,
    exc: &'s PyBaseExceptionRef,
}

#[cfg(feature = "serde")]
impl<'vm, 's> SerializeException<'vm, 's> {
    pub fn new(vm: &'vm VirtualMachine, exc: &'s PyBaseExceptionRef) -> Self {
        SerializeException { vm, exc }
    }
}

#[cfg(feature = "serde")]
pub struct SerializeExceptionOwned<'vm> {
    vm: &'vm VirtualMachine,
    exc: PyBaseExceptionRef,
}

#[cfg(feature = "serde")]
impl serde::Serialize for SerializeExceptionOwned<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let Self { vm, exc } = self;
        SerializeException::new(vm, exc).serialize(s)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for SerializeException<'_, '_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::*;

        let mut struc = s.serialize_struct("PyBaseException", 7)?;
        struc.serialize_field("exc_type", &*self.exc.class().name())?;
        let tbs = {
            struct Tracebacks(PyTracebackRef);
            impl serde::Serialize for Tracebacks {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    let mut s = s.serialize_seq(None)?;
                    for tb in self.0.iter() {
                        s.serialize_element(&**tb)?;
                    }
                    s.end()
                }
            }
            self.exc.traceback().map(Tracebacks)
        };
        struc.serialize_field("traceback", &tbs)?;
        struc.serialize_field(
            "cause",
            &self
                .exc
                .cause()
                .map(|exc| SerializeExceptionOwned { vm: self.vm, exc }),
        )?;
        struc.serialize_field(
            "context",
            &self
                .exc
                .context()
                .map(|exc| SerializeExceptionOwned { vm: self.vm, exc }),
        )?;
        struc.serialize_field("suppress_context", &self.exc.get_suppress_context())?;

        let args = {
            struct Args<'vm>(&'vm VirtualMachine, PyTupleRef);
            impl serde::Serialize for Args<'_> {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.collect_seq(
                        self.1
                            .iter()
                            .map(|arg| crate::py_serde::PyObjectSerializer::new(self.0, arg)),
                    )
                }
            }
            Args(self.vm, self.exc.args())
        };
        struc.serialize_field("args", &args)?;

        let rendered = {
            let mut rendered = String::new();
            self.vm
                .write_exception(&mut rendered, self.exc)
                .map_err(S::Error::custom)?;
            rendered
        };
        struc.serialize_field("rendered", &rendered)?;

        struc.end()
    }
}

pub fn cstring_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_value_error("embedded null character".to_owned())
}

impl ToPyException for std::ffi::NulError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        cstring_error(vm)
    }
}

#[cfg(windows)]
impl<C: widestring::UChar> ToPyException for widestring::NulError<C> {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        cstring_error(vm)
    }
}

#[cfg(any(unix, windows, target_os = "wasi"))]
pub(crate) fn raw_os_error_to_exc_type(
    errno: i32,
    vm: &VirtualMachine,
) -> Option<&'static Py<PyType>> {
    use crate::stdlib::errno::errors;
    let excs = &vm.ctx.exceptions;
    match errno {
        errors::EWOULDBLOCK => Some(excs.blocking_io_error),
        errors::EALREADY => Some(excs.blocking_io_error),
        errors::EINPROGRESS => Some(excs.blocking_io_error),
        errors::EPIPE => Some(excs.broken_pipe_error),
        #[cfg(not(target_os = "wasi"))]
        errors::ESHUTDOWN => Some(excs.broken_pipe_error),
        errors::ECHILD => Some(excs.child_process_error),
        errors::ECONNABORTED => Some(excs.connection_aborted_error),
        errors::ECONNREFUSED => Some(excs.connection_refused_error),
        errors::ECONNRESET => Some(excs.connection_reset_error),
        errors::EEXIST => Some(excs.file_exists_error),
        errors::ENOENT => Some(excs.file_not_found_error),
        errors::EISDIR => Some(excs.is_a_directory_error),
        errors::ENOTDIR => Some(excs.not_a_directory_error),
        errors::EINTR => Some(excs.interrupted_error),
        errors::EACCES => Some(excs.permission_error),
        errors::EPERM => Some(excs.permission_error),
        errors::ESRCH => Some(excs.process_lookup_error),
        errors::ETIMEDOUT => Some(excs.timeout_error),
        _ => None,
    }
}

#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
pub(crate) fn raw_os_error_to_exc_type(
    _errno: i32,
    _vm: &VirtualMachine,
) -> Option<&'static Py<PyType>> {
    None
}

pub(super) mod types {
    use crate::common::lock::PyRwLock;
    #[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
    use crate::{
        builtins::{
            traceback::PyTracebackRef, tuple::IntoPyTuple, PyInt, PyStrRef, PyTupleRef, PyTypeRef,
        },
        convert::ToPyResult,
        function::FuncArgs,
        types::{Constructor, Initializer},
        AsObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;

    // This module is designed to be used as `use builtins::*;`.
    // Do not add any pub symbols not included in builtins module.
    // `PyBaseExceptionRef` is the only exception.

    pub type PyBaseExceptionRef = PyRef<PyBaseException>;

    // Sorted By Hierarchy then alphabetized.

    #[pyclass(module = false, name = "BaseException", traverse = "manual")]
    pub struct PyBaseException {
        pub(super) traceback: PyRwLock<Option<PyTracebackRef>>,
        pub(super) cause: PyRwLock<Option<PyRef<Self>>>,
        pub(super) context: PyRwLock<Option<PyRef<Self>>>,
        pub(super) suppress_context: AtomicCell<bool>,
        pub(super) args: PyRwLock<PyTupleRef>,
    }

    #[pyexception(name, base = "PyBaseException", ctx = "system_exit", impl)]
    #[derive(Debug)]
    pub struct PySystemExit {}

    #[pyexception(name, base = "PyBaseException", ctx = "base_exception_group", impl)]
    #[derive(Debug)]
    pub struct PyBaseExceptionGroup {}

    #[pyexception(name, base = "PyBaseException", ctx = "generator_exit", impl)]
    #[derive(Debug)]
    pub struct PyGeneratorExit {}

    #[pyexception(name, base = "PyBaseException", ctx = "keyboard_interrupt", impl)]
    #[derive(Debug)]
    pub struct PyKeyboardInterrupt {}

    #[pyexception(name, base = "PyBaseException", ctx = "exception_type", impl)]
    #[derive(Debug)]
    pub struct PyException {}

    #[pyexception(name, base = "PyException", ctx = "stop_iteration")]
    #[derive(Debug)]
    pub struct PyStopIteration {}

    #[pyexception]
    impl PyStopIteration {
        #[pyslot]
        #[pymethod(name = "__init__")]
        pub(crate) fn slot_init(
            zelf: PyObjectRef,
            args: ::rustpython_vm::function::FuncArgs,
            vm: &::rustpython_vm::VirtualMachine,
        ) -> ::rustpython_vm::PyResult<()> {
            zelf.set_attr("value", vm.unwrap_or_none(args.args.get(0).cloned()), vm)?;
            Ok(())
        }
    }

    #[pyexception(name, base = "PyException", ctx = "stop_async_iteration", impl)]
    #[derive(Debug)]
    pub struct PyStopAsyncIteration {}

    #[pyexception(name, base = "PyException", ctx = "arithmetic_error", impl)]
    #[derive(Debug)]
    pub struct PyArithmeticError {}

    #[pyexception(name, base = "PyArithmeticError", ctx = "floating_point_error", impl)]
    #[derive(Debug)]
    pub struct PyFloatingPointError {}

    #[pyexception(name, base = "PyArithmeticError", ctx = "overflow_error", impl)]
    #[derive(Debug)]
    pub struct PyOverflowError {}

    #[pyexception(name, base = "PyArithmeticError", ctx = "zero_division_error", impl)]
    #[derive(Debug)]
    pub struct PyZeroDivisionError {}

    #[pyexception(name, base = "PyException", ctx = "assertion_error", impl)]
    #[derive(Debug)]
    pub struct PyAssertionError {}

    #[pyexception(name, base = "PyException", ctx = "attribute_error", impl)]
    #[derive(Debug)]
    pub struct PyAttributeError {}

    #[pyexception(name, base = "PyException", ctx = "buffer_error", impl)]
    #[derive(Debug)]
    pub struct PyBufferError {}

    #[pyexception(name, base = "PyException", ctx = "eof_error", impl)]
    #[derive(Debug)]
    pub struct PyEOFError {}

    #[pyexception(name, base = "PyException", ctx = "import_error")]
    #[derive(Debug)]
    pub struct PyImportError {}

    #[pyexception]
    impl PyImportError {
        #[pyslot]
        #[pymethod(name = "__init__")]
        pub(crate) fn slot_init(
            zelf: PyObjectRef,
            args: ::rustpython_vm::function::FuncArgs,
            vm: &::rustpython_vm::VirtualMachine,
        ) -> ::rustpython_vm::PyResult<()> {
            zelf.set_attr(
                "name",
                vm.unwrap_or_none(args.kwargs.get("name").cloned()),
                vm,
            )?;
            zelf.set_attr(
                "path",
                vm.unwrap_or_none(args.kwargs.get("path").cloned()),
                vm,
            )?;
            Ok(())
        }
        #[pymethod(magic)]
        fn reduce(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyTupleRef {
            let obj = exc.as_object().to_owned();
            let mut result: Vec<PyObjectRef> = vec![
                obj.class().to_owned().into(),
                vm.new_tuple((exc.get_arg(0).unwrap(),)).into(),
            ];

            if let Some(dict) = obj.dict().filter(|x| !x.is_empty()) {
                result.push(dict.into());
            }

            result.into_pytuple(vm)
        }
    }

    #[pyexception(name, base = "PyImportError", ctx = "module_not_found_error", impl)]
    #[derive(Debug)]
    pub struct PyModuleNotFoundError {}

    #[pyexception(name, base = "PyException", ctx = "lookup_error", impl)]
    #[derive(Debug)]
    pub struct PyLookupError {}

    #[pyexception(name, base = "PyLookupError", ctx = "index_error", impl)]
    #[derive(Debug)]
    pub struct PyIndexError {}

    #[pyexception(name, base = "PyLookupError", ctx = "key_error")]
    #[derive(Debug)]
    pub struct PyKeyError {}

    #[pyexception]
    impl PyKeyError {
        #[pymethod(magic)]
        fn str(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyStrRef {
            let args = exc.args();
            if args.len() == 1 {
                vm.exception_args_as_string(args, false)
                    .into_iter()
                    .exactly_one()
                    .unwrap()
            } else {
                exc.str(vm)
            }
        }
    }

    #[pyexception(name, base = "PyException", ctx = "memory_error", impl)]
    #[derive(Debug)]
    pub struct PyMemoryError {}

    #[pyexception(name, base = "PyException", ctx = "name_error", impl)]
    #[derive(Debug)]
    pub struct PyNameError {}

    #[pyexception(name, base = "PyNameError", ctx = "unbound_local_error", impl)]
    #[derive(Debug)]
    pub struct PyUnboundLocalError {}

    #[pyexception(name, base = "PyException", ctx = "os_error")]
    #[derive(Debug)]
    pub struct PyOSError {}

    // OS Errors:
    #[pyexception]
    impl PyOSError {
        #[cfg(not(target_arch = "wasm32"))]
        fn optional_new(args: Vec<PyObjectRef>, vm: &VirtualMachine) -> Option<PyBaseExceptionRef> {
            let len = args.len();
            if (2..=5).contains(&len) {
                let errno = &args[0];
                errno
                    .payload_if_subclass::<PyInt>(vm)
                    .and_then(|errno| errno.try_to_primitive::<i32>(vm).ok())
                    .and_then(|errno| super::raw_os_error_to_exc_type(errno, vm))
                    .and_then(|typ| vm.invoke_exception(typ.to_owned(), args.to_vec()).ok())
            } else {
                None
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // We need this method, because of how `CPython` copies `init`
            // from `BaseException` in `SimpleExtendsException` macro.
            // See: `BaseException_new`
            if *cls.name() == *vm.ctx.exceptions.os_error.name() {
                match Self::optional_new(args.args.to_vec(), vm) {
                    Some(error) => error.to_pyresult(vm),
                    None => PyBaseException::slot_new(cls, args, vm),
                }
            } else {
                PyBaseException::slot_new(cls, args, vm)
            }
        }
        #[cfg(target_arch = "wasm32")]
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            PyBaseException::slot_new(cls, args, vm)
        }
        #[pyslot]
        #[pymethod(name = "__init__")]
        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let len = args.args.len();
            let mut new_args = args;
            if (3..=5).contains(&len) {
                zelf.set_attr("filename", new_args.args[2].clone(), vm)?;
                if len == 5 {
                    zelf.set_attr("filename2", new_args.args[4].clone(), vm)?;
                }

                new_args.args.truncate(2);
            }
            PyBaseException::slot_init(zelf, new_args, vm)
        }

        #[pymethod(magic)]
        fn str(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let args = exc.args();
            let obj = exc.as_object().to_owned();

            if args.len() == 2 {
                // SAFETY: len() == 2 is checked so get_arg 1 or 2 won't panic
                let errno = exc.get_arg(0).unwrap().str(vm)?;
                let msg = exc.get_arg(1).unwrap().str(vm)?;

                let s = match obj.get_attr("filename", vm) {
                    Ok(filename) => match obj.get_attr("filename2", vm) {
                        Ok(filename2) => format!(
                            "[Errno {}] {}: '{}' -> '{}'",
                            errno,
                            msg,
                            filename.str(vm)?,
                            filename2.str(vm)?
                        ),
                        Err(_) => format!("[Errno {}] {}: '{}'", errno, msg, filename.str(vm)?),
                    },
                    Err(_) => {
                        format!("[Errno {errno}] {msg}")
                    }
                };
                Ok(vm.ctx.new_str(s))
            } else {
                Ok(exc.str(vm))
            }
        }

        #[pymethod(magic)]
        fn reduce(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyTupleRef {
            let args = exc.args();
            let obj = exc.as_object().to_owned();
            let mut result: Vec<PyObjectRef> = vec![obj.class().to_owned().into()];

            if args.len() >= 2 && args.len() <= 5 {
                // SAFETY: len() == 2 is checked so get_arg 1 or 2 won't panic
                let errno = exc.get_arg(0).unwrap();
                let msg = exc.get_arg(1).unwrap();

                if let Ok(filename) = obj.get_attr("filename", vm) {
                    if !vm.is_none(&filename) {
                        let mut args_reduced: Vec<PyObjectRef> = vec![errno, msg, filename];

                        if let Ok(filename2) = obj.get_attr("filename2", vm) {
                            if !vm.is_none(&filename2) {
                                args_reduced.push(filename2);
                            }
                        }
                        result.push(args_reduced.into_pytuple(vm).into());
                    } else {
                        result.push(vm.new_tuple((errno, msg)).into());
                    }
                } else {
                    result.push(vm.new_tuple((errno, msg)).into());
                }
            } else {
                result.push(args.into());
            }

            if let Some(dict) = obj.dict().filter(|x| !x.is_empty()) {
                result.push(dict.into());
            }
            result.into_pytuple(vm)
        }
    }

    #[pyexception(name, base = "PyOSError", ctx = "blocking_io_error", impl)]
    #[derive(Debug)]
    pub struct PyBlockingIOError {}

    #[pyexception(name, base = "PyOSError", ctx = "child_process_error", impl)]
    #[derive(Debug)]
    pub struct PyChildProcessError {}

    #[pyexception(name, base = "PyOSError", ctx = "connection_error", impl)]
    #[derive(Debug)]
    pub struct PyConnectionError {}

    #[pyexception(name, base = "PyConnectionError", ctx = "broken_pipe_error", impl)]
    #[derive(Debug)]
    pub struct PyBrokenPipeError {}

    #[pyexception(
        name,
        base = "PyConnectionError",
        ctx = "connection_aborted_error",
        impl
    )]
    #[derive(Debug)]
    pub struct PyConnectionAbortedError {}

    #[pyexception(
        name,
        base = "PyConnectionError",
        ctx = "connection_refused_error",
        impl
    )]
    #[derive(Debug)]
    pub struct PyConnectionRefusedError {}

    #[pyexception(name, base = "PyConnectionError", ctx = "connection_reset_error", impl)]
    #[derive(Debug)]
    pub struct PyConnectionResetError {}

    #[pyexception(name, base = "PyOSError", ctx = "file_exists_error", impl)]
    #[derive(Debug)]
    pub struct PyFileExistsError {}

    #[pyexception(name, base = "PyOSError", ctx = "file_not_found_error", impl)]
    #[derive(Debug)]
    pub struct PyFileNotFoundError {}

    #[pyexception(name, base = "PyOSError", ctx = "interrupted_error", impl)]
    #[derive(Debug)]
    pub struct PyInterruptedError {}

    #[pyexception(name, base = "PyOSError", ctx = "is_a_directory_error", impl)]
    #[derive(Debug)]
    pub struct PyIsADirectoryError {}

    #[pyexception(name, base = "PyOSError", ctx = "not_a_directory_error", impl)]
    #[derive(Debug)]
    pub struct PyNotADirectoryError {}

    #[pyexception(name, base = "PyOSError", ctx = "permission_error", impl)]
    #[derive(Debug)]
    pub struct PyPermissionError {}

    #[pyexception(name, base = "PyOSError", ctx = "process_lookup_error", impl)]
    #[derive(Debug)]
    pub struct PyProcessLookupError {}

    #[pyexception(name, base = "PyOSError", ctx = "timeout_error", impl)]
    #[derive(Debug)]
    pub struct PyTimeoutError {}

    #[pyexception(name, base = "PyException", ctx = "reference_error", impl)]
    #[derive(Debug)]
    pub struct PyReferenceError {}

    #[pyexception(name, base = "PyException", ctx = "runtime_error", impl)]
    #[derive(Debug)]
    pub struct PyRuntimeError {}

    #[pyexception(name, base = "PyRuntimeError", ctx = "not_implemented_error", impl)]
    #[derive(Debug)]
    pub struct PyNotImplementedError {}

    #[pyexception(name, base = "PyRuntimeError", ctx = "recursion_error", impl)]
    #[derive(Debug)]
    pub struct PyRecursionError {}

    #[pyexception(name, base = "PyException", ctx = "syntax_error", impl)]
    #[derive(Debug)]
    pub struct PySyntaxError {}

    #[pyexception(name, base = "PySyntaxError", ctx = "indentation_error", impl)]
    #[derive(Debug)]
    pub struct PyIndentationError {}

    #[pyexception(name, base = "PyIndentationError", ctx = "tab_error", impl)]
    #[derive(Debug)]
    pub struct PyTabError {}

    #[pyexception(name, base = "PyException", ctx = "system_error", impl)]
    #[derive(Debug)]
    pub struct PySystemError {}

    #[pyexception(name, base = "PyException", ctx = "type_error", impl)]
    #[derive(Debug)]
    pub struct PyTypeError {}

    #[pyexception(name, base = "PyException", ctx = "value_error", impl)]
    #[derive(Debug)]
    pub struct PyValueError {}

    #[pyexception(name, base = "PyValueError", ctx = "unicode_error", impl)]
    #[derive(Debug)]
    pub struct PyUnicodeError {}

    #[pyexception(name, base = "PyUnicodeError", ctx = "unicode_decode_error", impl)]
    #[derive(Debug)]
    pub struct PyUnicodeDecodeError {}

    #[pyexception(name, base = "PyUnicodeError", ctx = "unicode_encode_error", impl)]
    #[derive(Debug)]
    pub struct PyUnicodeEncodeError {}

    #[pyexception(name, base = "PyUnicodeError", ctx = "unicode_translate_error", impl)]
    #[derive(Debug)]
    pub struct PyUnicodeTranslateError {}

    /// JIT error.
    #[cfg(feature = "jit")]
    #[pyexception(name, base = "PyException", ctx = "jit_error", impl)]
    #[derive(Debug)]
    pub struct PyJitError {}

    // Warnings
    #[pyexception(name, base = "PyException", ctx = "warning", impl)]
    #[derive(Debug)]
    pub struct PyWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "deprecation_warning", impl)]
    #[derive(Debug)]
    pub struct PyDeprecationWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "pending_deprecation_warning", impl)]
    #[derive(Debug)]
    pub struct PyPendingDeprecationWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "runtime_warning", impl)]
    #[derive(Debug)]
    pub struct PyRuntimeWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "syntax_warning", impl)]
    #[derive(Debug)]
    pub struct PySyntaxWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "user_warning", impl)]
    #[derive(Debug)]
    pub struct PyUserWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "future_warning", impl)]
    #[derive(Debug)]
    pub struct PyFutureWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "import_warning", impl)]
    #[derive(Debug)]
    pub struct PyImportWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "unicode_warning", impl)]
    #[derive(Debug)]
    pub struct PyUnicodeWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "bytes_warning", impl)]
    #[derive(Debug)]
    pub struct PyBytesWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "resource_warning", impl)]
    #[derive(Debug)]
    pub struct PyResourceWarning {}

    #[pyexception(name, base = "PyWarning", ctx = "encoding_warning", impl)]
    #[derive(Debug)]
    pub struct PyEncodingWarning {}
}
