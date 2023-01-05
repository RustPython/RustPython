use ast::{Located, Location};

use crate::{ast, error::LexicalError, lexer::LexResult, token::Tok};

#[derive(Debug, Clone)]
pub struct Parser {
    tokens: Vec<Tok>,
    locations: Vec<(Location, Location)>,
}

impl Parser {
    fn from(lexer: impl Iterator<Item = LexResult>) -> Result<Self, LexicalError> {
        let mut tokens = vec![];
        let mut locations = vec![];
        for tok in lexer {
            let (begin, tok, end) = tok?;
            tokens.push(tok);
            locations.push((begin, end));
        }

        Ok(Self { tokens, locations })
    }

    fn new_located<T>(&self, begin: usize, end: usize, node: T) -> Located<T> {
        assert!(begin < end);
        let location = self.locations[begin].0;
        let end_location = self.locations[end - 1].1;
        Located::new(location, end_location, node)
    }

    fn new_located_single<T>(&self, tok_pos: usize, node: T) -> Located<T> {
        let loc = self.locations[tok_pos];
        Located::new(loc.0, loc.1, node)
    }
}

impl peg::Parse for Parser {
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

impl<'input> peg::ParseElem<'input> for Parser {
    type Element = &'input Tok;

    fn parse_elem(&'input self, pos: usize) -> peg::RuleResult<Self::Element> {
        match self.tokens.get(pos) {
            Some(tok) => peg::RuleResult::Matched(pos + 1, tok),
            None => peg::RuleResult::Failed,
        }
    }
}

impl<'input> peg::ParseSlice<'input> for Parser {
    type Slice = &'input [Tok];

    fn parse_slice(&'input self, p1: usize, p2: usize) -> Self::Slice {
        &self.tokens[p1..p2]
    }
}

peg::parser! { grammar python_parser(zelf: &Parser) for Parser {
    use Tok::*;
    use std::option::Option::None;

    pub rule file() -> ast::Mod = a:statements() [EndOfFile] { ast::Mod::Module { body: a, type_ignores: vec![] } }
    pub rule interactive() -> ast::Mod = a:statement_newline() { ast::Mod::Interactive { body: a } }
    pub rule eval() -> ast::Mod = a:expression() [Newline]* [EndOfFile] {
        ast::Mod::Expression { body: Box::new(a) }
    }

    rule statements() -> Vec<ast::Stmt> = a:statement()+ { a.into_iter().flatten().collect() }

    rule statement() -> Vec<ast::Stmt> =
        a:compound_stmt() { vec![a] } /
        simple_stmts()

    rule statement_newline() -> Vec<ast::Stmt> =
        a:compound_stmt() [Newline] { vec![a] } /
        simple_stmts() /
        begin:position!() [Newline] {
            vec![zelf.new_located_single(begin, ast::StmtKind::Pass)]
        }
        // TODO: Error if EOF

    rule simple_stmts() -> Vec<ast::Stmt> = a:simple_stmt() ++ [Comma] [Comma]? [Newline] { a }

    rule simple_stmt() -> ast::Stmt =
        assignment() /
        begin:position!() a:star_expressions() end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::Expr { value: Box::new(a) })
        } /
        return_stmt() /
        import_stmt() /
        raise_stmt() /
        begin:position!() [Pass] {
            zelf.new_located_single(begin, ast::StmtKind::Pass)
        } /
        del_stmt() /
        yield_stmt() /
        assert_stmt() /
        begin:position!() [Break] {
            zelf.new_located_single(begin, ast::StmtKind::Break)
        } /
        begin:position!() [Continue] {
            zelf.new_located_single(begin, ast::StmtKind::Continue)
        } /
        global_stmt() /
        nonlocal_stmt()

    rule compound_stmt() -> ast::Stmt =
        &[Def | At | Async] { function_def() } /
        &[If] { if_stmt() } /
        &[Class | At] { class_def() } /
        &[With | Async] { with_stmt() } /
        &[For | Async] { for_stmt() } /
        &[Try] { try_stmt() } /
        &[While] { while_stmt() } /
        match_stmt()

