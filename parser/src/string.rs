use crate::ast::{Constant, Expr, ExprKind, Location};
use crate::error::LexicalErrorType::FStringError;
use crate::error::{LexicalError, LexicalErrorType};
use crate::fstring::parse_located_fstring;
use crate::token::StringKind;
use itertools::Itertools;

pub fn parse_implicit_concatenation(
    values: Vec<(Location, (String, StringKind))>,
) -> Result<Expr, LexicalError> {
    // Preserve the initial location and kind.
    let initial_location = values[0].0;
    let initial_kind = (values[0].1.1 == StringKind::U).then(|| "u".to_owned());

    // Determine whether the list of values contains any f-strings. (If not, we can return a
    // single Constant at the end, rather than a JoinedStr.)
    let has_fstring = values
        .iter()
        .any(|(_, (_, string_kind))| *string_kind == StringKind::F);

    // Collect all the sub-expressions: any Constants, along with any (flattened) f-strings.
    let values: Vec<Expr> = values
        .into_iter()
        .map(|(location, (string, string_kind))| match string_kind {
            StringKind::F => {
                if let ExprKind::JoinedStr { values } = parse_located_fstring(&string, location)
                    .map_err(|e| LexicalError {
                        location,
                        error: FStringError(e.error),
                    })?
                    .node
                {
                    Ok(values)
                } else {
                    panic!("parse_located_fstring returned a non-JoinedStr.")
                }
            }
            _ => Ok(vec![Expr::new(
                location,
                ExprKind::Constant {
                    value: Constant::Str(string),
                    kind: (string_kind == StringKind::U).then(|| "u".to_owned()),
                },
            )]),
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
                        initial_location,
                        ExprKind::Constant {
                            value: Constant::Str(current.join("")),
                            kind: initial_kind.clone(),
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
                    panic!("Unexpected non-string constant.");
                }
            }
            _ => {
                return Err(LexicalError {
                    location: value.location,
                    error: LexicalErrorType::OtherError(
                        "Unexpected expression kind in string concatenation.".to_string(),
                    ),
                });
            }
        }
    }

    if !current.is_empty() {
        deduped.push(Expr::new(
            initial_location,
            ExprKind::Constant {
                value: Constant::Str(current.join("")),
                kind: initial_kind,
            },
        ));
        current.clear();
    }

    Ok(if has_fstring {
        Expr::new(initial_location, ExprKind::JoinedStr { values: deduped })
    } else {
        deduped
            .into_iter()
            .exactly_one()
            .expect("String must be concatenated to a single element.")
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
    fn test_parse_u_f_string_concat_1() {
        let source = String::from("u'Hello ' f'world'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }

    #[test]
    fn test_parse_u_f_string_concat_2() {
        let source = String::from("u'Hello ' f'world' '!'");
        let parse_ast = parse_program(&source).unwrap();
        insta::assert_debug_snapshot!(parse_ast);
    }
}
