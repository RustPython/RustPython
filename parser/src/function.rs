use ahash::RandomState;
use std::collections::HashSet;

use crate::ast;
use crate::error::{LexicalError, LexicalErrorType};

type ParameterDefs = (Vec<ast::Parameter>, Vec<ast::Expression>);
type ParameterDef = (ast::Parameter, Option<ast::Expression>);

pub fn parse_params(
    params: (Vec<ParameterDef>, Vec<ParameterDef>),
) -> Result<ParameterDefs, LexicalError> {
    let mut names = vec![];
    let mut defaults = vec![];

    let mut try_default = |name: &ast::Parameter, default| {
        if let Some(default) = default {
            defaults.push(default);
        } else if !defaults.is_empty() {
            // Once we have started with defaults, all remaining arguments must
            // have defaults
            return Err(LexicalError {
                error: LexicalErrorType::DefaultArgumentError,
                location: name.location,
            });
        }
        Ok(())
    };

    for (name, default) in params.0 {
        try_default(&name, default)?;
        names.push(name);
    }

    for (name, default) in params.1 {
        try_default(&name, default)?;
        names.push(name);
    }

    Ok((names, defaults))
}

type FunctionArgument = (Option<Option<String>>, ast::Expression);

pub fn parse_args(func_args: Vec<FunctionArgument>) -> Result<ast::ArgumentList, LexicalError> {
    let mut args = vec![];
    let mut keywords = vec![];

    let mut keyword_names = HashSet::with_capacity_and_hasher(func_args.len(), RandomState::new());
    for (name, value) in func_args {
        match name {
            Some(n) => {
                if let Some(keyword_name) = n.clone() {
                    if keyword_names.contains(&keyword_name) {
                        return Err(LexicalError {
                            error: LexicalErrorType::DuplicateKeywordArgumentError,
                            location: value.location,
                        });
                    }

                    keyword_names.insert(keyword_name.clone());
                }

                keywords.push(ast::Keyword { name: n, value });
            }
            None => {
                // Allow starred args after keyword arguments.
                if !keywords.is_empty() && !is_starred(&value) {
                    return Err(LexicalError {
                        error: LexicalErrorType::PositionalArgumentError,
                        location: value.location,
                    });
                }

                args.push(value);
            }
        }
    }
    Ok(ast::ArgumentList { args, keywords })
}

fn is_starred(exp: &ast::Expression) -> bool {
    matches!(exp.node, ast::ExpressionType::Starred { .. })
}
