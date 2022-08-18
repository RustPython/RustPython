use crate::ast::{Constant, Expr, ExprKind};
use crate::error::{LexicalError, LexicalErrorType};

pub fn parse_implicit_concatenation(values: Vec<Expr>) -> Result<Expr, LexicalError> {
    Ok(if values.len() > 1 {
        // As in CPython, use the kind of the first Expression.
        let kind = if let ExprKind::Constant { kind, .. } = &values[0].node {
            kind.clone()
        } else {
            None
        };

        // Preserve the initial location.
        let location = values[0].location;

        if values
            .iter()
            .all(|value| matches!(value.node, ExprKind::Constant { .. }))
        {
            // If every expression is a Constant, return a single Constant by concatenating the
            // underlying strings.
            Expr::new(
                location,
                ExprKind::Constant {
                    value: Constant::Str(
                        values
                            .into_iter()
                            .map(|value| {
                                if let ExprKind::Constant {
                                    value: Constant::Str(value),
                                    ..
                                } = value.node
                                {
                                    Ok(value)
                                } else {
                                    Err(LexicalError {
                                        location,
                                        error: LexicalErrorType::OtherError(
                                            "Unexpected non-string constant.".to_string(),
                                        ),
                                    })
                                }
                            })
                            .collect::<Result<Vec<String>, LexicalError>>()?
                            .join(""),
                    ),
                    kind,
                },
            )
        } else {
            // Otherwise, we have at least one JoinedStr, so return a single JointedStr with as few
            // Constants as possible by merging adjacent Constants.
            let values: Vec<Expr> = values
                .into_iter()
                .map(|value| match value.node {
                    ExprKind::JoinedStr { values } => Ok(values),
                    ExprKind::Constant { .. } => Ok(vec![value]),
                    _ => Err(LexicalError {
                        location,
                        error: LexicalErrorType::OtherError(
                            "Unexpected expression kind in string concatenation.".to_string(),
                        ),
                    }),
                })
                .flat_map(|result| match result {
                    Ok(values) => values.into_iter().map(Ok).collect(),
                    Err(e) => vec![Err(e)],
                })
                .collect::<Result<Vec<Expr>, LexicalError>>()?;

            // De-duplicate adjacent constants.
            let mut deduped: Vec<Expr> = vec![];
            let mut current: Vec<String> = vec![];
            for value in values {
                match value.node {
                    ExprKind::FormattedValue { .. } => {
                        if !current.is_empty() {
                            deduped.push(Expr::new(
                                location,
                                ExprKind::Constant {
                                    value: Constant::Str(current.join("")),
                                    kind: kind.clone(),
                                },
                            ));
                            current.clear();
                        }
                        deduped.push(value)
                    }
                    ExprKind::Constant { value, .. } => {
                        if let Constant::Str(value) = value {
                            current.push(value);
                        } else {
                            return Err(LexicalError {
                                location,
                                error: LexicalErrorType::OtherError(
                                    "Unexpected non-string constant.".to_string(),
                                ),
                            });
                        }
                    }
                    _ => {
                        return Err(LexicalError {
                            location,
                            error: LexicalErrorType::OtherError(
                                "Unexpected expression kind in string concatenation.".to_string(),
                            ),
                        });
                    }
                }
            }

            if !current.is_empty() {
                deduped.push(Expr::new(
                    location,
                    ExprKind::Constant {
                        value: Constant::Str(current.join("")),
                        kind,
                    },
                ));
                current.clear();
            }

            Expr::new(location, ExprKind::JoinedStr { values: deduped })
        }
    } else {
        values.into_iter().next().unwrap()
    })
}

#[cfg(test)]
mod tests {
    use crate::parser::parse_program;

    #[test]
    fn test_parse_string_concat() {
        let source = String::from("'Hello ' 'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_u_string_concat_1() {
        let source = String::from("'Hello ' u'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_u_string_concat_2() {
        let source = String::from("u'Hello ' 'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_f_string_concat_1() {
        let source = String::from("'Hello ' f'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_f_string_concat_2() {
        let source = String::from("'Hello ' f'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_f_string_concat_3() {
        let source = String::from("'Hello ' f'world{\"!\"}'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_u_f_string_concat_2() {
        let source = String::from("u'Hello ' f'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_u_f_string_concat_3() {
        let source = String::from("u'Hello ' f'world' '!'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }
}
