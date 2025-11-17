use rustpython_vm::{Interpreter};

unsafe extern "C" {
    fn kv_get(kp: i32, kl: i32, vp: i32, vl: i32) -> i32;

    /// kp and kl are the key pointer and length in wasm memory, vp and vl are for the value
    fn kv_put(kp: i32, kl: i32, vp: i32, vl: i32) -> i32;

    fn print(p: i32, l: i32) -> i32;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval(s: *const u8, l: usize) -> i32 {
    // let src = unsafe { std::slice::from_raw_parts(s, l) };
    // let src = std::str::from_utf8(src).unwrap();
    // TODO: use src
    let src = "1 + 3";

    // 2. Execute Python code
    let interpreter = Interpreter::without_stdlib(Default::default());
    let result = interpreter.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let res = match vm.run_block_expr(scope, src) {
            Ok(val) => val,
            Err(_) => return Err(-1), // Python execution error
        };
        let repr_str = match res.repr(vm) {
            Ok(repr) => repr.as_str().to_string(),
            Err(_) => return Err(-1), // Failed to get string representation
        };
        Ok(repr_str)
    });
    let result = match result {
        Ok(r) => r,
        Err(code) => return code,
    };

    let msg = format!("eval result: {result}");

    unsafe {
        print(
            msg.as_str().as_ptr() as usize as i32,
            msg.len() as i32,
        )
    };

    0
}

#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    _dest: *mut u8,
    _len: usize,
) -> Result<(), getrandom::Error> {
    // Err(getrandom::Error::UNSUPPORTED)

    // WARNING: This function **MUST** perform proper getrandom
    Ok(())
}
