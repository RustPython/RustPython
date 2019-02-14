extern crate lalrpop_util;

use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::iter;
use std::mem;
use std::path::Path;

use self::lalrpop_util::ParseError;

use super::ast;
use super::lexer;
use super::python;
use super::token;

pub fn read_file(filename: &Path) -> Result<String, String> {
    info!("Loading file {:?}", filename);
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
    let txt = read_file(filename)?;
    debug!("Read contents of file: {}", txt);
    parse_program(&txt)
}

macro_rules! do_lalr_parsing {
    ($input: expr, $pat: ident, $tok: ident) => {{
        let lxr = lexer::make_tokenizer($input);
        let marker_token = (Default::default(), token::Tok::$tok, Default::default());
        let tokenizer = iter::once(Ok(marker_token)).chain(lxr);

        match python::TopParser::new().parse(tokenizer) {
            Err(why) => Err(format!("{:?}", why)),
            Ok(top) => {
                if let ast::Top::$pat(x) = top {
                    Ok(x)
                } else {
                    unreachable!()
                }
            }
        }
    }};
}

pub fn parse_program(source: &str) -> Result<ast::Program, String> {
    do_lalr_parsing!(source, Program, StartProgram)
}

pub fn parse_statement(source: &str) -> Result<ast::LocatedStatement, String> {
    do_lalr_parsing!(source, Statement, StartStatement)
}

/// Parses a python expression
///
/// # Example
/// ```
/// extern crate num_bigint;
/// extern crate rustpython_parser;
/// use num_bigint::BigInt;
/// use rustpython_parser::{parser, ast};
/// let expr = parser::parse_expression("1+2").unwrap();
///
/// assert_eq!(ast::Expression::Binop {
///         a: Box::new(ast::Expression::Number {
///             value: ast::Number::Integer { value: BigInt::from(1) }
///         }),
///         op: ast::Operator::Add,
///         b: Box::new(ast::Expression::Number {
///             value: ast::Number::Integer { value: BigInt::from(2) }
///         })
///     },
///     expr);
///
/// ```
pub fn parse_expression(source: &str) -> Result<ast::Expression, String> {
    do_lalr_parsing!(source, Expression, StartExpression)
}

pub enum FStringError {
    UnclosedLbrace,
    UnopenedRbrace,
    InvalidExpression,
}

impl From<FStringError> for ParseError<lexer::Location, token::Tok, lexer::LexicalError> {
    fn from(_err: FStringError) -> Self {
        // TODO: we should have our own top-level ParseError to properly propagate f-string (and
        // other) syntax errors
        ParseError::User {
            error: lexer::LexicalError::StringError,
        }
    }
}

pub fn parse_fstring(source: &str) -> Result<ast::StringGroup, FStringError> {
    let mut values = vec![];
    let mut start = 0;
    let mut depth = 0;
    let mut escaped = false;
    let mut content = String::new();

    let mut chars = source.char_indices().peekable();
    while let Some((pos, ch)) = chars.next() {
        match ch {
            '{' | '}' if escaped => {
                content.push(ch);
                escaped = false;
            }
            '{' => {
                if depth == 0 {
                    if let Some((_, '{')) = chars.peek() {
                        escaped = true;
                        continue;
                    }

                    values.push(ast::StringGroup::Constant {
                        value: mem::replace(&mut content, String::new()),
                    });

                    start = pos + 1;
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    if let Some((_, '}')) = chars.peek() {
                        escaped = true;
                        continue;
                    }

                    return Err(FStringError::UnopenedRbrace);
                }

                depth -= 1;
                if depth == 0 {
                    values.push(ast::StringGroup::FormattedValue {
                        value: Box::new(match parse_expression(source[start..pos].trim()) {
                            Ok(expr) => expr,
                            Err(_) => return Err(FStringError::InvalidExpression),
                        }),
                    });
                }
            }
            ch => {
                if depth == 0 {
                    content.push(ch);
                }
            }
        }
    }

    if depth != 0 {
        return Err(FStringError::UnclosedLbrace);
    }

    if !content.is_empty() {
        values.push(ast::StringGroup::Constant { value: content })
    }

    Ok(match values.len() {
        0 => ast::StringGroup::Constant {
            value: "".to_string(),
        },
        1 => values.into_iter().next().unwrap(),
        _ => ast::StringGroup::Joined { values },
    })
}

#[cfg(test)]
mod tests {
    use super::ast;
    use super::parse_expression;
    use super::parse_program;
    use super::parse_statement;
    use num_bigint::BigInt;

    #[test]
    fn test_parse_empty() {
        let parse_ast = parse_program(&String::from("\n"));

        assert_eq!(parse_ast, Ok(ast::Program { statements: vec![] }))
    }

