use super::obj::objsequence;
use super::obj::objstr;
use super::obj::objtype;
use super::pyobject::{
    create_type, AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::vm::VirtualMachine;

fn exception_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let zelf = args.args[0].clone();
    let msg = if args.args.len() > 1 {
        args.args[1].clone()
    } else {
        vm.new_str("No msg".to_string())
    };
    let traceback = vm.ctx.new_list(Vec::new());
    vm.ctx.set_attr(&zelf, "msg", msg);
    vm.ctx.set_attr(&zelf, "__traceback__", traceback);
    Ok(vm.get_none())
}

// Print exception including traceback:
pub fn print_exception(vm: &mut VirtualMachine, exc: &PyObjectRef) {
    if let Some(tb) = exc.get_attr("__traceback__") {
        println!("Traceback (most recent call last):");
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let mut elements = objsequence::get_elements(&tb).to_vec();
            elements.reverse();
            for element in elements.iter() {
                if objtype::isinstance(&element, &vm.ctx.tuple_type()) {
                    let element = objsequence::get_elements(&element);
                    let filename = if let Ok(x) = vm.to_str(&element[0]) {
                        objstr::get_value(&x)
                    } else {
                        "<error>".to_string()
                    };

                    let lineno = if let Ok(x) = vm.to_str(&element[1]) {
                        objstr::get_value(&x)
                    } else {
                        "<error>".to_string()
                    };

                    let obj_name = if let Ok(x) = vm.to_str(&element[2]) {
                        objstr::get_value(&x)
                    } else {
                        "<error>".to_string()
                    };

                    println!("  File {}, line {}, in {}", filename, lineno, obj_name);
                } else {
                    println!("  File ??");
                }
            }
        }
    } else {
        println!("No traceback set on exception");
    }

    match vm.to_str(exc) {
        Ok(txt) => println!("{}", objstr::get_value(&txt)),
        Err(err) => println!("Error during error {:?}", err),
    }
}

fn exception_str(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(exc, Some(vm.ctx.exceptions.exception_type.clone()))]
    );
    let type_name = objtype::get_type_name(&exc.typ());
    let msg = if let Some(m) = exc.get_attr("msg") {
        match vm.to_pystr(&m) {
            Ok(msg) => msg,
            _ => "<exception str() failed>".to_string(),
        }
    } else {
        panic!("Error message must be set");
    };
    let s = format!("{}: {}", type_name, msg);
    Ok(vm.new_str(s))
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub arithmetic_error: PyObjectRef,
    pub assertion_error: PyObjectRef,
    pub attribute_error: PyObjectRef,
    pub base_exception_type: PyObjectRef,
    pub exception_type: PyObjectRef,
    pub file_not_found_error: PyObjectRef,
    pub import_error: PyObjectRef,
    pub index_error: PyObjectRef,
    pub key_error: PyObjectRef,
    pub module_not_found_error: PyObjectRef,
    pub name_error: PyObjectRef,
    pub not_implemented_error: PyObjectRef,
    pub os_error: PyObjectRef,
    pub overflow_error: PyObjectRef,
    pub permission_error: PyObjectRef,
    pub runtime_error: PyObjectRef,
    pub stop_iteration: PyObjectRef,
    pub syntax_error: PyObjectRef,
    pub type_error: PyObjectRef,
    pub value_error: PyObjectRef,
    pub zero_division_error: PyObjectRef,
}

impl ExceptionZoo {
    pub fn new(
        type_type: &PyObjectRef,
        object_type: &PyObjectRef,
        dict_type: &PyObjectRef,
    ) -> Self {
        // Sorted By Hierarchy then alphabetized.
        let base_exception_type =
            create_type("BaseException", &type_type, &object_type, &dict_type);

        let exception_type = create_type("Exception", &type_type, &base_exception_type, &dict_type);

        let arithmetic_error =
            create_type("ArithmeticError", &type_type, &exception_type, &dict_type);
        let assertion_error =
            create_type("AssertionError", &type_type, &exception_type, &dict_type);
        let attribute_error =
            create_type("AttributeError", &type_type, &exception_type, &dict_type);
        let import_error = create_type("ImportError", &type_type, &exception_type, &dict_type);
        let index_error = create_type("IndexError", &type_type, &exception_type, &dict_type);
        let key_error = create_type("KeyError", &type_type, &exception_type, &dict_type);
        let name_error = create_type("NameError", &type_type, &exception_type, &dict_type);
        let os_error = create_type("OSError", &type_type, &exception_type, &dict_type);
        let runtime_error = create_type("RuntimeError", &type_type, &exception_type, &dict_type);
        let stop_iteration = create_type("StopIteration", &type_type, &exception_type, &dict_type);
        let syntax_error = create_type("SyntaxError", &type_type, &exception_type, &dict_type);
        let type_error = create_type("TypeError", &type_type, &exception_type, &dict_type);
        let value_error = create_type("ValueError", &type_type, &exception_type, &dict_type);

        let overflow_error =
            create_type("OverflowError", &type_type, &arithmetic_error, &dict_type);
        let zero_division_error = create_type(
            "ZeroDivisionError",
            &type_type,
            &arithmetic_error,
            &dict_type,
        );

        let module_not_found_error =
            create_type("ModuleNotFoundError", &type_type, &import_error, &dict_type);

        let not_implemented_error = create_type(
            "NotImplementedError",
            &type_type,
            &runtime_error,
            &dict_type,
        );

        let file_not_found_error =
            create_type("FileNotFoundError", &type_type, &os_error, &dict_type);
        let permission_error = create_type("PermissionError", &type_type, &os_error, &dict_type);

        ExceptionZoo {
            arithmetic_error,
            assertion_error,
            attribute_error,
            base_exception_type,
            exception_type,
            file_not_found_error,
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
        }
    }
}

pub fn init(context: &PyContext) {
    let base_exception_type = &context.exceptions.base_exception_type;
    context.set_attr(
        &base_exception_type,
        "__init__",
        context.new_rustfunc(exception_init),
    );
    let exception_type = &context.exceptions.exception_type;
    context.set_attr(
        &exception_type,
        "__str__",
        context.new_rustfunc(exception_str),
    );
}
