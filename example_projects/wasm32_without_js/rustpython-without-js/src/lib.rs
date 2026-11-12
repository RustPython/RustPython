use rustpython_vm::{Interpreter, Settings, pymodule};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn print(p: i32, l: i32) -> i32;
    fn kv_put(key_ptr: i32, key_len: i32, value_ptr: i32, value_len: i32) -> i32;
    fn kv_get(key_ptr: i32, key_len: i32, buf_ptr: i32, buf_len: i32) -> i32;
}

// The host writes source into guest-owned memory before calling eval.
#[unsafe(no_mangle)]
pub extern "C" fn alloc_source(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()).cast::<u8>()
}

fn report(message: &str) {
    unsafe { print(message.as_ptr() as i32, message.len() as i32) };
}

#[pymodule]
mod kv {
    use rustpython_vm::builtins::PyBytesRef;

    #[pyfunction]
    fn put(key: PyBytesRef, value: PyBytesRef) {
        let key = key.as_bytes();
        let value = value.as_bytes();
        unsafe {
            super::kv_put(
                key.as_ptr() as i32,
                key.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            );
        }
    }

    #[pyfunction]
    fn get(key: PyBytesRef) -> Option<Vec<u8>> {
        let key = key.as_bytes();
        let key_ptr = key.as_ptr() as i32;
        let key_len = key.len() as i32;
        let mut buf = vec![0u8; 64];
        loop {
            let len = unsafe {
                super::kv_get(key_ptr, key_len, buf.as_mut_ptr() as i32, buf.len() as i32)
            };
            // A negative length means the key is missing.
            let len = usize::try_from(len).ok()?;
            if len <= buf.len() {
                buf.truncate(len);
                return Some(buf);
            }
            buf.resize(len, 0);
        }
    }
}

/// Execute UTF-8 Python source. Return 0 on success, 1 for a Python exception,
/// or 2 for invalid UTF-8. The result or exception is sent to env.print.
///
/// # Safety
/// `source` must be the pointer returned by `alloc_source` for `len` bytes.
/// This function takes ownership of that buffer and frees it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn eval(source: *mut u8, len: usize) -> i32 {
    let bytes = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(source, len)) };
    let Ok(source) = std::str::from_utf8(&bytes) else {
        report("source is not valid UTF-8");
        return 2;
    };

    // This no-OS example has no entropy provider. Use a deterministic hash seed
    // rather than pretending that the custom getrandom backend produced bytes.
    let mut settings = Settings::default();
    settings.hash_seed = Some(0);
    let builder = Interpreter::builder(settings);
    let def = kv::module_def(&builder.ctx);
    let interpreter = builder.add_native_module(def).build();
    interpreter.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        match vm
            .run_block_expr(scope, source)
            .and_then(|value| value.repr_utf8(vm))
        {
            Ok(value) => {
                report(value.as_str());
                0
            }
            Err(exc) => {
                let mut message = String::new();
                vm.write_exception(&mut message, &exc).unwrap();
                report(&message);
                1
            }
        }
    })
}

#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    _dest: *mut u8,
    _len: usize,
) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
