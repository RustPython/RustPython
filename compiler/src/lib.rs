use ruff_source_file::SourceLocation;
use rustpython_codegen::{compile, symboltable};
// use rustpython_parser::ast::{self as ast, fold::Fold, ConstantOptimizer};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{bytecode::CodeObject, Mode};
use rustpython_compiler_source::SourceCode;
// pub use rustpython_parser::{source_code::LinearLocator, Parse};

// these modules are out of repository. re-exporting them here for convenience.
pub use ruff_python_ast as ast;
pub use ruff_python_parser as parser;
pub use rustpython_codegen as codegen;
pub use rustpython_compiler_core as core;
pub use rustpython_compiler_source as source;
use thiserror::Error;

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
    pub source_path: String,
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
            source_path: source_code.path.to_owned(),
        })
    }

    pub fn location(&self) -> Option<SourceLocation> {
        match self {
            CompileError::Codegen(codegen_error) => codegen_error.location.clone(),
            CompileError::Parse(parse_error) => Some(parse_error.location.clone()),
        }
    }

    pub fn python_location(&self) -> (usize, usize) {
        match self {
            CompileError::Codegen(codegen_error) => {
                if let Some(location) = &codegen_error.location {
                    (location.row.get(), location.column.get())
                } else {
                    (0, 0)
                }
            }
            CompileError::Parse(parse_error) => (
                parse_error.location.row.get(),
                parse_error.location.column.get(),
            ),
        }
    }

    pub fn source_path(&self) -> &str {
        match self {
            CompileError::Codegen(codegen_error) => &codegen_error.source_path,
            CompileError::Parse(parse_error) => &parse_error.source_path,
        }
    }
}

/// Compile a given source code into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: &str,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let source_code = SourceCode::new(source_path, source);
    _compile(source_code, mode, opts)
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
    source_code: SourceCode,
    mode: core::Mode,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let parsed = parser::parse(source_code.text, mode.into())
        .map_err(|err| CompileError::from_ruff_parse_error(err, &source_code))?;
    let ast = parsed.into_syntax();
    compile::compile_top(ast, source_code, mode, opts).map_err(|e| e.into())
}

pub fn compile_symtable(
    source: &str,
    mode: Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileError> {
    let source_code = SourceCode::new(source_path, source);
    _compile_symtable(source_code, mode)
}

pub fn _compile_symtable(
    source_code: SourceCode,
    mode: Mode,
) -> Result<symboltable::SymbolTable, CompileError> {
    // let mut locator = LinearLocator::new(source);
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            let ast = ruff_python_parser::parse_module(source_code.text)
                .map_err(|e| CompileError::from_ruff_parse_error(e, &source_code))?;
            // let ast =
            //     ast::Suite::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            // let ast = locator.fold(ast).unwrap();
            symboltable::SymbolTable::scan_program(&ast.into_syntax(), source_code.clone())
        }
        Mode::Eval => {
            // let expr =
            //     ast::Expr::parse(source, source_path).map_err(|e| locator.locate_error(e))?;
            // let expr = locator.fold(expr).unwrap();
            let ast =
                ruff_python_parser::parse(source_code.text, ruff_python_parser::Mode::Ipython)
                    .map_err(|e| CompileError::from_ruff_parse_error(e, &source_code))?;
            symboltable::SymbolTable::scan_expr(
                &ast.into_syntax().expect_expression(),
                source_code.clone(),
            )
        }
    };
    res.map_err(|e| e.into_codegen_error(source_code.path.to_owned()).into())
}

#[test]
fn test_compile() {
    let code = "x = 'abc'";
    let compiled = compile(&code, Mode::Single, "<>", CompileOpts::default());
    dbg!(compiled);
}

#[test]
fn test_compile_phello() {
    let code = r#"
initialized = True
def main():
    print("Hello world!")
if __name__ == '__main__':
    main()
"#;
    let compiled = compile(&code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_if_elif_else() {
    let code = r#"
if False:
    pass
elif False:
    pass
elif False:
    pass
else:
    pass
"#;
    let compiled = compile(&code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}
