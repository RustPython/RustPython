use std::error::Error;
use std::path::Path;
use std::fs::File;
use std::io::Read;

use super::python;
use super::ast;
use super::lexer;

pub fn read_file(filename: &Path) -> Result<String, String> {
    match File::open(&filename) {
        Ok(mut file) => {
            let mut s = String::new();

            match file.read_to_string(&mut s) {
                Err(why) => Err(String::from("Reading file failed: ") + why.description()),
                Ok(_) => Ok(s),
            }
        }
        Err(why) => Err(String::from("Opening file failed: ") + why.description()),
    }
}

/*
 * Parse python code.
 * Grammar may be inspired by antlr grammar for python:
 * https://github.com/antlr/grammars-v4/tree/master/python3
 */

pub fn parse(filename: &Path) -> Result<ast::Program, String> {
    info!("Parsing: {}", filename.display());
    match read_file(filename) {
        Ok(txt) => {
            debug!("Read contents of file: {}", txt);
            parse_program(&txt)
        }
        Err(msg) => Err(msg),
    }
}

pub fn parse_program(source: &String) -> Result<ast::Program, String> {
    let lxr = lexer::Lexer::new(&source);
    match python::ProgramParser::new().parse(lxr) {
        Err(why) => Err(String::from(format!("{:?}", why))),
        Ok(p) => Ok(p),
    }
}

pub fn parse_statement(source: &String) -> Result<ast::Statement, String> {
    let lxr = lexer::Lexer::new(&source);
    match python::StatementParser::new().parse(lxr) {
        Err(why) => Err(String::from(format!("{:?}", why))),
        Ok(p) => Ok(p),
    }
}

pub fn parse_expression(source: &String) -> Result<ast::Expression, String> {
    let lxr = lexer::Lexer::new(&source);
    match python::ExpressionParser::new().parse(lxr) {
        Err(why) => Err(String::from(format!("{:?}", why))),
        Ok(p) => Ok(p),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_program;
    use super::ast;

    #[test]
    fn test_parse_print_hello() {
        let source = String::from("print('Hello world')\n");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![
                    ast::Statement::Expression {
                        expression: ast::Expression::Call {
                            function: Box::new(ast::Expression::Identifier {
                                name: String::from("print"),
                            }),
                            args: vec![
                                ast::Expression::String {
                                    value: String::from("Hello world"),
                                },
                            ],
                        },
                    },
                ],
            }
        );
    }

    #[test]
    fn test_parse_print_2() {
        let source = String::from("print('Hello world', 2)\n");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![
                    ast::Statement::Expression {
                        expression: ast::Expression::Call {
                            function: Box::new(ast::Expression::Identifier {
                                name: String::from("print"),
                            }),
                            args: vec![
                                ast::Expression::String {
                                    value: String::from("Hello world"),
                                },
                                ast::Expression::Number { value: 2 },
                            ],
                        },
                    },
                ],
            }
        );
    }
}
