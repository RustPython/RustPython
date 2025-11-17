use rustpython_vm::{Interpreter, eval};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval(s: *const u8, l: usize) -> u32 {
    let src = std::slice::from_raw_parts(s, l);
    let src = std::str::from_utf8(src).unwrap();
    Interpreter::without_stdlib(Default::default()).enter(|vm| {
        let res = eval::eval(vm, src, vm.new_scope_with_builtins(), "<string>").unwrap();
        res.try_into_value(vm).unwrap()
    })
}

#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    _dest: *mut u8,
    _len: usize,
) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
