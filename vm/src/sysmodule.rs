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
    let sys_name = "sys".to_string();
    let sys_mod = py_item!(ctx, mod sys {
        let modules = modules.clone();
        let argv = argv(ctx);
        fn getrefcount = sys_getrefcount;
        fn getsizeof = sys_getsizeof;
        let maxsize = ctx.new_int(std::usize::MAX.to_bigint().unwrap());
        let path = path;
        let ps1 = ctx.new_str(">>>>> ".to_string());
        let ps2 = ctx.new_str("..... ".to_string());
        fn _getframe = getframe;
    });
    ctx.set_item(&modules, &sys_name, sys_mod.clone());
    sys_mod
}