    rule assignment() -> ast::Stmt =
        begin:position!() [Name { name }] [Colon] b:expression() c:([Equal] d:annotated_rhs() { d })? end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::AnnAssign {
                target: Box::new(zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })),
                annotation: Box::new(b),
                value: c.map(|x| Box::new(x)),
                simple: 1,
            })
        } /
        [Lpar] a:single_target() [Rpar] { a }

    rule annotated_rhs() -> ast::Expr = yield_expr() / star_expressions()

    rule return_stmt() -> ast::Stmt = begin:position!() [Return] a:star_expressions() end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Return { value: a })
    }

    rule raise_stmt() -> ast::Stmt =
        begin:position!() [Raise] a:expression() b:([From] z:expression() { z })? end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::Raise { exc: Some(Box::new(a)), cause: b.map(|x| Box::new(x)) })
        } /
        begin:position!() [Raise] {
            zelf.new_located_single(begin, ast::StmtKind::Raise { exc: None, cause: None })
        }
    
    rule global_stmt() -> ast::Stmt = begin:position!() [Global] a:([Name { name }] { name.clone() }) ++ [Comma] end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Global { names: a })
    }
    
    rule nonlocal_stmt() -> ast::Stmt = begin:position!() [Nonlocal] a:([Name { name }] { name.clone() }) ++ [Comma] end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Nonlocal { names: a })
    }

    rule del_stmt() -> ast::Stmt = begin:position!() [Del] a:del_targets() &[Comma | Newline] end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Delete { targets: a })
    }

    rule yield_stmt() -> ast::Stmt = begin:position!() a:yield_expr() end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Expr { value: Box::new(a) })
    }

    rule assert_stmt() -> ast::Stmt = begin:position!() [Assert] a:expression() b:([Comma] z:expression() { z })? end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Assert { test: Box::new(a), msg: b.map(|x| Box::new(x)) })
    }

    rule import_stmt() -> ast::Stmt = import_name() / import_from()

    rule import_name() -> ast::Stmt = begin:position!() [Import] a:dotted_as_names() end:position!() {
        zelf.new_located(begin, end, ast::StmtKind::Import { names: a })
    }

    rule import_from() -> ast::Stmt =
        begin:position!() [From] a:[Dot | Ellipsis]* b:dotted_name() [Import] c:import_from_targets() end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::ImportFrom { module: Some(b), names: c, level: count_dots(a) })
        } /
        begin:position!() [From] a:[Dot | Ellipsis]+ [Import] b:import_from_targets() end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::ImportFrom { module: None, names: b, level: count_dots(a) })
        }
    
    rule import_from_targets() -> Vec<ast::Alias> =
        [Lpar] a:import_from_as_names() [Comma]? [Rpar] { a } /
        a:import_from_as_names() ![Comma] { a } /
        begin:position!() [Star] { vec![zelf.new_located_single(begin, ast::AliasData { name: "*".to_owned(), asname: None })] }
    
    rule import_from_as_names() -> Vec<ast::Alias> = import_from_as_name() ++ [Comma]

    rule import_from_as_name() -> ast::Alias = begin:position!() [Name { name }] b:([As] [Name { name }] { name })? end:position!() {
        zelf.new_located(begin, end, ast::AliasData { name: name.clone(), asname: b.cloned() })
    }

    rule dotted_as_names() -> Vec<ast::Alias> = dotted_as_name() ++ [Comma]

    rule dotted_as_name() -> ast::Alias = begin:position!() a:dotted_name() b:([As] [Name { name }] { name.clone() })? end:position!() {
        zelf.new_located(begin, end, ast::AliasData { name:a, asname: b })
    }

    #[cache_left_rec]
    rule dotted_name() -> std::string::String =
        a:dotted_name() [Dot] [Name { name }] {
            format!("{}.{}", a, name)
        } /
        [Name { name }] { name.clone() }

    rule expressions() -> Vec<ast::Expr> = a:expression() ++ [Comma] [Comma]? { a }

    // rule expression() -> ast:Expr =

    rule yield_expr() -> ast::Expr = pos:position!() [Yield] [From] a:expression() {  }

    rule disjunction() -> ast::Expr = begin:position!() a:conjunction() ++ [Or] end:position!() {
        zelf.new_located(begin, end, ast::ExprKind::BoolOp { op: ast::Boolop::Or, values: a })
    }

    rule conjunction() -> ast::Expr = begin:position!() a:inversion() ++ [And] end:position!() {
        zelf.new_located(begin, end, ast::ExprKind::BoolOp { op: ast::Boolop::And, values: a })
    }

    rule inversion() -> ast::Expr =
        begin:position!() [Not] a:inversion() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::UnaryOp { op: ast::Unaryop::Not, operand: Box::new(a) })
        } /
        comparison()

    // rule comparison() -> ast::Expr

    rule eq_bitwise_or() -> (ast::Cmpop, ast::Expr) = [EqEqual] a:bitwise_or() { (ast::Cmpop::Eq, a) }
    rule noteq_bitwise_or() -> (ast::Cmpop, ast::Expr) = [NotEqual] a:bitwise_or() { (ast::Cmpop::NotEq, a) }
    rule lte_bitwise_or() -> (ast::Cmpop, ast::Expr) = [LessEqual] a:bitwise_or() { (ast::Cmpop::LtE, a) }
    rule lt_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Less] a:bitwise_or() { (ast::Cmpop::Lt, a) }
    rule gte_bitwise_or() -> (ast::Cmpop, ast::Expr) = [GreaterEqual] a:bitwise_or() { (ast::Cmpop::GtE, a) }
    rule gt_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Greater] a:bitwise_or() { (ast::Cmpop::Gt, a) }
    rule notin_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Not] [In] a:bitwise_or() { (ast::Cmpop::NotIn, a) }
    rule in_bitwise_or() -> (ast::Cmpop, ast::Expr) = [In] a:bitwise_or() { (ast::Cmpop::In, a) }
    rule isnot_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Is] [Not] a:bitwise_or() { (ast::Cmpop::IsNot, a) }
    rule is_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Is] a:bitwise_or() { (ast::Cmpop::Is, a) }

    #[cache_left_rec]
    rule bitwise_or() -> ast::Expr =
        begin:position!() a:bitwise_or() [Or] b:bitwise_xor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitOr, right: Box::new(b) })
        } /
        bitwise_xor()

    #[cache_left_rec]
    rule bitwise_xor() -> ast::Expr =
        begin:position!() a:bitwise_xor() [CircumFlex] b:bitwise_and() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitXor, right: Box::new(b) })
        } /
        bitwise_and()

    #[cache_left_rec]
    rule bitwise_and() -> ast::Expr =
        begin:position!() a:bitwise_and() [Amper] b:shift_expr() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitAnd, right: Box::new(b) })
        } /
        shift_expr()

    #[cache_left_rec]
    rule shift_expr() -> ast::Expr =
        begin:position!() a:shift_expr() [LeftShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::LShift, right: Box::new(b) })
        } /
        begin:position!() a:shift_expr() [RightShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::RShift, right: Box::new(b) })
        } /
        sum()

    #[cache_left_rec]
    rule sum() -> ast::Expr =
        begin:position!() a:sum() [Plus] b:term() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Add, right: Box::new(b) })
        } /
        begin:position!() a:sum() [Minus] b:term() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Sub, right: Box::new(b) })
        } /
        term()

    #[cache_left_rec]
    rule term() -> ast::Expr =
        begin:position!() a:term() [Star] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mult, right: Box::new(b) })
        } /
        begin:position!() a:term() [Slash] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Div, right: Box::new(b) })
        } /
        begin:position!() a:term() [DoubleSlash] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::FloorDiv, right: Box::new(b) })
        } /
        begin:position!() a:term() [Percent] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mod, right: Box::new(b) })
        } /
        begin:position!() a:term() [At] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::MatMult, right: Box::new(b) })
        } /
        factor()

    // rule bitwise() -> ast::Expr = precedence!{
    //     begin:position!() a:@ [BitOr] b:@ { zelf.new_located() }
    // }

    // rule compound_stmt() -> ast::StmtKind = [Def]

    rule star_targets() -> Vec<ast::Expr> =
        a:star_target() ![Comma] { vec![a] } /
        a:star_target() ++ [Comma] [Comma]? { a }

    rule star_targets_list() -> Vec<ast::Expr> = a:star_target() ++ [Comma] [Comma]? { a }

    rule star_targets_tuple() -> Vec<ast::Expr> =
        a:star_target() **<2,> [Comma] [Comma]? { a } /
        a:star_target() [Comma] { vec![a] }

    rule star_target() -> ast::Expr =
        begin:position!() [Star] ![Star] a:star_target() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Starred { value: Box::new(a), ctx: ast::ExprContext::Store })
        } /
        target_with_star_atom()

    rule target_with_star_atom() -> ast::Expr =
        single_subscript_attribute_target() /
        star_atom()

    rule star_atom() -> ast::Expr =
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })
        } /
        [Lpar] a:target_with_star_atom() [Rpar] { a } /
        begin:position!() [Lpar] a:star_targets_tuple() [Rpar] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Tuple { elts: a, ctx: ast::ExprContext::Store })
        } /
        begin:position!() [Lsqb] a:star_targets_list() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::List { elts: a, ctx: ast::ExprContext::Store })
        }

    rule single_target() -> ast::Expr =
        single_subscript_attribute_target() /
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })
        } /
        [Lpar] a:single_target() [Rpar] { a }

    rule single_subscript_attribute_target() -> ast::Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Store })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Store })
        }

    #[cache_left_rec]
    rule t_primary() -> ast::Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Load })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Load })
        }
        // TODO:

    rule t_lookahead() = [Lpar] / [Lsqb] / [Dot]

    rule del_targets() -> Vec<ast::Expr> = a:del_target() ++ [Comma] [Comma]? { a }

    rule del_target() -> ast::Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Del })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Del })
        } /
        del_t_atom()

    rule del_t_atom() -> ast::Expr =
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Del })
        } /
        begin:position!() [Lpar] a:del_target() [Rpar] end:position!() { a } /
        begin:position!() [Lpar] a:del_targets() [Rpar] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Tuple { elts: a, ctx: ast::ExprContext::Del })
        } /
        begin:position!() [Lsqb] a:del_targets() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::List { elts: a, ctx: ast::ExprContext::Del })
        }

}}

fn count_dots(toks: Vec<&Tok>) -> Option<usize> {
    if toks.is_empty() {
        return None;
    }

    let mut count = 0;
    for tok in toks {
        count += match tok {
            Tok::Dot => 1,
            Tok::Ellipsis => 3,
            _ => unreachable!(),
        };
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::make_tokenizer;

    #[test]
    fn test_return() {
        let source = "return";
        let lexer = make_tokenizer(source);
        let parser = Parser::from(lexer).unwrap();
        dbg!(&parser);
        dbg!(python_parser::interactive(&parser, &parser));
    }
}
