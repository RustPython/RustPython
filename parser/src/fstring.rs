use std::iter;
use std::mem;
use std::str;

use crate::ast::{ConversionFlag, StringGroup};
use crate::error::{FStringError, FStringErrorType};
use crate::location::Location;
use crate::parser::parse_expression;

use self::FStringErrorType::*;
use self::StringGroup::*;

struct FStringParser<'a> {
    chars: iter::Peekable<str::Chars<'a>>,
}

impl<'a> FStringParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().peekable(),
        }
    }

    fn parse_formatted_value(&mut self) -> Result<StringGroup, FStringErrorType> {
        let mut expression = String::new();
        let mut spec = None;
        let mut delims = Vec::new();
        let mut conversion = None;

        while let Some(ch) = self.chars.next() {
            match ch {
                '!' if delims.is_empty() => {
                    let x = self.chars.next();
                    if let Some('=') = x {
                        expression.push(ch);
                        expression.push('=');
                    } else {
                        if expression.is_empty() {
                            return Err(EmptyExpression);
                        }

                        conversion = Some(match x {
                            Some('s') => ConversionFlag::Str,
                            Some('a') => ConversionFlag::Ascii,
                            Some('r') => ConversionFlag::Repr,
                            Some(_) => {
                                return Err(InvalidConversionFlag);
                            }
                            None => {
                                break;
                            }
                        });

                        match self.chars.peek() {
                            Some('}') => {
                                continue;
                            }
                            Some(_) => {
                                return Err(ExpectedRbrace);
                            }
                            None => {
                                break;
                            }
                        }
                    }
                }
                ':' if delims.is_empty() => {
                    let mut nested = false;
                    let mut in_nested = false;
                    let mut spec_expression = String::new();
                    while let Some(&next) = self.chars.peek() {
                        match next {
                            '{' => {
                                if in_nested {
                                    return Err(ExpressionNestedTooDeeply);
                                }
                                in_nested = true;
                                nested = true;
                                self.chars.next();
                                continue;
                            }
                            '}' => {
                                if in_nested {
                                    in_nested = false;
                                    self.chars.next();
                                }
                                break;
                            }
                            _ => (),
                        }
                        spec_expression.push(next);
                        self.chars.next();
                    }
                    if in_nested {
                        return Err(UnclosedLbrace);
                    }
                    if nested {
                        spec = Some(Box::new(FormattedValue {
                            value: Box::new(
                                parse_expression(spec_expression.trim())
                                    .map_err(|e| InvalidExpression(Box::new(e.error)))?,
                            ),
                            conversion: None,
                            spec: None,
                        }))
                    } else {
                        spec = Some(Box::new(Constant {
                            value: spec_expression.to_owned(),
                        }))
                    }
                }
                '(' | '{' | '[' => {
                    expression.push(ch);
                    delims.push(ch);
                }
                ')' => {
                    if delims.pop() != Some('(') {
                        return Err(MismatchedDelimiter);
                    }
                    expression.push(ch);
                }
                ']' => {
                    if delims.pop() != Some('[') {
                        return Err(MismatchedDelimiter);
                    }
                    expression.push(ch);
                }
                '}' if !delims.is_empty() => {
                    if delims.pop() != Some('{') {
                        return Err(MismatchedDelimiter);
                    }
                    expression.push(ch);
                }
                '}' => {
                    if expression.is_empty() {
                        return Err(EmptyExpression);
                    }
                    return Ok(FormattedValue {
                        value: Box::new(
                            parse_expression(expression.trim())
                                .map_err(|e| InvalidExpression(Box::new(e.error)))?,
                        ),
                        conversion,
                        spec,
                    });
                }
                '"' | '\'' => {
                    expression.push(ch);
                    while let Some(next) = self.chars.next() {
                        expression.push(next);
                        if next == ch {
                            break;
                        }
                    }
                }
                _ => {
                    expression.push(ch);
                }
            }
        }

        Err(UnclosedLbrace)
    }

    fn parse(mut self) -> Result<StringGroup, FStringErrorType> {
        let mut content = String::new();
        let mut values = vec![];

        while let Some(ch) = self.chars.next() {
            match ch {
                '{' => {
                    if let Some('{') = self.chars.peek() {
                        self.chars.next();
                        content.push('{');
                    } else {
                        if !content.is_empty() {
                            values.push(Constant {
                                value: mem::replace(&mut content, String::new()),
                            });
                        }

                        values.push(self.parse_formatted_value()?);
                    }
                }
                '}' => {
                    if let Some('}') = self.chars.peek() {
                        self.chars.next();
                        content.push('}');
                    } else {
                        return Err(UnopenedRbrace);
                    }
                }
                _ => {
                    content.push(ch);
                }
            }
        }

        if !content.is_empty() {
            values.push(Constant { value: content })
        }

        Ok(match values.len() {
            0 => Constant {
                value: String::new(),
            },
            1 => values.into_iter().next().unwrap(),
            _ => Joined { values },
        })
    }
}

