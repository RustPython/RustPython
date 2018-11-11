use super::pyobject::{DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult};
use super::vm::VirtualMachine;
use std::env;

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
    sys_mod.set_item("path", path);
    sys_mod.set_item("_getframe", ctx.new_rustfunc(getframe));
    sys_mod
}
