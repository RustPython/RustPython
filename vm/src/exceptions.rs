use crate::function::PyFuncArgs;
use crate::obj::objnone::PyNone;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtraceback::PyTracebackRef;
use crate::obj::objtuple::{PyTuple, PyTupleRef};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::pyobject::{
    PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::types::create_type;
use crate::vm::VirtualMachine;
use itertools::Itertools;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

#[pyclass]
pub struct PyBaseException {
    traceback: RefCell<Option<PyTracebackRef>>,
    cause: RefCell<Option<PyBaseExceptionRef>>,
    context: RefCell<Option<PyBaseExceptionRef>>,
    suppress_context: Cell<bool>,
    args: RefCell<PyTupleRef>,
}

impl fmt::Debug for PyBaseException {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: implement more detailed, non-recursive Debug formatter
        f.write_str("PyBaseException")
    }
}

pub type PyBaseExceptionRef = PyRef<PyBaseException>;

impl PyValue for PyBaseException {
    const HAVE_DICT: bool = true;

    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.exceptions.base_exception_type.clone()
    }
}

#[pyimpl]
impl PyBaseException {
    pub(crate) fn new(args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyBaseException {
        PyBaseException {
            traceback: RefCell::new(None),
            cause: RefCell::new(None),
            context: RefCell::new(None),
            suppress_context: Cell::new(false),
            args: RefCell::new(PyTuple::from(args).into_ref(vm)),
        }
    }

    #[pyslot(new)]
    fn tp_new(cls: PyClassRef, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyBaseException::new(args.args, vm).into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__init__")]
    fn init(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        self.args.replace(PyTuple::from(args.args).into_ref(vm));
        Ok(())
    }

    #[pyproperty(name = "args")]
    fn get_args(&self, _vm: &VirtualMachine) -> PyTupleRef {
        self.args.borrow().clone()
    }

    #[pyproperty(setter)]
    fn set_args(&self, args: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        let args = args.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
        self.args.replace(PyTuple::from(args).into_ref(vm));
        Ok(())
    }

    #[pyproperty(name = "__traceback__")]
    fn get_traceback(&self, _vm: &VirtualMachine) -> Option<PyTracebackRef> {
        self.traceback.borrow().clone()
    }

    #[pyproperty(name = "__traceback__", setter)]
    fn setter_traceback(&self, traceback: Option<PyTracebackRef>, _vm: &VirtualMachine) {
        self.traceback.replace(traceback);
    }

    #[pyproperty(name = "__cause__")]
    fn get_cause(&self, _vm: &VirtualMachine) -> Option<PyBaseExceptionRef> {
        self.cause.borrow().clone()
    }

    #[pyproperty(name = "__cause__", setter)]
    fn setter_cause(&self, cause: Option<PyBaseExceptionRef>, _vm: &VirtualMachine) {
        self.cause.replace(cause);
    }

    #[pyproperty(name = "__context__")]
    fn get_context(&self, _vm: &VirtualMachine) -> Option<PyBaseExceptionRef> {
        self.context.borrow().clone()
    }

    #[pyproperty(name = "__context__", setter)]
    fn setter_context(&self, context: Option<PyBaseExceptionRef>, _vm: &VirtualMachine) {
        self.context.replace(context);
    }

    #[pyproperty(name = "__suppress_context__")]
    fn get_suppress_context(&self, _vm: &VirtualMachine) -> bool {
        self.suppress_context.get()
    }

    #[pyproperty(name = "__suppress_context__", setter)]
    fn set_suppress_context(&self, suppress_context: bool, _vm: &VirtualMachine) {
        self.suppress_context.set(suppress_context);
    }

    #[pymethod]
    fn with_traceback(
        zelf: PyRef<Self>,
        tb: Option<PyTracebackRef>,
        _vm: &VirtualMachine,
    ) -> PyResult {
        zelf.traceback.replace(tb);
        Ok(zelf.as_object().clone())
    }

    #[pymethod(name = "__str__")]
    fn str(&self, vm: &VirtualMachine) -> PyStringRef {
        let str_args = exception_args_as_string(vm, self.args(), false);
        match str_args.into_iter().exactly_one() {
            Err(i) if i.len() == 0 => PyString::from("").into_ref(vm),
            Ok(s) => s,
            Err(i) => PyString::from(format!("({})", i.format(", "))).into_ref(vm),
        }
    }

    #[pymethod(name = "__repr__")]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> String {
        let repr_args = exception_args_as_string(vm, zelf.args(), false);
        let cls = zelf.class();
        match repr_args.into_iter().exactly_one() {
            Ok(one) => format!("{}({},)", cls.name, one),
            Err(i) => format!("{}({})", cls.name, i.format(", ")),
        }
    }

    pub fn args(&self) -> PyTupleRef {
        self.args.borrow().clone()
    }

    pub fn traceback(&self) -> Option<PyTracebackRef> {
        self.traceback.borrow().clone()
    }
    pub fn set_traceback(&self, tb: Option<PyTracebackRef>) {
        self.traceback.replace(tb);
    }

    pub fn cause(&self) -> Option<PyBaseExceptionRef> {
        self.cause.borrow().clone()
    }
    pub fn set_cause(&self, cause: Option<PyBaseExceptionRef>) {
        self.cause.replace(cause);
    }

    pub fn context(&self) -> Option<PyBaseExceptionRef> {
        self.context.borrow().clone()
    }
    pub fn set_context(&self, context: Option<PyBaseExceptionRef>) {
        self.context.replace(context);
    }
}

/// Print exception chain
pub fn print_exception(vm: &VirtualMachine, exc: &PyBaseExceptionRef) {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let _ = write_exception(&mut stdout, vm, exc);
}

pub fn write_exception<W: Write>(
    output: &mut W,
    vm: &VirtualMachine,
    exc: &PyBaseExceptionRef,
) -> io::Result<()> {
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

fn print_source_line<W: Write>(output: &mut W, filename: &str, lineno: usize) -> io::Result<()> {
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
fn write_traceback_entry<W: Write>(output: &mut W, tb_entry: &PyTracebackRef) -> io::Result<()> {
    let filename = tb_entry.frame.code.source_path.to_string();
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
) -> io::Result<()> {
    if let Some(tb) = exc.traceback.borrow().clone() {
        writeln!(output, "Traceback (most recent call last):")?;
        let mut tb = Some(&tb);
        while let Some(traceback) = tb {
            write_traceback_entry(output, traceback)?;
            tb = traceback.next.as_ref();
        }
    } else {
        writeln!(output, "No traceback set on exception")?;
    }

    let varargs = exc.args();
    let args_repr = exception_args_as_string(vm, varargs, true);

    let exc_name = exc.class().name.clone();
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
    match varargs.elements.len() {
        0 => vec![],
        1 => {
            let args0_repr = if str_single {
                vm.to_str(&varargs.elements[0])
                    .unwrap_or_else(|_| PyString::from("<element str() failed>").into_ref(vm))
            } else {
                vm.to_repr(&varargs.elements[0])
                    .unwrap_or_else(|_| PyString::from("<element repr() failed>").into_ref(vm))
            };
            vec![args0_repr]
        }
        _ => varargs
            .elements
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
                    obj.class().name
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
                    .new_type_error("instance exception may not have a separate value".to_string()))
            }
            // if the "type" is an instance and the value isn't, use the "type"
            (Self::Instance(exc), None) => Ok(exc),
            // if the value is an instance of the type, use the instance value
            (Self::Class(cls), Some(exc)) if objtype::isinstance(&exc, &cls) => Ok(exc),
            // otherwise; construct an exception of the type using the value as args
            (Self::Class(cls), _) => {
                let args = match_class!(match value {
                    PyNone => vec![],
                    tup @ PyTuple => tup.elements.clone(),
                    exc @ PyBaseException => exc.args().elements.clone(),
                    obj => vec![obj],
                });
                invoke(cls, args, vm)
            }
        }
    }
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
    pub arithmetic_error: PyClassRef,
    pub assertion_error: PyClassRef,
    pub attribute_error: PyClassRef,
    pub base_exception_type: PyClassRef,
    pub exception_type: PyClassRef,
    pub import_error: PyClassRef,
    pub index_error: PyClassRef,
    pub key_error: PyClassRef,
    pub lookup_error: PyClassRef,
    pub module_not_found_error: PyClassRef,
    pub name_error: PyClassRef,
    pub not_implemented_error: PyClassRef,
    pub recursion_error: PyClassRef,
    pub overflow_error: PyClassRef,
    pub reference_error: PyClassRef,
    pub runtime_error: PyClassRef,
    pub stop_iteration: PyClassRef,
    pub stop_async_iteration: PyClassRef,
    pub syntax_error: PyClassRef,
    pub indentation_error: PyClassRef,
    pub tab_error: PyClassRef,
    pub system_error: PyClassRef,
    pub type_error: PyClassRef,
    pub value_error: PyClassRef,
    pub unicode_error: PyClassRef,
    pub unicode_decode_error: PyClassRef,
    pub unicode_encode_error: PyClassRef,
    pub unicode_translate_error: PyClassRef,
    pub zero_division_error: PyClassRef,
    pub eof_error: PyClassRef,
    pub memory_error: PyClassRef,

