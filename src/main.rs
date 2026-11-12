use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() -> std::process::ExitCode {
    let mut config = InterpreterBuilder::new();
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    // SAFETY: the executable has not started threads or initialized the VM.
    unsafe { rustpython::run_with_locale(config) }
}
