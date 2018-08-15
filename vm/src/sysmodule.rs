// use super::pyobject::{Executor, PyObject, PyObjectKind, PyObjectRef};

/*
 * The magic sys module.
 */

use super::pyobject::{DictProtocol, PyContext, PyObjectRef};

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let path = ctx.new_list(None);
    let modules = ctx.new_dict();
    let sys_name = "sys".to_string();
    let sys_mod = ctx.new_module(&sys_name, ctx.new_scope(None));
    modules.set_item(&sys_name, sys_mod.clone());
    sys_mod.set_item(&"modules".to_string(), modules);
    sys_mod.set_item(&"path".to_string(), path);
    sys_mod
}