/// Parse an f-string into a string group.
fn parse_fstring(source: &str) -> Result<StringGroup, FStringErrorType> {
    FStringParser::new(source).parse()
}

/// Parse an fstring from a string, located at a certain position in the sourcecode.
/// In case of errors, we will get the location and the error returned.
pub fn parse_located_fstring(
    source: &str,
    location: Location,
) -> Result<StringGroup, FStringError> {
    parse_fstring(source).map_err(|error| FStringError { error, location })
}

#[cfg(test)]
mod tests {
    use crate::ast;

    use super::*;

    fn mk_ident(name: &str, row: usize, col: usize) -> ast::Expression {
        ast::Expression {
            location: ast::Location::new(row, col),
            node: ast::ExpressionType::Identifier {
                name: name.to_owned(),
            },
        }
    }

    #[test]
    fn test_parse_fstring() {
        let source = String::from("{a}{ b }{{foo}}");
        let parse_ast = parse_fstring(&source).unwrap();

        assert_eq!(
            parse_ast,
            Joined {
                values: vec![
                    FormattedValue {
                        value: Box::new(mk_ident("a", 1, 1)),
                        conversion: None,
                        spec: None,
                    },
                    FormattedValue {
                        value: Box::new(mk_ident("b", 1, 1)),
                        conversion: None,
                        spec: None,
                    },
                    Constant {
                        value: "{foo}".to_owned()
                    }
                ]
            }
        );
    }

    #[test]
    fn test_parse_fstring_nested_spec() {
        let source = String::from("{foo:{spec}}");
        let parse_ast = parse_fstring(&source).unwrap();

        assert_eq!(
            parse_ast,
            FormattedValue {
                value: Box::new(mk_ident("foo", 1, 1)),
                conversion: None,
                spec: Some(Box::new(FormattedValue {
                    value: Box::new(mk_ident("spec", 1, 1)),
                    conversion: None,
                    spec: None,
                })),
            }
        );
    }

    #[test]
    fn test_parse_fstring_not_nested_spec() {
        let source = String::from("{foo:spec}");
        let parse_ast = parse_fstring(&source).unwrap();

        assert_eq!(
            parse_ast,
            FormattedValue {
                value: Box::new(mk_ident("foo", 1, 1)),
                conversion: None,
                spec: Some(Box::new(Constant {
                    value: "spec".to_owned(),
                })),
            }
        );
    }

    #[test]
    fn test_parse_empty_fstring() {
        assert_eq!(
            parse_fstring(""),
            Ok(Constant {
                value: String::new(),
            }),
        );
    }

    #[test]
    fn test_parse_invalid_fstring() {
        assert_eq!(parse_fstring("{"), Err(UnclosedLbrace));
        assert_eq!(parse_fstring("}"), Err(UnopenedRbrace));
        assert_eq!(parse_fstring("{5!a"), Err(UnclosedLbrace));
        assert_eq!(parse_fstring("{5!a1}"), Err(ExpectedRbrace));
        assert_eq!(parse_fstring("abc{!a 'cat'}"), Err(EmptyExpression));
        assert_eq!(parse_fstring("{!x}"), Err(EmptyExpression));

        assert_eq!(parse_fstring("{a:{a:{b}}"), Err(ExpressionNestedTooDeeply));
        assert_eq!(parse_fstring("{a:b}}"), Err(UnopenedRbrace));
        assert_eq!(parse_fstring("{a:{b}"), Err(UnclosedLbrace));

        // TODO: check for InvalidExpression enum?
        assert!(parse_fstring("{class}").is_err());
    }

    #[test]
    fn test_parse_fstring_not_equals() {
        let source = String::from("{1 != 2}");
        let parse_ast = parse_fstring(&source);
        assert_ne!(parse_ast, Err(InvalidConversionFlag));
    }
}
