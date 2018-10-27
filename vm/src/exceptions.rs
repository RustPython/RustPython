use super::obj::objlist;
use super::obj::objstr;
use super::obj::objtuple;
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
    zelf.set_attr("msg", msg);
    zelf.set_attr("__traceback__", traceback);
    Ok(vm.get_none())
}

// Print exception including traceback:
pub fn print_exception(vm: &mut VirtualMachine, exc: &PyObjectRef) {
    if let Some(tb) = exc.get_attr("__traceback__") {
        println!("Traceback (most recent call last):");
        if objtype::isinstance(&tb, &vm.ctx.list_type()) {
            let mut elements = objlist::get_elements(&tb);
            elements.reverse();
            for element in elements {
                if objtype::isinstance(&element, &vm.ctx.tuple_type()) {
                    let element = objtuple::get_elements(&element);
                    let filename = if let Ok(x) = vm.to_str(element[0].clone()) {
                        objstr::get_value(&x)
                    } else {
                        "<error>".to_string()
                    };

                    let lineno = if let Ok(x) = vm.to_str(element[1].clone()) {
                        objstr::get_value(&x)
                    } else {
                        "<error>".to_string()
                    };

                    let obj_name = if let Ok(x) = vm.to_str(element[2].clone()) {
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

    match vm.to_str(exc.clone()) {
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
        objstr::get_value(&m)
    } else {
        panic!("Error message must be set");
    };
    let s = format!("{}: {}", type_name, msg);
    Ok(vm.new_str(s))
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub base_exception_type: PyObjectRef,
    pub exception_type: PyObjectRef,
    pub syntax_error: PyObjectRef,
    pub assertion_error: PyObjectRef,
    pub attribute_error: PyObjectRef,
    pub name_error: PyObjectRef,
    pub runtime_error: PyObjectRef,
    pub not_implemented_error: PyObjectRef,
    pub stop_iteration: PyObjectRef,
    pub type_error: PyObjectRef,
    pub value_error: PyObjectRef,
    pub import_error: PyObjectRef,
    pub module_not_found_error: PyObjectRef,
}

impl ExceptionZoo {
    pub fn new(
        type_type: &PyObjectRef,
        object_type: &PyObjectRef,
        dict_type: &PyObjectRef,
    ) -> Self {
        let base_exception_type =
            create_type("BaseException", &type_type, &object_type, &dict_type);

        let exception_type = create_type(
            &String::from("Exception"),
            &type_type,
            &base_exception_type,
            &dict_type,
        );
        let syntax_error = create_type(
            &String::from("SyntaxError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let assertion_error = create_type(
            &String::from("AssertionError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let attribute_error = create_type(
            &String::from("AttributeError"),
            &type_type,
            &exception_type.clone(),
            &dict_type,
        );
        let name_error = create_type(
            &String::from("NameError"),
            &type_type,
            &exception_type.clone(),
            &dict_type,
        );
        let runtime_error = create_type(
            &String::from("RuntimeError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let not_implemented_error = create_type(
            &String::from("NotImplementedError"),
            &type_type,
            &runtime_error,
            &dict_type,
        );
        let stop_iteration = create_type(
            &String::from("StopIteration"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let type_error = create_type(
            &String::from("TypeError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let value_error = create_type(
            &String::from("ValueError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let import_error = create_type(
            &String::from("ImportError"),
            &type_type,
            &exception_type,
            &dict_type,
        );
        let module_not_found_error = create_type(
            &String::from("ModuleNotFoundError"),
            &type_type,
            &import_error,
            &dict_type,
        );

        ExceptionZoo {
            base_exception_type: base_exception_type,
            exception_type: exception_type,
            syntax_error: syntax_error,
            assertion_error: assertion_error,
            attribute_error: attribute_error,
            name_error: name_error,
            runtime_error: runtime_error,
            not_implemented_error: not_implemented_error,
            stop_iteration: stop_iteration,
            type_error: type_error,
            value_error: value_error,
            import_error: import_error,
            module_not_found_error: module_not_found_error,
        }
    }
}

pub fn init(context: &PyContext) {
    let ref base_exception_type = context.exceptions.base_exception_type;
    base_exception_type.set_attr("__init__", context.new_rustfunc(exception_init));
    let ref exception_type = context.exceptions.exception_type;
    exception_type.set_attr("__str__", context.new_rustfunc(exception_str));
}
