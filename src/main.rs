use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

pub fn main() -> std::process::ExitCode {
    let mut config = InterpreterBuilder::new().init_hook(rustpython_capi::initialize_for_vm);
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    rustpython::run(config)
}
