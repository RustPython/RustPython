use crate::bytecode::CodeFlags;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{ItemProtocol, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;
use rustpython_compiler::compile;

fn dis_dis(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // Method or function:
    if let Ok(co) = vm.get_attribute(obj.clone(), "__code__") {
        return dis_disassemble(co, vm);
    }

    // String:
    if let Ok(co_str) = PyStringRef::try_from_object(vm, obj.clone()) {
        let code = vm
            .compile(co_str.as_str(), compile::Mode::Exec, "<string>".to_owned())
            .map_err(|err| vm.new_syntax_error(&err))?
            .into_object();
        return dis_disassemble(code, vm);
    }

    dis_disassemble(obj, vm)
}

fn dis_disassemble(co: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let code = &PyCodeRef::try_from_object(vm, co)?.code;
    print!("{}", code);
    Ok(vm.get_none())
}

fn dis_compiler_flag_names(vm: &VirtualMachine) -> PyObjectRef {
    let dict = vm.ctx.new_dict();
    for (name, flag) in CodeFlags::NAME_MAPPING {
        dict.set_item(
            &vm.ctx.new_int(flag.bits()),
            vm.ctx.new_str((*name).to_owned()),
            vm,
        )
        .unwrap();
    }
    dict.into_object()
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "dis", {
        "dis" => ctx.new_function(dis_dis),
        "disassemble" => ctx.new_function(dis_disassemble),
        "COMPILER_FLAG_NAMES" => dis_compiler_flag_names(vm),
    })
}
