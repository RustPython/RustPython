use ruff_source_file::{PositionEncoding, SourceFile, SourceFileBuilder, SourceLocation};
use rustpython_codegen::{compile, symboltable};

pub use rustpython_codegen::compile::CompileOpts;
pub use rustpython_compiler_core::{Mode, bytecode::CodeObject};

// these modules are out of repository. re-exporting them here for convenience.
pub use ruff_python_ast as ast;
pub use ruff_python_parser as parser;
pub use rustpython_codegen as codegen;
pub use rustpython_compiler_core as core;
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
    pub raw_location: ruff_text_size::TextRange,
    pub location: SourceLocation,
    pub end_location: SourceLocation,
    pub source_path: String,
    /// Set when the error is an unclosed bracket (converted from EOF).
    pub is_unclosed_bracket: bool,
}

impl ::core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
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
    pub fn from_ruff_parse_error(error: parser::ParseError, source_file: &SourceFile) -> Self {
        let source_code = source_file.to_source_code();
        let source_text = source_file.source_text();

        // For EOF errors (unclosed brackets), find the unclosed bracket position
        // and adjust both the error location and message
        let mut is_unclosed_bracket = false;
        let (error_type, location, end_location) = if matches!(
            &error.error,
            parser::ParseErrorType::Lexical(parser::LexicalErrorType::Eof)
        ) {
            if let Some((bracket_char, bracket_offset)) = find_unclosed_bracket(source_text) {
                let bracket_text_size = ruff_text_size::TextSize::new(bracket_offset as u32);
                let loc = source_code.source_location(bracket_text_size, PositionEncoding::Utf8);
                let end_loc = SourceLocation {
                    line: loc.line,
                    character_offset: loc.character_offset.saturating_add(1),
                };
                let msg = format!("'{}' was never closed", bracket_char);
                is_unclosed_bracket = true;
                (parser::ParseErrorType::OtherError(msg), loc, end_loc)
            } else {
                let loc =
                    source_code.source_location(error.location.start(), PositionEncoding::Utf8);
                let end_loc =
                    source_code.source_location(error.location.end(), PositionEncoding::Utf8);
                (error.error, loc, end_loc)
            }
        } else if matches!(
            &error.error,
            parser::ParseErrorType::Lexical(parser::LexicalErrorType::IndentationError)
        ) {
            // For IndentationError, point the offset to the end of the line content
            // instead of the beginning
            let loc = source_code.source_location(error.location.start(), PositionEncoding::Utf8);
            let line_idx = loc.line.to_zero_indexed();
            let line = source_text.split('\n').nth(line_idx).unwrap_or("");
            let line_end_col = line.chars().count() + 1; // 1-indexed, past last char
            let end_loc = SourceLocation {
                line: loc.line,
                character_offset: ruff_source_file::OneIndexed::new(line_end_col)
                    .unwrap_or(loc.character_offset),
            };
            (error.error, end_loc, end_loc)
        } else {
            let loc = source_code.source_location(error.location.start(), PositionEncoding::Utf8);
            let mut end_loc =
                source_code.source_location(error.location.end(), PositionEncoding::Utf8);

            // If the error range ends at the start of a new line (column 1),
            // adjust it to the end of the previous line
            if end_loc.character_offset.get() == 1 && end_loc.line > loc.line {
                let prev_line_end = error.location.end() - ruff_text_size::TextSize::from(1);
                end_loc = source_code.source_location(prev_line_end, PositionEncoding::Utf8);
                end_loc.character_offset = end_loc.character_offset.saturating_add(1);
            }

            (error.error, loc, end_loc)
        };

        Self::Parse(ParseError {
            error: error_type,
            raw_location: error.location,
            location,
            end_location,
            source_path: source_file.name().to_owned(),
            is_unclosed_bracket,
        })
    }

    pub const fn location(&self) -> Option<SourceLocation> {
        match self {
            Self::Codegen(codegen_error) => codegen_error.location,
            Self::Parse(parse_error) => Some(parse_error.location),
        }
    }

    pub const fn python_location(&self) -> (usize, usize) {
        if let Some(location) = self.location() {
            (location.line.get(), location.character_offset.get())
        } else {
            (0, 0)
        }
    }

    pub fn python_end_location(&self) -> Option<(usize, usize)> {
        match self {
            CompileError::Codegen(_) => None,
            CompileError::Parse(parse_error) => Some((
                parse_error.end_location.line.get(),
                parse_error.end_location.character_offset.get(),
            )),
        }
    }

    pub fn source_path(&self) -> &str {
        match self {
            Self::Codegen(codegen_error) => &codegen_error.source_path,
            Self::Parse(parse_error) => &parse_error.source_path,
        }
    }
}

