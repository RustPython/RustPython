use super::pyobject::{
    create_type, AttributeProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult,
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
    zelf.set_attr("__msg__", msg);
    zelf.set_attr("__traceback__", traceback);
    Ok(vm.get_none())
}

#[derive(Debug)]
pub struct ExceptionZoo {
    pub base_exception_type: PyObjectRef,
    pub exception_type: PyObjectRef,
    pub assertion_error: PyObjectRef,
    pub attribute_error: PyObjectRef,
    pub name_error: PyObjectRef,
    pub runtime_error: PyObjectRef,
    pub not_implemented_error: PyObjectRef,
    pub type_error: PyObjectRef,
    pub value_error: PyObjectRef,
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

        ExceptionZoo {
            base_exception_type: base_exception_type,
            exception_type: exception_type,
            assertion_error: assertion_error,
            attribute_error: attribute_error,
            name_error: name_error,
            runtime_error: runtime_error,
            not_implemented_error: not_implemented_error,
            type_error: type_error,
            value_error: value_error,
        }
    }
}

pub fn init(context: &PyContext) {
    let ref base_exception_type = context.exceptions.base_exception_type;
    base_exception_type.set_attr("__init__", context.new_rustfunc(exception_init));

    // TODO: create a whole exception hierarchy somehow?
}
