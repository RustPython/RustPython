use crate::{builtins::PyBaseExceptionRef, convert::ToPyException, VirtualMachine};

#[cfg(feature = "rustpython-codegen")]
pub use rustpython_codegen::CompileOpts;
#[cfg(feature = "rustpython-compiler")]
pub use rustpython_compiler::*;
pub use rustpython_compiler_core::Mode;

#[cfg(not(feature = "rustpython-compiler"))]
mod error {
    #[cfg(all(feature = "rustpython-parser", feature = "rustpython-codegen"))]
    panic!("Use --features=compiler to enable both parser and codegen");

    #[derive(Debug, thiserror::Error)]
    pub enum CompileErrorType {
        #[cfg(feature = "rustpython-codegen")]
        #[error(transparent)]
        Codegen(#[from] rustpython_codegen::error::CodegenErrorType),
        #[cfg(feature = "rustpython-parser")]
        #[error(transparent)]
        Parse(#[from] rustpython_parser::error::ParseErrorType),
    }

    pub type CompileError = rustpython_compiler_core::CompileError<CompileErrorType>;
}
#[cfg(not(feature = "rustpython-compiler"))]
pub use error::{CompileError, CompileErrorType};

impl ToPyException for CompileError {
    fn to_pyexception(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_syntax_error(self)
    }
}