    #[test]
    fn test_parse_print_hello() {
        let source = String::from("print('Hello world')\n");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![ast::LocatedStatement {
                    location: ast::Location::new(1, 1),
                    node: ast::Statement::Expression {
                        expression: ast::Expression::Call {
                            function: Box::new(ast::Expression::Identifier {
                                name: String::from("print"),
                            }),
                            args: vec![ast::Expression::String {
                                value: String::from("Hello world"),
                            }],
                            keywords: vec![],
                        },
                    },
                },],
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
                statements: vec![ast::LocatedStatement {
                    location: ast::Location::new(1, 1),
                    node: ast::Statement::Expression {
                        expression: ast::Expression::Call {
                            function: Box::new(ast::Expression::Identifier {
                                name: String::from("print"),
                            }),
                            args: vec![
                                ast::Expression::String {
                                    value: String::from("Hello world"),
                                },
                                ast::Expression::Number {
                                    value: ast::Number::Integer {
                                        value: BigInt::from(2)
                                    },
                                }
                            ],
                            keywords: vec![],
                        },
                    },
                },],
            }
        );
    }

    #[test]
    fn test_parse_kwargs() {
        let source = String::from("my_func('positional', keyword=2)\n");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![ast::LocatedStatement {
                    location: ast::Location::new(1, 1),
                    node: ast::Statement::Expression {
                        expression: ast::Expression::Call {
                            function: Box::new(ast::Expression::Identifier {
                                name: String::from("my_func"),
                            }),
                            args: vec![ast::Expression::String {
                                value: String::from("positional"),
                            }],
                            keywords: vec![ast::Keyword {
                                name: Some("keyword".to_string()),
                                value: ast::Expression::Number {
                                    value: ast::Number::Integer {
                                        value: BigInt::from(2)
                                    },
                                }
                            }],
                        },
                    },
                },],
            }
        );
    }

    #[test]
    fn test_parse_if_elif_else() {
        let source = String::from("if 1: 10\nelif 2: 20\nelse: 30\n");
        let parse_ast = parse_statement(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::LocatedStatement {
                location: ast::Location::new(1, 1),
                node: ast::Statement::If {
                    test: ast::Expression::Number {
                        value: ast::Number::Integer {
                            value: BigInt::from(1)
                        },
                    },
                    body: vec![ast::LocatedStatement {
                        location: ast::Location::new(1, 7),
                        node: ast::Statement::Expression {
                            expression: ast::Expression::Number {
                                value: ast::Number::Integer {
                                    value: BigInt::from(10)
                                },
                            }
                        },
                    },],
                    orelse: Some(vec![ast::LocatedStatement {
                        location: ast::Location::new(2, 1),
                        node: ast::Statement::If {
                            test: ast::Expression::Number {
                                value: ast::Number::Integer {
                                    value: BigInt::from(2)
                                },
                            },
                            body: vec![ast::LocatedStatement {
                                location: ast::Location::new(2, 9),
                                node: ast::Statement::Expression {
                                    expression: ast::Expression::Number {
                                        value: ast::Number::Integer {
                                            value: BigInt::from(20)
                                        },
                                    },
                                },
                            },],
                            orelse: Some(vec![ast::LocatedStatement {
                                location: ast::Location::new(3, 7),
                                node: ast::Statement::Expression {
                                    expression: ast::Expression::Number {
                                        value: ast::Number::Integer {
                                            value: BigInt::from(30)
                                        },
                                    },
                                },
                            },]),
                        }
                    },]),
                }
            }
        );
    }

    #[test]
    fn test_parse_lambda() {
        let source = String::from("lambda x, y: x * y\n"); // lambda(x, y): x * y");
        let parse_ast = parse_statement(&source);
        assert_eq!(
            parse_ast,
            Ok(ast::LocatedStatement {
                location: ast::Location::new(1, 1),
                node: ast::Statement::Expression {
                    expression: ast::Expression::Lambda {
                        args: ast::Parameters {
                            args: vec![String::from("x"), String::from("y")],
                            kwonlyargs: vec![],
                            vararg: None,
                            kwarg: None,
                            defaults: vec![],
                            kw_defaults: vec![],
                        },
                        body: Box::new(ast::Expression::Binop {
                            a: Box::new(ast::Expression::Identifier {
                                name: String::from("x"),
                            }),
                            op: ast::Operator::Mult,
                            b: Box::new(ast::Expression::Identifier {
                                name: String::from("y"),
                            })
                        })
                    }
                }
            })
        )
    }

    #[test]
    fn test_parse_tuples() {
        let source = String::from("a, b = 4, 5\n");

        assert_eq!(
            parse_statement(&source),
            Ok(ast::LocatedStatement {
                location: ast::Location::new(1, 1),
                node: ast::Statement::Assign {
                    targets: vec![ast::Expression::Tuple {
                        elements: vec![
                            ast::Expression::Identifier {
                                name: "a".to_string()
                            },
                            ast::Expression::Identifier {
                                name: "b".to_string()
                            }
                        ]
                    }],
                    value: ast::Expression::Tuple {
                        elements: vec![
                            ast::Expression::Number {
                                value: ast::Number::Integer {
                                    value: BigInt::from(4)
                                }
                            },
                            ast::Expression::Number {
                                value: ast::Number::Integer {
                                    value: BigInt::from(5)
                                }
                            }
                        ]
                    }
                }
            })
        )
    }

    #[test]
    fn test_parse_class() {
        let source = String::from("class Foo(A, B):\n def __init__(self):\n  pass\n def method_with_default(self, arg='default'):\n  pass\n");
        assert_eq!(
            parse_statement(&source),
            Ok(ast::LocatedStatement {
                location: ast::Location::new(1, 1),
                node: ast::Statement::ClassDef {
                    name: String::from("Foo"),
                    bases: vec![
                        ast::Expression::Identifier {
                            name: String::from("A")
                        },
                        ast::Expression::Identifier {
                            name: String::from("B")
                        }
                    ],
                    keywords: vec![],
                    body: vec![
                        ast::LocatedStatement {
                            location: ast::Location::new(2, 2),
                            node: ast::Statement::FunctionDef {
                                name: String::from("__init__"),
                                args: ast::Parameters {
                                    args: vec![String::from("self")],
                                    kwonlyargs: vec![],
                                    vararg: None,
                                    kwarg: None,
                                    defaults: vec![],
                                    kw_defaults: vec![],
                                },
                                body: vec![ast::LocatedStatement {
                                    location: ast::Location::new(3, 3),
                                    node: ast::Statement::Pass,
                                }],
                                decorator_list: vec![],
                            }
                        },
                        ast::LocatedStatement {
                            location: ast::Location::new(4, 2),
                            node: ast::Statement::FunctionDef {
                                name: String::from("method_with_default"),
                                args: ast::Parameters {
                                    args: vec![String::from("self"), String::from("arg"),],
                                    kwonlyargs: vec![],
                                    vararg: None,
                                    kwarg: None,
                                    defaults: vec![ast::Expression::String {
                                        value: "default".to_string()
                                    }],
                                    kw_defaults: vec![],
                                },
                                body: vec![ast::LocatedStatement {
                                    location: ast::Location::new(5, 3),
                                    node: ast::Statement::Pass,
                                }],
                                decorator_list: vec![],
                            }
                        }
                    ],
                    decorator_list: vec![],
                }
            })
        )
    }

    #[test]
    fn test_parse_list_comprehension() {
        let source = String::from("[x for y in z]");
        let parse_ast = parse_expression(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Expression::Comprehension {
                kind: Box::new(ast::ComprehensionKind::List {
                    element: ast::Expression::Identifier {
                        name: "x".to_string()
                    }
                }),
                generators: vec![ast::Comprehension {
                    target: ast::Expression::Identifier {
                        name: "y".to_string()
                    },
                    iter: ast::Expression::Identifier {
                        name: "z".to_string()
                    },
                    ifs: vec![],
                }],
            }
        );
    }

    #[test]
    fn test_parse_double_list_comprehension() {
        let source = String::from("[x for y, y2 in z for a in b if a < 5 if a > 10]");
        let parse_ast = parse_expression(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Expression::Comprehension {
                kind: Box::new(ast::ComprehensionKind::List {
                    element: ast::Expression::Identifier {
                        name: "x".to_string()
                    }
                }),
                generators: vec![
                    ast::Comprehension {
                        target: ast::Expression::Tuple {
                            elements: vec![
                                ast::Expression::Identifier {
                                    name: "y".to_string()
                                },
                                ast::Expression::Identifier {
                                    name: "y2".to_string()
                                },
                            ],
                        },
                        iter: ast::Expression::Identifier {
                            name: "z".to_string()
                        },
                        ifs: vec![],
                    },
                    ast::Comprehension {
                        target: ast::Expression::Identifier {
                            name: "a".to_string()
                        },
                        iter: ast::Expression::Identifier {
                            name: "b".to_string()
                        },
                        ifs: vec![
                            ast::Expression::Compare {
                                a: Box::new(ast::Expression::Identifier {
                                    name: "a".to_string()
                                }),
                                op: ast::Comparison::Less,
                                b: Box::new(ast::Expression::Number {
                                    value: ast::Number::Integer {
                                        value: BigInt::from(5)
                                    }
                                }),
                            },
                            ast::Expression::Compare {
                                a: Box::new(ast::Expression::Identifier {
                                    name: "a".to_string()
                                }),
                                op: ast::Comparison::Greater,
                                b: Box::new(ast::Expression::Number {
                                    value: ast::Number::Integer {
                                        value: BigInt::from(10)
                                    }
                                }),
                            },
                        ],
                    }
                ],
            }
        );
    }
}
