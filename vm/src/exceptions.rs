use crate::function::PyFuncArgs;
use crate::obj::objtraceback::PyTracebackRef;
use crate::obj::objtuple::{PyTuple, PyTupleRef};
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IdProtocol, PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::types::create_type;
use crate::vm::VirtualMachine;
use itertools::Itertools;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

fn exception_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let exc_self = args.args[0].clone();
    let exc_args = vm.ctx.new_tuple(args.args[1..].to_vec());
    vm.set_attr(&exc_self, "args", exc_args)?;

    // TODO: have an actual `traceback` object for __traceback__
    vm.set_attr(&exc_self, "__traceback__", vm.get_none())?;
    vm.set_attr(&exc_self, "__cause__", vm.get_none())?;
    vm.set_attr(&exc_self, "__context__", vm.get_none())?;
    vm.set_attr(&exc_self, "__suppress_context__", vm.new_bool(false))?;
    Ok(vm.get_none())
}

/// Print exception chain
pub fn print_exception(vm: &VirtualMachine, exc: &PyObjectRef) {
    let _ = write_exception(io::stdout(), vm, exc);
}

pub fn write_exception<W: Write>(
    mut output: W,
    vm: &VirtualMachine,
    exc: &PyObjectRef,
) -> io::Result<()> {
    let mut had_cause = false;
    if let Ok(cause) = vm.get_attribute(exc.clone(), "__cause__") {
        if !vm.get_none().is(&cause) {
            had_cause = true;
            print_exception(vm, &cause);
            writeln!(
                output,
                "\nThe above exception was the direct cause of the following exception:\n"
            )?;
        }
    }
    if !had_cause {
        if let Ok(context) = vm.get_attribute(exc.clone(), "__context__") {
            if !vm.get_none().is(&context) {
                print_exception(vm, &context);
                writeln!(
                    output,
                    "\nDuring handling of the above exception, another exception occurred:\n"
                )?;
            }
        }
    }
    print_exception_inner(output, vm, exc)
}

fn print_source_line<W: Write>(mut output: W, filename: &str, lineno: usize) -> io::Result<()> {
    // TODO: use io.open() method instead, when available, according to https://github.com/python/cpython/blob/master/Python/traceback.c#L393
    // TODO: support different encodings
    let file = match File::open(filename) {
        Ok(file) => file,
        Err(_) => {
            return Ok(());
        }
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
fn print_traceback_entry<W: Write>(mut output: W, tb_entry: &PyTracebackRef) -> io::Result<()> {
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
pub fn print_exception_inner<W: Write>(
    mut output: W,
    vm: &VirtualMachine,
    exc: &PyObjectRef,
) -> io::Result<()> {
    if let Ok(tb) = vm.get_attribute(exc.clone(), "__traceback__") {
        if objtype::isinstance(&tb, &vm.ctx.traceback_type()) {
            writeln!(output, "Traceback (most recent call last):")?;
            let mut tb: PyTracebackRef = tb.downcast().expect(" must be a traceback object");
            loop {
                print_traceback_entry(&mut output, &tb)?;
                tb = match &tb.next {
                    Some(tb) => tb.clone(),
                    None => break,
                };
            }
        }
    } else {
        writeln!(output, "No traceback set on exception")?;
    }

    let varargs = vm
        .get_attribute(exc.clone(), "args")
        .unwrap()
        .downcast::<PyTuple>()
        .expect("'args' must be a tuple");
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
) -> Vec<String> {
    match varargs.elements.len() {
        0 => vec![],
        1 => {
            let args0_repr = if str_single {
                vm.to_pystr(&varargs.elements[0])
                    .unwrap_or_else(|_| "<element str() failed>".to_string())
            } else {
                vm.to_repr(&varargs.elements[0])
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_else(|_| "<element repr() failed>".to_string())
            };
            vec![args0_repr]
        }
        _ => varargs
            .elements
            .iter()
            .map(|vararg| match vm.to_repr(vararg) {
                Ok(arg_repr) => arg_repr.as_str().to_string(),
                Err(_) => "<element repr() failed>".to_string(),
            })
            .collect(),
    }
}

fn exception_str(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(exc, Some(vm.ctx.exceptions.exception_type.clone()))]
    );
    let args = vm
        .get_attribute(exc.clone(), "args")
        .unwrap()
        .downcast::<PyTuple>()
        .expect("'args' must be a tuple");
    let args_str = exception_args_as_string(vm, args, false);
    let joined_str = match args_str.len() {
        0 => "".to_string(),
        1 => args_str.into_iter().next().unwrap(),
        _ => format!("({})", args_str.into_iter().format(", ")),
    };
    Ok(vm.new_str(joined_str))
}

fn exception_repr(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(exc, Some(vm.ctx.exceptions.exception_type.clone()))]
    );
    let args = vm
        .get_attribute(exc.clone(), "args")
        .unwrap()
        .downcast::<PyTuple>()
        .expect("'args' must be a tuple");
    let args_repr = exception_args_as_string(vm, args, false);

    let exc_name = exc.class().name.clone();
    let joined_str = match args_repr.len() {
        0 => format!("{}()", exc_name),
        1 => format!("{}({},)", exc_name, args_repr[0]),
        _ => format!("{}({})", exc_name, args_repr.join(", ")),
    };
    Ok(vm.new_str(joined_str))
}

fn exception_with_traceback(
    zelf: PyObjectRef,
    tb: Option<PyTracebackRef>,
    vm: &VirtualMachine,
) -> PyResult {
    vm.set_attr(
        &zelf,
        "__traceback__",
        tb.map_or(vm.get_none(), |tb| tb.into_object()),
    )?;
    Ok(zelf)
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
        let index_error = create_type("IndexError", &type_type, &exception_type);
        let key_error = create_type("KeyError", &type_type, &exception_type);
        let lookup_error = create_type("LookupError", &type_type, &exception_type);
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

fn import_error_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: call super().__init__(*args) instead
    exception_init(vm, args.clone())?;

    let exc_self = args.args[0].clone();
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
    vm.set_attr(
        &exc_self,
        "msg",
        args.args.get(1).cloned().unwrap_or_else(|| vm.get_none()),
    )?;
    Ok(vm.get_none())
}

pub fn init(context: &PyContext) {
    let base_exception_type = &context.exceptions.base_exception_type;
    extend_class!(context, base_exception_type, {
        "__init__" => context.new_rustfunc(exception_init),
        "with_traceback" => context.new_rustfunc(exception_with_traceback)
    });

    let exception_type = &context.exceptions.exception_type;
    extend_class!(context, exception_type, {
        "__str__" => context.new_rustfunc(exception_str),
        "__repr__" => context.new_rustfunc(exception_repr),
    });

    let import_error_type = &context.exceptions.import_error;
    extend_class!(context, import_error_type, {
        "__init__" => context.new_rustfunc(import_error_init)
    });
}
