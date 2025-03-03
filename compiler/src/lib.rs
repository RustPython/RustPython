use rustpython_codegen::{compile, symboltable};
use rustpython_parser::ast::{self as ast, ConstantOptimizer, fold::Fold};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{Mode, bytecode::CodeObject};
pub use rustpython_parser::{Parse, source_code::LinearLocator};

// these modules are out of repository. re-exporting them here for convenience.
pub use rustpython_codegen as codegen;
pub use rustpython_compiler_core as core;
pub use rustpython_parser as parser;

#[derive(Debug)]
pub enum CompileErrorType {
    Codegen(rustpython_codegen::error::CodegenErrorType),
    Parse(parser::ParseErrorType),
}

impl std::error::Error for CompileErrorType {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CompileErrorType::Codegen(e) => e.source(),
            CompileErrorType::Parse(e) => e.source(),
        }
    }
}
impl std::fmt::Display for CompileErrorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

pub type CompileError = rustpython_parser::source_code::LocatedError<CompileErrorType>;

/// Compile a given source code into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: String,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let mut locator = LinearLocator::new(source);
    let mut ast = match parser::parse(source, mode.into(), &source_path) {
        Ok(x) => x,
        Err(e) => return Err(locator.locate_error(e)),
    };
    if opts.optimize > 0 {
        ast = ConstantOptimizer::new()
            .fold_mod(ast)
            .unwrap_or_else(|e| match e {});
    }
    let ast = locator.fold_mod(ast).unwrap_or_else(|e| match e {});
    compile::compile_top(&ast, source_path, mode, opts).map_err(|e| e.into())
}

pub fn compile_symtable(
    source: &str,
    mode: Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileError> {
    let mut locator = LinearLocator::new(source);
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            let ast =
                ast::Suite::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            let ast = locator.fold(ast).unwrap();
            symboltable::SymbolTable::scan_program(&ast)
        }
        Mode::Eval => {
            let expr =
                ast::Expr::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            let expr = locator.fold(expr).unwrap();
            symboltable::SymbolTable::scan_expr(&expr)
        }
    };
    res.map_err(|e| e.into_codegen_error(source_path.to_owned()).into())
}
