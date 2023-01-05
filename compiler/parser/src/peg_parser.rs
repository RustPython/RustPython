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
    pub rule file() -> ast::Mod = a:statements() { ast::Mod::Module { body: a, type_ignores: vec![] } }
    pub rule interactive() -> ast::Mod = a:statement() { ast::Mod::Interactive { body: a } }
    // pub rule ret() -> ast::StmtKind = [Tok::Return] [Tok::Newline] { ast::StmtKind::Return { value: None } }
    rule statements() -> Vec<ast::Stmt> = a:(statement())+ { a.into_iter().flatten().collect() }
    rule statement() -> Vec<ast::Stmt> = simple_stmts()//compound_stmt() / simple_stmts()

    rule simple_stmts() -> Vec<ast::Stmt> = a:simple_stmt() ++ [Tok::Newline] [Tok::Newline]? { a }
    rule simple_stmt() -> ast::Stmt =
        pos:position!() [Tok::Return] { zelf.new_located_single(pos, ast::StmtKind::Return { value: None }) }

    rule assignment() -> ast::Stmt =
        begin:position!() [Tok::Name { name }] [Tok::Colon] b:expression() c:([Tok::Equal] d:annotated_rhs() { d })? end:position!() {
            zelf.new_located(begin, end, ast::StmtKind::AnnAssign {
                target: Box::new(zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })),
                annotation: Box::new(b),
                value: c.map(|x| Box::new(x)),
                simple: 1,
            })
        } /
        [Tok::Lpar] a:single_target() [Tok::Rpar] { a }


    rule annotated_rhs() -> ast::Expr = yield_expr() / star_expressions()

    rule expressions() -> Vec<ast::Expr> = a:expression() ++ [Tok::Comma] [Tok::Comma]? { a }

    // rule expression() -> ast:Expr =

    rule yield_expr() -> ast::Expr = pos:position!() [Tok::Yield] [Tok::From] a:expression() {  }

    rule disjunction() -> ast::Expr = begin:position!() a:conjunction() ++ [Tok::Or] end:position!() {
        zelf.new_located(begin, end, ast::ExprKind::BoolOp { op: ast::Boolop::Or, values: a })
    }

    rule conjunction() -> ast::Expr = begin:position!() a:inversion() ++ [Tok::And] end:position!() {
        zelf.new_located(begin, end, ast::ExprKind::BoolOp { op: ast::Boolop::And, values: a })
    }

    rule inversion() -> ast::Expr =
        begin:position!() [Tok::Not] a:inversion() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::UnaryOp { op: ast::Unaryop::Not, operand: Box::new(a) })
        } /
        comparison()

    // rule comparison() -> ast::Expr

    rule eq_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::EqEqual] a:bitwise_or() { (ast::Cmpop::Eq, a) }
    rule noteq_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::NotEqual] a:bitwise_or() { (ast::Cmpop::NotEq, a) }
    rule lte_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::LessEqual] a:bitwise_or() { (ast::Cmpop::LtE, a) }
    rule lt_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::Less] a:bitwise_or() { (ast::Cmpop::Lt, a) }
    rule gte_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::GreaterEqual] a:bitwise_or() { (ast::Cmpop::GtE, a) }
    rule gt_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::Greater] a:bitwise_or() { (ast::Cmpop::Gt, a) }
    rule notin_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::Not] [Tok::In] a:bitwise_or() { (ast::Cmpop::NotIn, a) }
    rule in_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::In] a:bitwise_or() { (ast::Cmpop::In, a) }
    rule isnot_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::Is] [Tok::Not] a:bitwise_or() { (ast::Cmpop::IsNot, a) }
    rule is_bitwise_or() -> (ast::Cmpop, ast::Expr) = [Tok::Is] a:bitwise_or() { (ast::Cmpop::Is, a) }

    #[cache_left_rec]
    rule bitwise_or() -> ast::Expr =
        begin:position!() a:bitwise_or() [Tok::Or] b:bitwise_xor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitOr, right: Box::new(b) })
        } /
        bitwise_xor()

    #[cache_left_rec]
    rule bitwise_xor() -> ast::Expr =
        begin:position!() a:bitwise_xor() [Tok::CircumFlex] b:bitwise_and() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitXor, right: Box::new(b) })
        } /
        bitwise_and()

    #[cache_left_rec]
    rule bitwise_and() -> ast::Expr =
        begin:position!() a:bitwise_and() [Tok::Amper] b:shift_expr() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitAnd, right: Box::new(b) })
        } /
        shift_expr()

    #[cache_left_rec]
    rule shift_expr() -> ast::Expr =
        begin:position!() a:shift_expr() [Tok::LeftShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::LShift, right: Box::new(b) })
        } /
        begin:position!() a:shift_expr() [Tok::RightShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::RShift, right: Box::new(b) })
        } /
        sum()

    #[cache_left_rec]
    rule sum() -> ast::Expr =
        begin:position!() a:sum() [Tok::Plus] b:term() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Add, right: Box::new(b) })
        } /
        begin:position!() a:sum() [Tok::Minus] b:term() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Sub, right: Box::new(b) })
        } /
        term()

    #[cache_left_rec]
    rule term() -> ast::Expr =
        begin:position!() a:term() [Tok::Star] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mult, right: Box::new(b) })
        } /
        begin:position!() a:term() [Tok::Slash] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Div, right: Box::new(b) })
        } /
        begin:position!() a:term() [Tok::DoubleSlash] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::FloorDiv, right: Box::new(b) })
        } /
        begin:position!() a:term() [Tok::Percent] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mod, right: Box::new(b) })
        } /
        begin:position!() a:term() [Tok::At] b:factor() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::BinOp { left: Box::new(a), op: ast::Operator::MatMult, right: Box::new(b) })
        } /
        factor()

    // rule bitwise() -> ast::Expr = precedence!{
    //     begin:position!() a:@ [Tok::BitOr] b:@ { zelf.new_located() }
    // }

    // rule compound_stmt() -> ast::StmtKind = [Tok::Def]

    rule star_targets() -> Vec<ast::Expr> =
        a:star_target() ![Tok::Comma] { vec![a] } /
        a:star_target() ++ [Tok::Comma] [Tok::Comma]? { a }
    
    rule star_targets_list() -> Vec<ast::Expr> = a:star_target() ++ [Tok::Comma] [Tok::Comma]? { a }

    rule star_targets_tuple() -> Vec<ast::Expr> =
        a:star_target() **<2,> [Tok::Comma] [Tok::Comma]? { a } /
        a:star_target() [Tok::Comma] { vec![a] }

    rule star_target() -> ast::Expr =
        begin:position!() [Tok::Star] ![Tok::Star] a:star_target() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Starred { value: Box::new(a), ctx: ast::ExprContext::Store })
        } /
        target_with_star_atom()
    
    rule target_with_star_atom() -> ast::Expr =
        single_subscript_attribute_target() /
        star_atom()
    
    rule star_atom() -> ast::Expr =
        begin:position!() [Tok::Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })
        } /
        [Tok::Lpar] a:target_with_star_atom() [Tok::Rpar] { a } /
        begin:position!() [Tok::Lpar] a:star_targets_tuple() [Tok::Rpar] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Tuple { elts: a, ctx: ast::ExprContext::Store })
        } /
        begin:position!() [Tok::Lsqb] a:star_targets_list() [Tok::Rsqb] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::List { elts: a, ctx: ast::ExprContext::Store })
        }
    
    rule single_target() -> ast::Expr =
        single_subscript_attribute_target() /
        begin:position!() [Tok::Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Store })
        } /
        [Tok::Lpar] a:single_target() [Tok::Rpar] { a }
    
    rule single_subscript_attribute_target() -> ast::Expr =
        begin:position!() a:t_primary() [Tok::Dot] [Tok::Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Store })
        } /
        begin:position!() a:t_primary() [Tok::Lsqb] b:slices() [Tok::Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Store })
        }
    
    #[cache_left_rec]
    rule t_primary() -> ast::Expr =
        begin:position!() a:t_primary() [Tok::Dot] [Tok::Name { name }] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Load })
        } /
        begin:position!() a:t_primary() [Tok::Lsqb] b:slices() [Tok::Rsqb] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Load })
        }
        // TODO:
    
    rule t_lookahead() = [Tok::Lpar] / [Tok::Lsqb] / [Tok::Dot]

    rule del_targets() -> Vec<ast::Expr> = a:del_target() ++ [Tok::Comma] [Tok::Comma]? { a }

    rule del_target() -> ast::Expr = 
        begin:position!() a:t_primary() [Tok::Dot] [Tok::Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ast::ExprContext::Del })
        } /
        begin:position!() a:t_primary() [Tok::Lsqb] b:slices() [Tok::Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ast::ExprContext::Del })
        } /
        del_t_atom()
    
    rule del_t_atom() -> ast::Expr =
        begin:position!() [Tok::Name { name }] {
            zelf.new_located_single(begin, ast::ExprKind::Name { id: name.clone(), ctx: ast::ExprContext::Del })
        } /
        begin:position!() [Tok::Lpar] a:del_target() [Tok::Rpar] end:position!() { a } /
        begin:position!() [Tok::Lpar] a:del_targets() [Tok::Rpar] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::Tuple { elts: a, ctx: ast::ExprContext::Del })
        } /
        begin:position!() [Tok::Lsqb] a:del_targets() [Tok::Rsqb] end:position!() {
            zelf.new_located(begin, end, ast::ExprKind::List { elts: a, ctx: ast::ExprContext::Del })
        }

}}

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
