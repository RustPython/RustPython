use rustpython_codegen::{compile, symboltable};
use rustpython_parser::{
    self as parser,
    ast::{fold::Fold, ConstantOptimizer},
};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{BaseError as CompileErrorBody, CodeObject, Mode};

use std::error::Error as StdError;
use std::fmt;

#[derive(Debug)]
pub enum CompileErrorType {
    Codegen(rustpython_codegen::error::CodegenErrorType),
    Parse(parser::ParseErrorType),
}

impl StdError for CompileErrorType {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            CompileErrorType::Codegen(e) => e.source(),
            CompileErrorType::Parse(e) => e.source(),
        }
    }
}
impl fmt::Display for CompileErrorType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CompileErrorType::Codegen(e) => e.fmt(f),
            CompileErrorType::Parse(e) => e.fmt(f),
        }
    }
}
impl From<rustpython_codegen::error::CodegenErrorType> for CompileErrorType {
    fn from(source: rustpython_codegen::error::CodegenErrorType) -> Self {
        CompileErrorType::Codegen(source)
    }
}
impl From<parser::ParseErrorType> for CompileErrorType {
    fn from(source: parser::ParseErrorType) -> Self {
        CompileErrorType::Parse(source)
    }
}

pub type CompileError = rustpython_compiler_core::CompileError<CompileErrorType>;

fn error_from_parse(error: parser::ParseError, source: &str) -> CompileError {
    let error: CompileErrorBody<parser::ParseErrorType> = error.into();
    CompileError::from(error, source)
}

/// Compile a given sourcecode into a bytecode object.
pub fn compile(
    source: &str,
    mode: compile::Mode,
    source_path: String,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let mut ast = match parser::parse(source, mode.into(), &source_path) {
        Ok(x) => x,
        Err(e) => return Err(error_from_parse(e, source)),
    };
    if opts.optimize > 0 {
        ast = ConstantOptimizer::new()
            .fold_mod(ast)
            .unwrap_or_else(|e| match e {});
    }
    compile::compile_top(&ast, source_path, mode, opts).map_err(|e| CompileError::from(e, source))
}

pub fn compile_symtable(
    source: &str,
    mode: compile::Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileError> {
    let parse_err = |e| error_from_parse(e, source);
    let res = match mode {
        compile::Mode::Exec | compile::Mode::Single | compile::Mode::BlockExpr => {
            let ast = parser::parse_program(source, source_path).map_err(parse_err)?;
            symboltable::SymbolTable::scan_program(&ast)
        }
        compile::Mode::Eval => {
            let expr = parser::parse_expression(source, source_path).map_err(parse_err)?;
            symboltable::SymbolTable::scan_expr(&expr)
        }
    };
    res.map_err(|e| CompileError::from(e.into_codegen_error(source_path.to_owned()), source))
}
