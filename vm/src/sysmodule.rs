use super::pyobject::{DictProtocol, PyContext, PyObjectRef};
use std::env;

/*
 * The magic sys module.
 */

fn argv(ctx: &PyContext) -> PyObjectRef {
    let mut argv: Vec<PyObjectRef> = env::args().map(|x| ctx.new_str(x)).collect();
    argv.remove(0);
    ctx.new_list(argv)
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
    sys_mod.set_item(&"modules".to_string(), modules);
    sys_mod.set_item(&"argv".to_string(), argv(ctx));
    sys_mod.set_item(&"path".to_string(), path);
    sys_mod
}
