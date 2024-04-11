#[cfg(feature = "rustpython-codegen")]
pub use rustpython_codegen::CompileOpts;
#[cfg(feature = "rustpython-compiler")]
pub use rustpython_compiler::*;
#[cfg(not(feature = "rustpython-compiler"))]
pub use rustpython_compiler_core::Mode;

#[cfg(not(feature = "rustpython-compiler"))]
pub use rustpython_compiler_core as core;

#[cfg(not(feature = "rustpython-compiler"))]
pub use rustpython_parser_core as parser;

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

    pub type CompileError = rustpython_parser_core::source_code::LocatedError<CompileErrorType>;
}
#[cfg(not(feature = "rustpython-compiler"))]
pub use error::{CompileError, CompileErrorType};

#[cfg(any(feature = "rustpython-parser", feature = "rustpython-codegen"))]
impl crate::convert::ToPyException for (CompileError, Option<&str>) {
    fn to_pyexception(&self, vm: &crate::VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        vm.new_syntax_error(&self.0, self.1)
    }
}
