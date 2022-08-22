#[cfg(feature = "rustpython-codegen")]
pub use rustpython_codegen::CompileOpts;
#[cfg(feature = "rustpython-compiler")]
pub use rustpython_compiler::*;
pub use rustpython_compiler_core::Mode;

#[cfg(not(feature = "rustpython-compiler"))]
mod error {
    #[cfg(all(feature = "rustpython-parser", feature = "rustpython-codegen"))]
    panic!("Use --features=compiler to enable both parser and codegen");
    #[cfg(feature = "rustpython-parser")]
    pub type CompileError = rustpython_compiler_core::CompileError<rustpython_parser::ParseError>;
    #[cfg(feature = "rustpython-codegen")]
    pub type CompileError =
        rustpython_compiler_core::CompileError<rustpython_codegen::CodegenError>;
}