    pub os_error: PyClassRef,
    pub file_not_found_error: PyClassRef,
    pub permission_error: PyClassRef,
    pub file_exists_error: PyClassRef,
    pub blocking_io_error: PyClassRef,
    pub interrupted_error: PyClassRef,
    pub connection_error: PyClassRef,
    pub connection_reset_error: PyClassRef,
    pub connection_refused_error: PyClassRef,
    pub connection_aborted_error: PyClassRef,
    pub broken_pipe_error: PyClassRef,

    pub warning: PyClassRef,
    pub bytes_warning: PyClassRef,
    pub unicode_warning: PyClassRef,
    pub deprecation_warning: PyClassRef,
    pub pending_deprecation_warning: PyClassRef,
    pub future_warning: PyClassRef,
    pub import_warning: PyClassRef,
    pub syntax_warning: PyClassRef,
    pub resource_warning: PyClassRef,
    pub runtime_warning: PyClassRef,
    pub user_warning: PyClassRef,

    pub keyboard_interrupt: PyClassRef,
    pub generator_exit: PyClassRef,
    pub system_exit: PyClassRef,
}

impl ExceptionZoo {
    pub fn new(type_type: &PyClassRef, object_type: &PyClassRef) -> Self {
        // Sorted By Hierarchy then alphabetized.
        let base_exception_type = create_type("BaseException", &type_type, &object_type);
        let exception_type = create_type("Exception", &type_type, &base_exception_type);
        let arithmetic_error = create_type("ArithmeticError", &type_type, &exception_type);
        let assertion_error = create_type("AssertionError", &type_type, &exception_type);
        let attribute_error = create_type("AttributeError", &type_type, &exception_type);
        let import_error = create_type("ImportError", &type_type, &exception_type);
        let lookup_error = create_type("LookupError", &type_type, &exception_type);
        let index_error = create_type("IndexError", &type_type, &lookup_error);
        let key_error = create_type("KeyError", &type_type, &lookup_error);
        let name_error = create_type("NameError", &type_type, &exception_type);
        let runtime_error = create_type("RuntimeError", &type_type, &exception_type);
        let reference_error = create_type("ReferenceError", &type_type, &exception_type);
        let stop_iteration = create_type("StopIteration", &type_type, &exception_type);
        let stop_async_iteration = create_type("StopAsyncIteration", &type_type, &exception_type);
        let syntax_error = create_type("SyntaxError", &type_type, &exception_type);
        let system_error = create_type("SystemError", &type_type, &exception_type);
        let type_error = create_type("TypeError", &type_type, &exception_type);
        let value_error = create_type("ValueError", &type_type, &exception_type);
        let overflow_error = create_type("OverflowError", &type_type, &arithmetic_error);
        let zero_division_error = create_type("ZeroDivisionError", &type_type, &arithmetic_error);
        let module_not_found_error = create_type("ModuleNotFoundError", &type_type, &import_error);
        let not_implemented_error = create_type("NotImplementedError", &type_type, &runtime_error);
        let recursion_error = create_type("RecursionError", &type_type, &runtime_error);
        let eof_error = create_type("EOFError", &type_type, &exception_type);
        let indentation_error = create_type("IndentationError", &type_type, &syntax_error);
        let tab_error = create_type("TabError", &type_type, &indentation_error);
        let unicode_error = create_type("UnicodeError", &type_type, &value_error);
        let unicode_decode_error = create_type("UnicodeDecodeError", &type_type, &unicode_error);
        let unicode_encode_error = create_type("UnicodeEncodeError", &type_type, &unicode_error);
        let unicode_translate_error =
            create_type("UnicodeTranslateError", &type_type, &unicode_error);
        let memory_error = create_type("MemoryError", &type_type, &exception_type);

        // os errors
        let os_error = create_type("OSError", &type_type, &exception_type);

        let file_not_found_error = create_type("FileNotFoundError", &type_type, &os_error);
        let permission_error = create_type("PermissionError", &type_type, &os_error);
        let file_exists_error = create_type("FileExistsError", &type_type, &os_error);
        let blocking_io_error = create_type("BlockingIOError", &type_type, &os_error);
        let interrupted_error = create_type("InterruptedError", &type_type, &os_error);
        let connection_error = create_type("ConnectionError", &type_type, &os_error);
        let connection_reset_error =
            create_type("ConnectionResetError", &type_type, &connection_error);
        let connection_refused_error =
            create_type("ConnectionRefusedError", &type_type, &connection_error);
        let connection_aborted_error =
            create_type("ConnectionAbortedError", &type_type, &connection_error);
        let broken_pipe_error = create_type("BrokenPipeError", &type_type, &connection_error);

        let warning = create_type("Warning", &type_type, &exception_type);
        let bytes_warning = create_type("BytesWarning", &type_type, &warning);
        let unicode_warning = create_type("UnicodeWarning", &type_type, &warning);
        let deprecation_warning = create_type("DeprecationWarning", &type_type, &warning);
        let pending_deprecation_warning =
            create_type("PendingDeprecationWarning", &type_type, &warning);
        let future_warning = create_type("FutureWarning", &type_type, &warning);
        let import_warning = create_type("ImportWarning", &type_type, &warning);
        let syntax_warning = create_type("SyntaxWarning", &type_type, &warning);
        let resource_warning = create_type("ResourceWarning", &type_type, &warning);
        let runtime_warning = create_type("RuntimeWarning", &type_type, &warning);
        let user_warning = create_type("UserWarning", &type_type, &warning);

        let keyboard_interrupt = create_type("KeyboardInterrupt", &type_type, &base_exception_type);
        let generator_exit = create_type("GeneratorExit", &type_type, &base_exception_type);
        let system_exit = create_type("SystemExit", &type_type, &base_exception_type);

        ExceptionZoo {
            arithmetic_error,
            assertion_error,
            attribute_error,
            base_exception_type,
            exception_type,
            import_error,
            index_error,
            key_error,
            lookup_error,
            module_not_found_error,
            name_error,
            not_implemented_error,
            recursion_error,
            overflow_error,
            runtime_error,
            stop_iteration,
            stop_async_iteration,
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
            zero_division_error,
            eof_error,
            memory_error,
            os_error,
            file_not_found_error,
            permission_error,
            file_exists_error,
            blocking_io_error,
            interrupted_error,
            connection_error,
            connection_reset_error,
            connection_refused_error,
            connection_aborted_error,
            broken_pipe_error,
            warning,
            bytes_warning,
            unicode_warning,
            deprecation_warning,
            pending_deprecation_warning,
            future_warning,
            import_warning,
            syntax_warning,
            resource_warning,
            runtime_warning,
            reference_error,
            user_warning,
            keyboard_interrupt,
            generator_exit,
            system_exit,
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
            .borrow()
            .elements
            .get(idx)
            .cloned()
            .unwrap_or_else(|| vm.get_none())
    }
}

pub fn init(ctx: &PyContext) {
    let excs = &ctx.exceptions;

    PyBaseException::extend_class(ctx, &excs.base_exception_type);

    extend_class!(ctx, &excs.syntax_error, {
        "msg" => ctx.new_property(make_arg_getter(0)),
        "filename" => ctx.new_property(make_arg_getter(1)),
        "lineno" => ctx.new_property(make_arg_getter(2)),
        "offset" => ctx.new_property(make_arg_getter(3)),
        "text" => ctx.new_property(make_arg_getter(4)),
    });

    extend_class!(ctx, &excs.import_error, {
        "__init__" => ctx.new_rustfunc(import_error_init),
        "msg" => ctx.new_property(make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.stop_iteration, {
        "value" => ctx.new_property(make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.unicode_decode_error, {
        "encoding" => ctx.new_property(make_arg_getter(0)),
        "object" => ctx.new_property(make_arg_getter(1)),
        "start" => ctx.new_property(make_arg_getter(2)),
        "end" => ctx.new_property(make_arg_getter(3)),
        "reason" => ctx.new_property(make_arg_getter(4)),
    });

    extend_class!(ctx, &excs.unicode_encode_error, {
        "encoding" => ctx.new_property(make_arg_getter(0)),
        "object" => ctx.new_property(make_arg_getter(1)),
        "start" => ctx.new_property(make_arg_getter(2)),
        "end" => ctx.new_property(make_arg_getter(3)),
        "reason" => ctx.new_property(make_arg_getter(4)),
    });

    extend_class!(ctx, &excs.unicode_translate_error, {
        "encoding" => ctx.new_property(none_getter),
        "object" => ctx.new_property(make_arg_getter(0)),
        "start" => ctx.new_property(make_arg_getter(1)),
        "end" => ctx.new_property(make_arg_getter(2)),
        "reason" => ctx.new_property(make_arg_getter(3)),
    });
}
