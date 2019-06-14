use crate::function::PyFuncArgs;
use crate::obj::objsequence;
use crate::obj::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{create_type, IdProtocol, PyContext, PyObjectRef, PyResult, TypeProtocol};
use crate::vm::VirtualMachine;
use std::fs::File;
use std::io::{BufRead, BufReader};

fn exception_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    let zelf = args.args[0].clone();
    let msg = if args.args.len() > 1 {
        args.args[1].clone()
    } else {
        let empty_string = String::default();
        vm.new_str(empty_string)
    };
    let traceback = vm.ctx.new_list(Vec::new());
    vm.set_attr(&zelf, "msg", msg)?;
    vm.set_attr(&zelf, "__traceback__", traceback)?;
    Ok(vm.get_none())
}

/// Print exception chain
pub fn print_exception(vm: &VirtualMachine, exc: &PyObjectRef) {
    let mut had_cause = false;
    if let Ok(cause) = vm.get_attribute(exc.clone(), "__cause__") {
        if !vm.get_none().is(&cause) {
            had_cause = true;
            print_exception(vm, &cause);
            println!("\nThe above exception was the direct cause of the following exception:\n");
        }
    }
    if !had_cause {
        if let Ok(context) = vm.get_attribute(exc.clone(), "__context__") {
            if !vm.get_none().is(&context) {
                print_exception(vm, &context);
                println!("\nDuring handling of the above exception, another exception occurred:\n");
            }
        }
    }
    print_exception_inner(vm, exc)
}

fn print_source_line(filename: String, lineno: usize) {
    // TODO: use io.open() method instead, when available, according to https://github.com/python/cpython/blob/master/Python/traceback.c#L393
    // TODO: support different encodings
    let file = match File::open(filename) {
        Ok(file) => file,
        Err(_) => {
            return;
        }
    };
    let file = BufReader::new(file);

    for (i, line) in file.lines().enumerate() {
        if i + 1 == lineno {
            if let Ok(line) = line {
                // Indented with 4 spaces
                println!("    {}", line.trim_start());
            }
            return;
        }
    }
}

/// Print exception occurrence location from traceback element
fn print_traceback_entry(vm: &VirtualMachine, tb_entry: &PyObjectRef) {
    if objtype::isinstance(&tb_entry, &vm.ctx.tuple_type()) {
        let location_attrs = objsequence::get_elements_tuple(&tb_entry);
        let filename = if let Ok(x) = vm.to_str(&location_attrs[0]) {
            x.value.clone()
        } else {
            "<error>".to_string()
        };

        let lineno = if let Ok(x) = vm.to_str(&location_attrs[1]) {
            x.value.clone()
        } else {
            "<error>".to_string()
        };

        let obj_name = if let Ok(x) = vm.to_str(&location_attrs[2]) {
            x.value.clone()
        } else {
            "<error>".to_string()
        };

        println!(
            r##"  File "{}", line {}, in {}"##,
            filename, lineno, obj_name
        );
        print_source_line(filename, lineno.parse().unwrap());
    } else {
        println!("  File ??");
        return;
    }
}

/// Print exception with traceback
pub fn print_exception_inner(vm: &VirtualMachine, exc: &PyObjectRef) {
    if let Ok(tb) = vm.get_attribute(exc.clone(), "__traceback__") {
        println!("Traceback (most recent call last):");
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let mut tb_entries = objsequence::get_elements_list(&tb).to_vec();
            tb_entries.reverse();

            for exc_location in tb_entries.iter() {
                print_traceback_entry(vm, exc_location);
            }
        }
    } else {
        println!("No traceback set on exception");
    }

    match vm.to_str(exc) {
        Ok(txt) => println!("{}", txt.value),
        Err(err) => println!("Error during error {:?}", err),
    }
}

fn exception_str(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(exc, Some(vm.ctx.exceptions.exception_type.clone()))]
    );
    let msg = if let Ok(m) = vm.get_attribute(exc.clone(), "msg") {
        match vm.to_pystr(&m) {
            Ok(msg) => msg,
            _ => "<exception str() failed>".to_string(),
        }
    } else {
        panic!("Error message must be set");
    };
    let mut exc_repr = exc.class().name.clone();
    if !msg.is_empty() {
        &exc_repr.push_str(&format!(": {}", msg));
    }
    Ok(vm.new_str(exc_repr))
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub arithmetic_error: PyClassRef,
    pub assertion_error: PyClassRef,
    pub attribute_error: PyClassRef,
    pub base_exception_type: PyClassRef,
    pub exception_type: PyClassRef,
    pub file_not_found_error: PyClassRef,
    pub file_exists_error: PyClassRef,
    pub import_error: PyClassRef,
    pub index_error: PyClassRef,
    pub key_error: PyClassRef,
    pub module_not_found_error: PyClassRef,
    pub name_error: PyClassRef,
    pub not_implemented_error: PyClassRef,
    pub os_error: PyClassRef,
    pub overflow_error: PyClassRef,
    pub permission_error: PyClassRef,
    pub reference_error: PyClassRef,
    pub runtime_error: PyClassRef,
    pub stop_iteration: PyClassRef,
    pub syntax_error: PyClassRef,
    pub type_error: PyClassRef,
    pub value_error: PyClassRef,
    pub zero_division_error: PyClassRef,

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
        let name_error = create_type("NameError", &type_type, &exception_type);
        let os_error = create_type("OSError", &type_type, &exception_type);
        let runtime_error = create_type("RuntimeError", &type_type, &exception_type);
        let reference_error = create_type("ReferenceError", &type_type, &exception_type);
        let stop_iteration = create_type("StopIteration", &type_type, &exception_type);
        let syntax_error = create_type("SyntaxError", &type_type, &exception_type);
        let type_error = create_type("TypeError", &type_type, &exception_type);
        let value_error = create_type("ValueError", &type_type, &exception_type);
        let overflow_error = create_type("OverflowError", &type_type, &arithmetic_error);
        let zero_division_error = create_type("ZeroDivisionError", &type_type, &arithmetic_error);
        let module_not_found_error = create_type("ModuleNotFoundError", &type_type, &import_error);
        let not_implemented_error = create_type("NotImplementedError", &type_type, &runtime_error);
        let file_not_found_error = create_type("FileNotFoundError", &type_type, &os_error);
        let permission_error = create_type("PermissionError", &type_type, &os_error);
        let file_exists_error = create_type("FileExistsError", &type_type, &os_error);

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

        ExceptionZoo {
            arithmetic_error,
            assertion_error,
            attribute_error,
            base_exception_type,
            exception_type,
            file_not_found_error,
            file_exists_error,
            import_error,
            index_error,
            key_error,
            module_not_found_error,
            name_error,
            not_implemented_error,
            os_error,
            overflow_error,
            permission_error,
            runtime_error,
            stop_iteration,
            syntax_error,
            type_error,
            value_error,
            zero_division_error,
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
        }
    }
}

pub fn init(context: &PyContext) {
    let base_exception_type = &context.exceptions.base_exception_type;
    extend_class!(context, base_exception_type, {
        "__init__" => context.new_rustfunc(exception_init)
    });

    let exception_type = &context.exceptions.exception_type;
    extend_class!(context, exception_type, {
        "__str__" => context.new_rustfunc(exception_str)
    });
}
