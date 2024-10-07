use ruff_source_file::{LineIndex, SourceCode, SourceLocation};
use rustpython_codegen::{compile, symboltable};
// use rustpython_parser::ast::{self as ast, fold::Fold, ConstantOptimizer};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{bytecode::CodeObject, Mode};
// pub use rustpython_parser::{source_code::LinearLocator, Parse};

// these modules are out of repository. re-exporting them here for convenience.
pub use ruff_python_ast as ast;
pub use ruff_python_parser as parser;
pub use rustpython_codegen as codegen;
pub use rustpython_compiler_core as core;
use thiserror::Error;

pub mod source_file {
    pub use ruff_source_file::OneIndexed as LineNumber;
}

#[derive(Error, Debug)]
pub enum CompileErrorType {
    #[error(transparent)]
    Codegen(#[from] codegen::error::CodegenErrorType),
    #[error(transparent)]
    Parse(#[from] parser::ParseErrorType),
}

#[derive(Error, Debug)]
pub struct ParseError {
    #[source]
    pub error: parser::ParseErrorType,
    pub location: SourceLocation,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

#[derive(Error, Debug)]
pub enum CompileError {
    #[error(transparent)]
    Codegen(#[from] codegen::error::CodegenError),
    #[error(transparent)]
    Parse(#[from] ParseError),
}

impl CompileError {
    pub fn from_ruff_parse_error(error: parser::ParseError, source_code: &SourceCode) -> Self {
        let location = source_code.source_location(error.location.start());
        Self::Parse(ParseError {
            error: error.error,
            location,
        })
    }

    pub fn location(&self) -> Option<SourceLocation> {
        match self {
            CompileError::Codegen(codegen_error) => codegen_error.location.clone(),
            CompileError::Parse(parse_error) => Some(parse_error.location.clone()),
        }
    }
}

/// Compile a given source code into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: String,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let index = LineIndex::from_source_text(source);
    let source_code = SourceCode::new(source, &index);
    _compile(source_path, source_code, mode, opts)
    // let index = LineIndex::from_source_text(source);
    // let source_code = SourceCode::new(source, &index);
    // let mut locator = LinearLocator::new(source);
    // let mut ast = match parser::parse(source, mode.into(), &source_path) {
    //     Ok(x) => x,
    //     Err(e) => return Err(locator.locate_error(e)),
    // };

    // TODO:
    // if opts.optimize > 0 {
    //     ast = ConstantOptimizer::new()
    //         .fold_mod(ast)
    //         .unwrap_or_else(|e| match e {});
    // }
    // let ast = locator.fold_mod(ast).unwrap_or_else(|e| match e {});
}

fn _compile(
    source_path: String,
    source_code: SourceCode,
    mode: core::Mode,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let parsed = parser::parse(source_code.text(), mode.into())
        .map_err(|err| CompileError::from_ruff_parse_error(err, &source_code))?;
    let ast = parsed.into_syntax();
    compile::compile_top(ast, source_path, source_code.text(), mode, opts).map_err(|e| e.into())
}

pub fn compile_symtable(
    source: &str,
    mode: Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileErrorType> {
    // let mut locator = LinearLocator::new(source);
    let index = LineIndex::from_source_text(source);
    let source_code = SourceCode::new(source, &index);
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            let ast = ruff_python_parser::parse_module(source).map_err(|e| e.error)?;
            // let ast =
            //     ast::Suite::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            // let ast = locator.fold(ast).unwrap();
            symboltable::SymbolTable::scan_program(&ast.into_syntax(), source_code)
        }
        Mode::Eval => {
            // let expr =
            //     ast::Expr::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            // let expr = locator.fold(expr).unwrap();
            let ast = ruff_python_parser::parse(source, ruff_python_parser::Mode::Ipython)
                .map_err(|e| e.error)?;
            symboltable::SymbolTable::scan_expr(&ast.into_syntax().expect_expression(), source_code)
        }
    };
    res.map_err(|e| e.into_codegen_error(source_path.to_owned()).error.into())
}
