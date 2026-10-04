#[cfg(all(not(feature = "compiler"), feature = "parser", feature = "codegen",))]
compile_error!("Use --features=compiler to enable both parser and codegen");

#[cfg(feature = "codegen")]
pub use rustpython_codegen::CompileOpts;

cfg_select! {
    feature = "compiler" => {
        pub use rustpython_compiler::*;
    }
    _ => {
        pub use ruff_python_parser as parser;

        pub use rustpython_compiler_core::Mode;
        pub use rustpython_compiler_core as core;
    }
}

#[cfg(not(feature = "compiler"))]
#[derive(Debug, thiserror::Error)]
pub enum CompileErrorType {
    #[cfg(feature = "codegen")]
    #[error(transparent)]
    Codegen(#[from] super::codegen::error::CodegenErrorType),
    #[cfg(feature = "parser")]
    #[error(transparent)]
    Parse(#[from] super::parser::ParseErrorType),
}

#[cfg(not(feature = "compiler"))]
#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[cfg(feature = "codegen")]
    #[error(transparent)]
    Codegen(#[from] super::codegen::error::CodegenError),
    #[cfg(feature = "parser")]
    #[error(transparent)]
    Parse(#[from] super::parser::ParseError),
}

#[cfg(any(feature = "parser", feature = "codegen"))]
impl crate::convert::ToPyException for (CompileError, Option<&str>) {
    fn to_pyexception(&self, vm: &crate::VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        vm.new_syntax_error(&self.0, self.1)
    }
}

#[cfg(any(feature = "parser", feature = "codegen"))]
impl crate::convert::ToPyException for (CompileError, Option<&str>, bool) {
    fn to_pyexception(&self, vm: &crate::VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        vm.new_syntax_error_maybe_incomplete(&self.0, self.1, self.2)
    }
}
