use num_bigint::ToBigInt;
use obj::objtype;
use pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use std::rc::Rc;
use std::{env, mem};
use vm::VirtualMachine;

/*
 * The magic sys module.
 */

fn argv(ctx: &PyContext) -> PyObjectRef {
    let mut argv: Vec<PyObjectRef> = env::args().map(|x| ctx.new_str(x)).collect();
    argv.remove(0);
    ctx.new_list(argv)
}

fn getframe(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    if let Some(frame) = &vm.current_frame {
        Ok(frame.clone())
    } else {
        panic!("Current frame is undefined!")
    }
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
    let sys_name = "sys";
    let sys_mod = ctx.new_module(&sys_name, ctx.new_scope(None));
    ctx.set_item(&modules, sys_name, sys_mod.clone());
    ctx.set_item(&sys_mod, "modules", modules);
    ctx.set_item(&sys_mod, "argv", argv(ctx));
    ctx.set_item(&sys_mod, "getrefcount", ctx.new_rustfunc(sys_getrefcount));
    ctx.set_item(&sys_mod, "getsizeof", ctx.new_rustfunc(sys_getsizeof));
    let maxsize = ctx.new_int(std::usize::MAX.to_bigint().unwrap());
    ctx.set_item(&sys_mod, "maxsize", maxsize);
    ctx.set_item(&sys_mod, "path", path);
    ctx.set_item(&sys_mod, "ps1", ctx.new_str(">>>>> ".to_string()));
    ctx.set_item(&sys_mod, "ps2", ctx.new_str("..... ".to_string()));
    ctx.set_item(&sys_mod, "_getframe", ctx.new_rustfunc(getframe));
    sys_mod
}
