use rustpython_vm::{eval, Interpreter};

pub unsafe extern "C" fn eval(s: *const u8, l: usize) -> u32 {
    let src = std::slice::from_raw_parts(s, l);
    let src = std::str::from_utf8(src).unwrap();
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let res = eval::eval(vm, src, vm.new_scope_with_builtins(), "<string>").unwrap();
        res.try_into_value(vm).unwrap()
    })
}

fn getrandom_always_fail(_buf: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}

getrandom::register_custom_getrandom!(getrandom_always_fail);
