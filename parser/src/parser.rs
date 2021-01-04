//! Python parsing.
//!
//! Use this module to parse python code into an AST.
//! There are three ways to parse python code. You could
//! parse a whole program, a single statement, or a single
//! expression.

use std::iter;

use crate::ast;
use crate::error::ParseError;
use crate::lexer;
pub use crate::mode::Mode;
use crate::python;
use crate::token;

/*
 * Parse python code.
 * Grammar may be inspired by antlr grammar for python:
 * https://github.com/antlr/grammars-v4/tree/master/python3
 */

macro_rules! do_lalr_parsing {
    ($input: expr, $pat: ident, $tok: ident) => {{
        let lxr = lexer::make_tokenizer($input);
        let marker_token = (Default::default(), token::Tok::$tok, Default::default());
        let tokenizer = iter::once(Ok(marker_token)).chain(lxr);

        match python::TopParser::new().parse(tokenizer) {
            Err(err) => Err(ParseError::from(err)),
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

/// Parse a full python program, containing usually multiple lines.
pub fn parse_program(source: &str) -> Result<ast::Program, ParseError> {
    do_lalr_parsing!(source, Program, StartProgram)
}

/// Parse a single statement.
pub fn parse_statement(source: &str) -> Result<Vec<ast::Statement>, ParseError> {
    do_lalr_parsing!(source, Statement, StartStatement)
}

/// Parses a python expression
///
/// # Example
/// ```
/// extern crate num_bigint;
/// use num_bigint::BigInt;
/// use rustpython_parser::{parser, ast};
/// let expr = parser::parse_expression("1 + 2").unwrap();
///
/// assert_eq!(ast::Expression {
///         location: ast::Location::new(1, 3),
///         custom: (),
///         node: ast::ExpressionType::Binop {
///             a: Box::new(ast::Expression {
///                 location: ast::Location::new(1, 1),
///                 custom: (),
///                 node: ast::ExpressionType::Number {
///                     value: ast::Number::Integer { value: BigInt::from(1) }
///                 }
///             }),
///             op: ast::Operator::Add,
///             b: Box::new(ast::Expression {
///                 location: ast::Location::new(1, 5),
///                 custom: (),
///                 node: ast::ExpressionType::Number {
///                     value: ast::Number::Integer { value: BigInt::from(2) }
///                 }
///             })
///         }
///     },
///     expr);
///
/// ```
pub fn parse_expression(source: &str) -> Result<ast::Expression, ParseError> {
    do_lalr_parsing!(source, Expression, StartExpression)
}

// Parse a given source code
pub fn parse(source: &str, mode: Mode) -> Result<ast::Top, ParseError> {
    Ok(match mode {
        Mode::Program => {
            let ast = parse_program(source)?;
            ast::Top::Program(ast)
        }
        Mode::Statement => {
            let statement = parse_statement(source)?;
            ast::Top::Statement(statement)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::ast;
    use super::parse_expression;
    use super::parse_program;
    use super::parse_statement;
    use num_bigint::BigInt;

    fn mk_ident(name: &str, row: usize, col: usize) -> ast::Expression {
        ast::Expression {
            location: ast::Location::new(row, col),
            custom: (),
            node: ast::ExpressionType::Identifier {
                name: name.to_owned(),
            },
        }
    }

    fn make_int(value: i32, row: usize, col: usize) -> ast::Expression {
        ast::Expression {
            location: ast::Location::new(row, col),
            custom: (),
            node: ast::ExpressionType::Number {
                value: ast::Number::Integer {
                    value: BigInt::from(value),
                },
            },
        }
    }

    fn make_string(value: &str, row: usize, col: usize) -> ast::Expression {
        ast::Expression {
            location: ast::Location::new(row, col),
            custom: (),
            node: ast::ExpressionType::String {
                value: ast::StringGroup::Constant {
                    value: String::from(value),
                },
            },
        }
    }

    fn as_statement(expr: ast::Expression) -> ast::Statement {
        ast::Statement {
            location: expr.location,
            custom: (),
            node: ast::StatementType::Expression { expression: expr },
        }
    }

    #[test]
    fn test_parse_empty() {
        let parse_ast = parse_program("");
        assert_eq!(parse_ast, Ok(ast::Program { statements: vec![] }))
    }

    #[test]
    fn test_parse_print_hello() {
        let source = String::from("print('Hello world')");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![ast::Statement {
                    location: ast::Location::new(1, 1),
                    custom: (),
                    node: ast::StatementType::Expression {
                        expression: ast::Expression {
                            custom: (),
                            location: ast::Location::new(1, 6),
                            node: ast::ExpressionType::Call {
                                function: Box::new(mk_ident("print", 1, 1)),
                                args: vec![make_string("Hello world", 1, 8)],
                                keywords: vec![],
                            }
                        },
                    },
                },],
            }
        );
    }

    #[test]
    fn test_parse_print_2() {
        let source = String::from("print('Hello world', 2)");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![ast::Statement {
                    location: ast::Location::new(1, 1),
                    custom: (),
                    node: ast::StatementType::Expression {
                        expression: ast::Expression {
                            location: ast::Location::new(1, 6),
                            custom: (),
                            node: ast::ExpressionType::Call {
                                function: Box::new(mk_ident("print", 1, 1)),
                                args: vec![make_string("Hello world", 1, 8), make_int(2, 1, 22),],
                                keywords: vec![],
                            },
                        },
                    },
                },],
            }
        );
    }

    #[test]
    fn test_parse_kwargs() {
        let source = String::from("my_func('positional', keyword=2)");
        let parse_ast = parse_program(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Program {
                statements: vec![ast::Statement {
                    location: ast::Location::new(1, 1),
                    custom: (),
                    node: ast::StatementType::Expression {
                        expression: ast::Expression {
                            location: ast::Location::new(1, 8),
                            custom: (),
                            node: ast::ExpressionType::Call {
                                function: Box::new(mk_ident("my_func", 1, 1)),
                                args: vec![make_string("positional", 1, 10)],
                                keywords: vec![ast::Keyword {
                                    name: Some("keyword".to_owned()),
                                    value: make_int(2, 1, 31),
                                }],
                            }
                        },
                    },
                },],
            }
        );
    }

    #[test]
    fn test_parse_if_elif_else() {
        let source = String::from("if 1: 10\nelif 2: 20\nelse: 30");
        let parse_ast = parse_statement(&source).unwrap();
        assert_eq!(
            parse_ast,
            vec![ast::Statement {
                location: ast::Location::new(1, 1),
                custom: (),
                node: ast::StatementType::If {
                    test: make_int(1, 1, 4),
                    body: vec![as_statement(make_int(10, 1, 7))],
                    orelse: Some(vec![ast::Statement {
                        location: ast::Location::new(2, 1),
                        custom: (),
                        node: ast::StatementType::If {
                            test: make_int(2, 2, 6),
                            body: vec![as_statement(make_int(20, 2, 9))],
                            orelse: Some(vec![as_statement(make_int(30, 3, 7))]),
                        }
                    },]),
                }
            }]
        );
    }

    #[test]
    fn test_parse_lambda() {
        let source = String::from("lambda x, y: x * y"); // lambda(x, y): x * y");
        let parse_ast = parse_statement(&source);
        assert_eq!(
            parse_ast,
            Ok(vec![as_statement(ast::Expression {
                location: ast::Location::new(1, 1),
                custom: (),
                node: ast::ExpressionType::Lambda {
                    args: Box::new(ast::Parameters {
                        posonlyargs_count: 0,
                        args: vec![
                            ast::Parameter {
                                location: ast::Location::new(1, 8),
                                arg: String::from("x"),
                                annotation: None,
                            },
                            ast::Parameter {
                                location: ast::Location::new(1, 11),
                                arg: String::from("y"),
                                annotation: None,
                            }
                        ],
                        kwonlyargs: vec![],
                        vararg: ast::Varargs::None,
                        kwarg: ast::Varargs::None,
                        defaults: vec![],
                        kw_defaults: vec![],
                    }),
                    body: Box::new(ast::Expression {
                        location: ast::Location::new(1, 16),
                        custom: (),
                        node: ast::ExpressionType::Binop {
                            a: Box::new(mk_ident("x", 1, 14)),
                            op: ast::Operator::Mult,
                            b: Box::new(mk_ident("y", 1, 18))
                        }
                    })
                }
            })])
        )
    }

    #[test]
    fn test_parse_tuples() {
        let source = String::from("a, b = 4, 5");

        assert_eq!(
            parse_statement(&source),
            Ok(vec![ast::Statement {
                location: ast::Location::new(1, 1),
                custom: (),
                node: ast::StatementType::Assign {
                    targets: vec![ast::Expression {
                        custom: (),
                        location: ast::Location::new(1, 1),
                        node: ast::ExpressionType::Tuple {
                            elements: vec![mk_ident("a", 1, 1), mk_ident("b", 1, 4),]
                        }
                    }],
                    value: ast::Expression {
                        location: ast::Location::new(1, 8),
                        custom: (),
                        node: ast::ExpressionType::Tuple {
                            elements: vec![make_int(4, 1, 8), make_int(5, 1, 11),]
                        }
                    }
                }
            }])
        )
    }

    #[test]
    fn test_parse_class() {
        let source = String::from(
            "class Foo(A, B):\n def __init__(self):\n  pass\n def method_with_default(self, arg='default'):\n  pass",
        );
        assert_eq!(
            parse_statement(&source),
            Ok(vec![ast::Statement {
                location: ast::Location::new(1, 1),
                custom: (),
                node: ast::StatementType::ClassDef {
                    name: String::from("Foo"),
                    bases: vec![mk_ident("A", 1, 11), mk_ident("B", 1, 14)],
                    keywords: vec![],
                    body: vec![
                        ast::Statement {
                            location: ast::Location::new(2, 2),
                            custom: (),
                            node: ast::StatementType::FunctionDef {
                                is_async: false,
                                name: String::from("__init__"),
                                args: Box::new(ast::Parameters {
                                    posonlyargs_count: 0,
                                    args: vec![ast::Parameter {
                                        location: ast::Location::new(2, 15),
                                        arg: String::from("self"),
                                        annotation: None,
                                    }],
                                    kwonlyargs: vec![],
                                    vararg: ast::Varargs::None,
                                    kwarg: ast::Varargs::None,
                                    defaults: vec![],
                                    kw_defaults: vec![],
                                }),
                                body: vec![ast::Statement {
                                    location: ast::Location::new(3, 3),
                                    custom: (),
                                    node: ast::StatementType::Pass,
                                }],
                                decorator_list: vec![],
                                returns: None,
                            }
                        },
                        ast::Statement {
                            location: ast::Location::new(4, 2),
                            custom: (),
                            node: ast::StatementType::FunctionDef {
                                is_async: false,
                                name: String::from("method_with_default"),
                                args: Box::new(ast::Parameters {
                                    posonlyargs_count: 0,
                                    args: vec![
                                        ast::Parameter {
                                            location: ast::Location::new(4, 26),
                                            arg: String::from("self"),
                                            annotation: None,
                                        },
                                        ast::Parameter {
                                            location: ast::Location::new(4, 32),
                                            arg: String::from("arg"),
                                            annotation: None,
                                        }
                                    ],
                                    kwonlyargs: vec![],
                                    vararg: ast::Varargs::None,
                                    kwarg: ast::Varargs::None,
                                    defaults: vec![make_string("default", 4, 37)],
                                    kw_defaults: vec![],
                                }),
                                body: vec![ast::Statement {
                                    location: ast::Location::new(5, 3),
                                    custom: (),
                                    node: ast::StatementType::Pass,
                                }],
                                decorator_list: vec![],
                                returns: None,
                            }
                        }
                    ],
                    decorator_list: vec![],
                }
            }])
        )
    }

    #[test]
    fn test_parse_dict_comprehension() {
        let source = String::from("{x1: x2 for y in z}");
        let parse_ast = parse_expression(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Expression {
                location: ast::Location::new(1, 1),
                custom: (),
                node: ast::ExpressionType::Comprehension {
                    kind: Box::new(ast::ComprehensionKind::Dict {
                        key: mk_ident("x1", 1, 2),
                        value: mk_ident("x2", 1, 6),
                    }),
                    generators: vec![ast::Comprehension {
                        location: ast::Location::new(1, 9),
                        target: mk_ident("y", 1, 13),
                        iter: mk_ident("z", 1, 18),
                        ifs: vec![],
                        is_async: false,
                    }],
                }
            }
        );
    }

    #[test]
    fn test_parse_list_comprehension() {
        let source = String::from("[x for y in z]");
        let parse_ast = parse_expression(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Expression {
                custom: (),
                location: ast::Location::new(1, 1),
                node: ast::ExpressionType::Comprehension {
                    kind: Box::new(ast::ComprehensionKind::List {
                        element: mk_ident("x", 1, 2),
                    }),
                    generators: vec![ast::Comprehension {
                        location: ast::Location::new(1, 4),
                        target: mk_ident("y", 1, 8),
                        iter: mk_ident("z", 1, 13),
                        ifs: vec![],
                        is_async: false,
                    }],
                }
            }
        );
    }

    #[test]
    fn test_parse_double_list_comprehension() {
        let source = String::from("[x for y, y2 in z for a in b if a < 5 if a > 10]");
        let parse_ast = parse_expression(&source).unwrap();
        assert_eq!(
            parse_ast,
            ast::Expression {
                custom: (),
                location: ast::Location::new(1, 1),
                node: ast::ExpressionType::Comprehension {
                    kind: Box::new(ast::ComprehensionKind::List {
                        element: mk_ident("x", 1, 2)
                    }),
                    generators: vec![
                        ast::Comprehension {
                            location: ast::Location::new(1, 4),
                            target: ast::Expression {
                                custom: (),
                                location: ast::Location::new(1, 8),
                                node: ast::ExpressionType::Tuple {
                                    elements: vec![mk_ident("y", 1, 8), mk_ident("y2", 1, 11),],
                                }
                            },
                            iter: mk_ident("z", 1, 17),
                            ifs: vec![],
                            is_async: false,
                        },
                        ast::Comprehension {
                            location: ast::Location::new(1, 19),
                            target: mk_ident("a", 1, 23),
                            iter: mk_ident("b", 1, 28),
                            ifs: vec![
                                ast::Expression {
                                    custom: (),
                                    location: ast::Location::new(1, 35),
                                    node: ast::ExpressionType::Compare {
                                        vals: vec![mk_ident("a", 1, 33), make_int(5, 1, 37),],
                                        ops: vec![ast::Comparison::Less],
                                    }
                                },
                                ast::Expression {
                                    custom: (),
                                    location: ast::Location::new(1, 44),
                                    node: ast::ExpressionType::Compare {
                                        vals: vec![mk_ident("a", 1, 42), make_int(10, 1, 46),],
                                        ops: vec![ast::Comparison::Greater],
                                    },
                                },
                            ],
                            is_async: false,
                        }
                    ],
                }
            }
        );
    }
}
