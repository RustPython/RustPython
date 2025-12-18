#[cfg(feature = "codegen")]
pub use rustpython_codegen::CompileOpts;

#[cfg(feature = "compiler")]
pub use rustpython_compiler::*;

#[cfg(not(feature = "compiler"))]
pub use rustpython_compiler_core::Mode;

#[cfg(not(feature = "compiler"))]
pub use rustpython_compiler_core as core;

#[cfg(not(feature = "compiler"))]
pub use ruff_python_parser as parser;

#[cfg(not(feature = "compiler"))]
mod error {
    #[cfg(all(feature = "parser", feature = "codegen"))]
    panic!("Use --features=compiler to enable both parser and codegen");

    #[derive(Debug, thiserror::Error)]
    pub enum CompileErrorType {
        #[cfg(feature = "codegen")]
        #[error(transparent)]
        Codegen(#[from] super::codegen::error::CodegenErrorType),
        #[cfg(feature = "parser")]
        #[error(transparent)]
        Parse(#[from] super::parser::ParseErrorType),
    }

    #[derive(Debug, thiserror::Error)]
    pub enum CompileError {
        #[cfg(feature = "codegen")]
        #[error(transparent)]
        Codegen(#[from] super::codegen::error::CodegenError),
        #[cfg(feature = "parser")]
        #[error(transparent)]
        Parse(#[from] super::parser::ParseError),
    }
}
#[cfg(not(feature = "compiler"))]
pub use error::{CompileError, CompileErrorType};

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