/// Find the last unclosed opening bracket in source code.
/// Returns the bracket character and its byte offset, or None if all brackets are balanced.
fn find_unclosed_bracket(source: &str) -> Option<(char, usize)> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    let mut in_string = false;
    let mut string_quote = '\0';
    let mut triple_quote = false;
    let mut escape_next = false;
    let mut is_raw_string = false;

    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut i = 0;

    while i < chars.len() {
        let (byte_offset, ch) = chars[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            if ch == '\\' && !is_raw_string {
                escape_next = true;
            } else if triple_quote {
                if ch == string_quote
                    && i + 2 < chars.len()
                    && chars[i + 1].1 == string_quote
                    && chars[i + 2].1 == string_quote
                {
                    in_string = false;
                    i += 3;
                    continue;
                }
            } else if ch == string_quote {
                in_string = false;
            }
            i += 1;
            continue;
        }

        // Check for comments
        if ch == '#' {
            // Skip to end of line
            while i < chars.len() && chars[i].1 != '\n' {
                i += 1;
            }
            continue;
        }

        // Check for string start (with optional prefix like r, b, f, u, rb, br, etc.)
        if ch == '\'' || ch == '"' {
            // Check up to 2 characters before the quote for string prefix
            is_raw_string = false;
            for look_back in 1..=2.min(i) {
                let prev = chars[i - look_back].1;
                if matches!(prev, 'r' | 'R') {
                    is_raw_string = true;
                    break;
                }
                if !matches!(prev, 'b' | 'B' | 'f' | 'F' | 'u' | 'U') {
                    break;
                }
            }
            string_quote = ch;
            if i + 2 < chars.len() && chars[i + 1].1 == ch && chars[i + 2].1 == ch {
                triple_quote = true;
                in_string = true;
                i += 3;
                continue;
            }
            triple_quote = false;
            in_string = true;
            i += 1;
            continue;
        }

        match ch {
            '(' | '[' | '{' => stack.push((ch, byte_offset)),
            ')' | ']' | '}' => {
                let expected = match ch {
                    ')' => '(',
                    ']' => '[',
                    '}' => '{',
                    _ => unreachable!(),
                };
                if stack.last().is_some_and(|&(open, _)| open == expected) {
                    stack.pop();
                }
            }
            _ => {}
        }

        i += 1;
    }

    stack.last().copied()
}

/// Compile a given source code into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: &str,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    // TODO: do this less hacky; ruff's parser should translate a CRLF line
    //       break in a multiline string into just an LF in the parsed value
    #[cfg(windows)]
    let source = source.replace("\r\n", "\n");
    #[cfg(windows)]
    let source = source.as_str();

    let source_file = SourceFileBuilder::new(source_path, source).finish();
    _compile(source_file, mode, opts)
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
    source_file: SourceFile,
    mode: Mode,
    opts: CompileOpts,
) -> Result<CodeObject, CompileError> {
    let parser_mode = match mode {
        Mode::Exec => parser::Mode::Module,
        Mode::Eval => parser::Mode::Expression,
        // ruff does not have an interactive mode, which is fine,
        // since these are only different in terms of compilation
        Mode::Single | Mode::BlockExpr => parser::Mode::Module,
    };
    let parsed = parser::parse(source_file.source_text(), parser_mode.into())
        .map_err(|err| CompileError::from_ruff_parse_error(err, &source_file))?;
    let ast = parsed.into_syntax();
    compile::compile_top(ast, source_file, mode, opts).map_err(|e| e.into())
}

pub fn compile_symtable(
    source: &str,
    mode: Mode,
    source_path: &str,
) -> Result<symboltable::SymbolTable, CompileError> {
    let source_file = SourceFileBuilder::new(source_path, source).finish();
    _compile_symtable(source_file, mode)
}

pub fn _compile_symtable(
    source_file: SourceFile,
    mode: Mode,
) -> Result<symboltable::SymbolTable, CompileError> {
    let res = match mode {
        Mode::Exec | Mode::Single | Mode::BlockExpr => {
            let ast = ruff_python_parser::parse_module(source_file.source_text())
                .map_err(|e| CompileError::from_ruff_parse_error(e, &source_file))?;
            symboltable::SymbolTable::scan_program(&ast.into_syntax(), source_file.clone())
        }
        Mode::Eval => {
            let ast = ruff_python_parser::parse(
                source_file.source_text(),
                parser::Mode::Expression.into(),
            )
            .map_err(|e| CompileError::from_ruff_parse_error(e, &source_file))?;
            symboltable::SymbolTable::scan_expr(
                &ast.into_syntax().expect_expression(),
                source_file.clone(),
            )
        }
    };
    res.map_err(|e| e.into_codegen_error(source_file.name().to_owned()).into())
}

#[test]
fn test_compile() {
    let code = "x = 'abc'";
    let compiled = compile(code, Mode::Single, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
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
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
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
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_lambda() {
    let code = r#"
lambda: 'a'
"#;
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_lambda2() {
    let code = r#"
(lambda x: f'hello, {x}')('world}')
"#;
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_lambda3() {
    let code = r#"
def g():
    pass
def f():
    if False:
        return lambda x: g(x)
    elif False:
        return g
    else:
        return g
"#;
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_int() {
    let code = r#"
a = 0xFF
"#;
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_bigint() {
    let code = r#"
a = 0xFFFFFFFFFFFFFFFFFFFFFFFF
"#;
    let compiled = compile(code, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_compile_fstring() {
    let code1 = r#"
assert f"1" == '1'
    "#;
    let compiled = compile(code1, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));

    let code2 = r#"
assert f"{1}" == '1'
    "#;
    let compiled = compile(code2, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
    let code3 = r#"
assert f"{1+1}" == '2'
    "#;
    let compiled = compile(code3, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));

    let code4 = r#"
assert f"{{{(lambda: f'{1}')}" == '{1'
    "#;
    let compiled = compile(code4, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));

    let code5 = r#"
assert f"a{1}" == 'a1'
    "#;
    let compiled = compile(code5, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));

    let code6 = r#"
assert f"{{{(lambda x: f'hello, {x}')('world}')}" == '{hello, world}'
    "#;
    let compiled = compile(code6, Mode::Exec, "<>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}

#[test]
fn test_simple_enum() {
    let code = r#"
import enum
@enum._simple_enum(enum.IntFlag, boundary=enum.KEEP)
class RegexFlag:
    NOFLAG = 0
    DEBUG = 1
print(RegexFlag.NOFLAG & RegexFlag.DEBUG)
"#;
    let compiled = compile(code, Mode::Exec, "<string>", CompileOpts::default());
    dbg!(compiled.expect("compile error"));
}
