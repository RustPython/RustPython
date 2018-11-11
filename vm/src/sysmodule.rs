use super::obj::objint;
use super::obj::objtype;
use super::pyobject::{DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::vm::VirtualMachine;
use num_bigint::ToBigInt;
use num_traits::ToPrimitive;
use std::env;
use std::rc::Rc;

/*
 * The magic sys module.
 */

fn argv(ctx: &PyContext) -> PyObjectRef {
    let mut argv: Vec<PyObjectRef> = env::args().map(|x| ctx.new_str(x)).collect();
    argv.remove(0);
    ctx.new_list(argv)
}

fn getframe(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [],
        optional = [(offset, Some(vm.ctx.int_type()))]
    );
    let idx = match offset {
        Some(int) => {
            if let Some(offset) = objint::get_value(int).to_usize() {
                if offset > vm.frames.len() - 1 {
                    return Err(vm.new_value_error("call stack is not deep enough".to_string()));
                }
                offset
            } else {
                0
            }
        }
        None => 0,
    };

    let idx = vm.frames.len() - idx - 1;

    let frame = &vm.frames[idx];
    Ok(frame.clone())
}

fn sys_getrefcount(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    let size = Rc::strong_count(&object);
    Ok(vm.ctx.new_int(size.to_bigint().unwrap()))
}

fn sys_getsizeof(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    // TODO: implement default optional argument.
    let size = mem::size_of_val(&object.borrow());
    Ok(vm.ctx.new_int(size.to_bigint().unwrap()))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let path_list = match env::var_os("PYTHONPATH") {
        Some(paths) => env::split_paths(&paths)
            .map(|path| ctx.new_str(path.to_str().unwrap().to_string()))
            .collect(),
        None => vec![],
    };
    let path = ctx.new_list(path_list);
    let modules = ctx.new_dict();
    let sys_name = "sys".to_string();
    let sys_mod = ctx.new_module(&sys_name, ctx.new_scope(None));
    modules.set_item(&sys_name, sys_mod.clone());
    sys_mod.set_item("modules", modules);
    sys_mod.set_item("argv", argv(ctx));
    sys_mod.set_item("getrefcount", ctx.new_rustfunc(sys_getrefcount));
    sys_mod.set_item("getsizeof", ctx.new_rustfunc(sys_getsizeof));
    let maxsize = ctx.new_int(std::usize::MAX.to_bigint().unwrap());
    sys_mod.set_item("maxsize", maxsize);
    sys_mod.set_item("path", path);
    sys_mod.set_item("ps1", ctx.new_str(">>>>> ".to_string()));
    sys_mod.set_item("ps2", ctx.new_str("..... ".to_string()));
    sys_mod.set_item("_getframe", ctx.new_rustfunc(getframe));
    sys_mod
}
