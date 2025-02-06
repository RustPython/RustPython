#[cfg(feature = "codegen")]
pub use rustpython_codegen::CompileOpts;
#[cfg(feature = "compiler")]
pub use rustpython_compiler::*;

#[cfg(not(feature = "compiler"))]
pub use rustpython_compiler_source as source;

#[cfg(not(feature = "compiler"))]
pub use rustpython_compiler_core::Mode;

#[cfg(not(feature = "compiler"))]
pub use rustpython_compiler_core as core;

#[cfg(not(feature = "compiler"))]
pub use rustpython_parser_core as parser;

#[cfg(not(feature = "compiler"))]
mod error {
    #[cfg(all(feature = "parser", feature = "codegen"))]
    panic!("Use --features=compiler to enable both parser and codegen");

    #[derive(Debug, thiserror::Error)]
    pub enum CompileErrorType {
        #[cfg(feature = "codegen")]
        #[error(transparent)]
        Codegen(#[from] rustpython_codegen::error::CodegenErrorType),
        #[cfg(feature = "parser")]
        #[error(transparent)]
        Parse(#[from] rustpython_parser::error::ParseErrorType),
    }

    pub type CompileError = rustpython_parser_core::source_code::LocatedError<CompileErrorType>;
}
#[cfg(not(feature = "compiler"))]
pub use error::{CompileError, CompileErrorType};

#[cfg(any(feature = "parser", feature = "codegen"))]
impl crate::convert::ToPyException for (CompileError, Option<&str>) {
    fn to_pyexception(&self, vm: &crate::VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
        vm.new_syntax_error(&self.0, self.1)
    }
}
