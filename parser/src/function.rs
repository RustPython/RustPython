use crate::ast;
use crate::error::{LexicalError, LexicalErrorType};

type FunctionArgument = (Option<Option<String>>, ast::Expression);

pub fn parse_args(func_args: Vec<FunctionArgument>) -> Result<ast::ArgumentList, LexicalError> {
    let mut args = vec![];
    let mut keywords = vec![];
    for (name, value) in func_args {
        match name {
            Some(n) => {
                keywords.push(ast::Keyword { name: n, value });
            }
            None => {
                // Allow starred args after keyword arguments.
                if !keywords.is_empty() && !is_starred(&value) {
                    return Err(LexicalError {
                        error: LexicalErrorType::PositionalArgumentError,
                        location: value.location.clone(),
                    });
                }

                args.push(value);
            }
        }
    }
    Ok(ast::ArgumentList { args, keywords })
}

fn is_starred(exp: &ast::Expression) -> bool {
    if let ast::ExpressionType::Starred { .. } = exp.node {
        true
    } else {
        false
    }
}
