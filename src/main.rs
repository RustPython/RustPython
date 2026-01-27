use rustpython::{InterpreterBuilder, InterpreterBuilderExt};

pub fn main() -> std::process::ExitCode {
    let mut config = InterpreterBuilder::new();
    #[cfg(feature = "stdlib")]
    {
        config = config.init_stdlib();
    }
    rustpython::run(config)
}
