use crate::function::PyFuncArgs;
use crate::obj::objnone::PyNone;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtraceback::PyTracebackRef;
use crate::obj::objtuple::{PyTuple, PyTupleRef};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::py_serde;
use crate::pyobject::{
    PyClassImpl, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PySetResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::slots::PyTpFlags;
use crate::types::create_type;
use crate::VirtualMachine;
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

#[pyimpl(flags(BASETYPE))]
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

    #[pyslot]
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
    fn set_args(&self, args: PyIterable, vm: &VirtualMachine) -> PySetResult {
        let args = args.iter(vm)?.collect::<PyResult<Vec<_>>>()?;
        self.args.replace(PyTuple::from(args).into_ref(vm));
        Ok(())
    }

    #[pyproperty(name = "__traceback__")]
    fn get_traceback(&self, _vm: &VirtualMachine) -> Option<PyTracebackRef> {
        self.traceback.borrow().clone()
    }

    #[pyproperty(name = "__traceback__", setter)]
    pub fn set_traceback(&self, traceback: Option<PyTracebackRef>) {
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
) -> io::Result<()> {
    if let Some(tb) = exc.traceback.borrow().clone() {
        writeln!(output, "Traceback (most recent call last):")?;
        for tb in tb.iter() {
            write_traceback_entry(output, &tb)?;
        }
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
    let varargs = varargs.as_slice();
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
                    tup @ PyTuple => tup.as_slice().to_vec(),
                    exc @ PyBaseException => exc.args().as_slice().to_vec(),
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
        let create_exception_type = |name: &str, base: &PyClassRef| {
            let typ = create_type(name, type_type, base);
            typ.slots.borrow_mut().flags |= PyTpFlags::BASETYPE;
            typ
        };
        // Sorted By Hierarchy then alphabetized.
        let base_exception_type = create_exception_type("BaseException", &object_type);
        let exception_type = create_exception_type("Exception", &base_exception_type);
        let arithmetic_error = create_exception_type("ArithmeticError", &exception_type);
        let assertion_error = create_exception_type("AssertionError", &exception_type);
        let attribute_error = create_exception_type("AttributeError", &exception_type);
        let import_error = create_exception_type("ImportError", &exception_type);
        let lookup_error = create_exception_type("LookupError", &exception_type);
        let index_error = create_exception_type("IndexError", &lookup_error);
        let key_error = create_exception_type("KeyError", &lookup_error);
        let name_error = create_exception_type("NameError", &exception_type);
        let runtime_error = create_exception_type("RuntimeError", &exception_type);
        let reference_error = create_exception_type("ReferenceError", &exception_type);
        let stop_iteration = create_exception_type("StopIteration", &exception_type);
        let stop_async_iteration = create_exception_type("StopAsyncIteration", &exception_type);
        let syntax_error = create_exception_type("SyntaxError", &exception_type);
        let system_error = create_exception_type("SystemError", &exception_type);
        let type_error = create_exception_type("TypeError", &exception_type);
        let value_error = create_exception_type("ValueError", &exception_type);
        let overflow_error = create_exception_type("OverflowError", &arithmetic_error);
        let zero_division_error = create_exception_type("ZeroDivisionError", &arithmetic_error);
        let module_not_found_error = create_exception_type("ModuleNotFoundError", &import_error);
        let not_implemented_error = create_exception_type("NotImplementedError", &runtime_error);
        let recursion_error = create_exception_type("RecursionError", &runtime_error);
        let eof_error = create_exception_type("EOFError", &exception_type);
        let indentation_error = create_exception_type("IndentationError", &syntax_error);
        let tab_error = create_exception_type("TabError", &indentation_error);
        let unicode_error = create_exception_type("UnicodeError", &value_error);
        let unicode_decode_error = create_exception_type("UnicodeDecodeError", &unicode_error);
        let unicode_encode_error = create_exception_type("UnicodeEncodeError", &unicode_error);
        let unicode_translate_error =
            create_exception_type("UnicodeTranslateError", &unicode_error);
        let memory_error = create_exception_type("MemoryError", &exception_type);

        // os errors
        let os_error = create_exception_type("OSError", &exception_type);

        let file_not_found_error = create_exception_type("FileNotFoundError", &os_error);
        let permission_error = create_exception_type("PermissionError", &os_error);
        let file_exists_error = create_exception_type("FileExistsError", &os_error);
        let blocking_io_error = create_exception_type("BlockingIOError", &os_error);
        let interrupted_error = create_exception_type("InterruptedError", &os_error);
        let connection_error = create_exception_type("ConnectionError", &os_error);
        let connection_reset_error =
            create_exception_type("ConnectionResetError", &connection_error);
        let connection_refused_error =
            create_exception_type("ConnectionRefusedError", &connection_error);
        let connection_aborted_error =
            create_exception_type("ConnectionAbortedError", &connection_error);
        let broken_pipe_error = create_exception_type("BrokenPipeError", &connection_error);

        let warning = create_exception_type("Warning", &exception_type);
        let bytes_warning = create_exception_type("BytesWarning", &warning);
        let unicode_warning = create_exception_type("UnicodeWarning", &warning);
        let deprecation_warning = create_exception_type("DeprecationWarning", &warning);
        let pending_deprecation_warning =
            create_exception_type("PendingDeprecationWarning", &warning);
        let future_warning = create_exception_type("FutureWarning", &warning);
        let import_warning = create_exception_type("ImportWarning", &warning);
        let syntax_warning = create_exception_type("SyntaxWarning", &warning);
        let resource_warning = create_exception_type("ResourceWarning", &warning);
        let runtime_warning = create_exception_type("RuntimeWarning", &warning);
        let user_warning = create_exception_type("UserWarning", &warning);

        let keyboard_interrupt = create_exception_type("KeyboardInterrupt", &base_exception_type);
        let generator_exit = create_exception_type("GeneratorExit", &base_exception_type);
        let system_exit = create_exception_type("SystemExit", &base_exception_type);

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
            .as_slice()
            .get(idx)
            .cloned()
            .unwrap_or_else(|| vm.get_none())
    }
}

fn key_error_str(exc: PyBaseExceptionRef, vm: &VirtualMachine) -> PyStringRef {
    let args = exc.args();
    if args.as_slice().len() == 1 {
        exception_args_as_string(vm, args, false)
            .into_iter()
            .exactly_one()
            .unwrap()
    } else {
        exc.str(vm)
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
        "__init__" => ctx.new_method(import_error_init),
        "msg" => ctx.new_property(make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.stop_iteration, {
        "value" => ctx.new_property(make_arg_getter(0)),
    });

    extend_class!(ctx, &excs.key_error, {
        "__str__" => ctx.new_method(key_error_str),
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
        struc.serialize_field("exc_type", &self.exc.class().name)?;
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
        struc.serialize_field("suppress_context", &self.exc.suppress_context.get())?;

        let args = {
            struct Args<'vm>(&'vm VirtualMachine, PyTupleRef);
            impl serde::Serialize for Args<'_> {
                fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                    s.collect_seq(
                        self.1
                            .as_slice()
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
