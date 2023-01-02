use crate::{ast, error::LexicalError, lexer::LexResult, token::Tok};

#[derive(Debug, Clone)]
pub struct PegTokens {
    tokens: Vec<Tok>,
}

impl PegTokens {
    fn from(lexer: impl Iterator<Item = LexResult>) -> Result<Self, LexicalError> {
        let mut tokens = vec![];
        for tok in lexer {
            let (begin, tok, end) = tok?;
            tokens.push(tok);
        }

        Ok(Self { tokens })
    }
}

impl peg::Parse for PegTokens {
    type PositionRepr = usize;

    fn start<'input>(&'input self) -> usize {
        0
    }

    fn is_eof<'input>(&'input self, p: usize) -> bool {
        p >= self.tokens.len()
    }

    fn position_repr<'input>(&'input self, p: usize) -> Self::PositionRepr {
        p
    }
}

impl<'input> peg::ParseElem<'input> for PegTokens {
    type Element = &'input Tok;

    fn parse_elem(&'input self, pos: usize) -> peg::RuleResult<Self::Element> {
        match self.tokens.get(pos) {
            Some(tok) => peg::RuleResult::Matched(pos + 1, tok),
            None => peg::RuleResult::Failed,
        }
    }
}

impl<'input> peg::ParseSlice<'input> for PegTokens {
    type Slice = &'input [Tok];

    fn parse_slice(&'input self, p1: usize, p2: usize) -> Self::Slice {
        &self.tokens[p1..p2]
    }
}

peg::parser! { grammar python_parser() for PegTokens {
    pub rule ret() -> ast::StmtKind = [Tok::Return] [Tok::Newline] { ast::StmtKind::Return { value: None } }
}}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::make_tokenizer;

    #[test]
    fn test_return() {
        let source = "return";
        let lexer = make_tokenizer(source);
        let tokens = PegTokens::from(lexer).unwrap();
        dbg!(python_parser::ret(&tokens));
    }
}
