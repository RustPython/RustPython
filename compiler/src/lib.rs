use rustpython_codegen::{compile, symboltable};
use rustpython_compiler_core::CodeObject;
use rustpython_parser::{
    ast::{fold::Fold, ConstantOptimizer},
    error::ParseErrorType,
    parser,
};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{BaseError as CompileErrorBody, Mode};

#[derive(Debug, thiserror::Error)]
pub enum CompileErrorType {
    #[error(transparent)]
    Codegen(#[from] rustpython_codegen::error::CodegenErrorType),
    #[error(transparent)]
    Parse(#[from] rustpython_parser::error::ParseErrorType),
}

pub type CompileError = rustpython_compiler_core::CompileError<CompileErrorType>;

fn error_from_parse(error: rustpython_parser::error::ParseError, source: &str) -> CompileError {
    let error: CompileErrorBody<ParseErrorType> = error.into();
    CompileError::from(error, source)
}

/// Compile a given sourcecode into a bytecode object.
pub fn compile(
    source: &str,
    mode: compile::Mode,
    source_path: String,
    opts: compile::CompileOpts,
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
