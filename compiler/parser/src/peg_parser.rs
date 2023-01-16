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
    use crate::token::StringKind;
    use ast::{Expr, Stmt, ExprKind, StmtKind, ExprContext, Withitem, Cmpop, Keyword, KeywordData, Comprehension};
    use std::option::Option::{Some, None};
    use std::string::String;

    pub rule file() -> ast::Mod = a:statements() [EndOfFile] { ast::Mod::Module { body: a, type_ignores: vec![] } }
    pub rule interactive() -> ast::Mod = a:statement_newline() { ast::Mod::Interactive { body: a } }
    pub rule eval() -> ast::Mod = a:expression() [Newline]* [EndOfFile] {
        ast::Mod::Expression { body: Box::new(a) }
    }
    // func_type
    // fstring

    rule statements() -> Vec<Stmt> = a:statement()+ { a.into_iter().flatten().collect() }

    rule statement() -> Vec<Stmt> =
        a:compound_stmt() { vec![a] } /
        simple_stmts()

    rule statement_newline() -> Vec<Stmt> =
        a:compound_stmt() [Newline] { vec![a] } /
        simple_stmts() /
        begin:position!() [Newline] {
            vec![zelf.new_located_single(begin, StmtKind::Pass)]
        }
        // TODO: Error if EOF

    rule simple_stmts() -> Vec<Stmt> = a:simple_stmt() ++ [Comma] [Comma]? [Newline] { a }

    rule simple_stmt() -> Stmt =
        assignment() /
        begin:position!() a:star_expressions() end:position!() {
            zelf.new_located(begin, end, StmtKind::Expr { value: Box::new(a) })
        } /
        return_stmt() /
        import_stmt() /
        raise_stmt() /
        begin:position!() [Pass] {
            zelf.new_located_single(begin, StmtKind::Pass)
        } /
        del_stmt() /
        yield_stmt() /
        assert_stmt() /
        begin:position!() [Break] {
            zelf.new_located_single(begin, StmtKind::Break)
        } /
        begin:position!() [Continue] {
            zelf.new_located_single(begin, StmtKind::Continue)
        } /
        global_stmt() /
        nonlocal_stmt()

    rule compound_stmt() -> Stmt =
        // function_def() /
        if_stmt() /
        // class_def() /
        with_stmt() /
        for_stmt() /
        // try_stmt() /
        while_stmt()
        // match_stmt()

    rule assignment() -> Stmt =
        begin:position!() [Name { name }] [Colon] b:expression() c:([Equal] d:annotated_rhs() { d })? end:position!() {
            zelf.new_located(begin, end, StmtKind::AnnAssign {
                target: Box::new(zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Store })),
                annotation: Box::new(b),
                value: c.map(|x| Box::new(x)),
                simple: 1,
            })
        } /
        begin:position!()
        a:([Lpar] z:single_target() [Rpar] {z} / single_subscript_attribute_target())
        [Colon] b:expression() c:([Equal] z:annotated_rhs() {z})?
        end:position!() {
            zelf.new_located(begin, end, StmtKind::AnnAssign { target: Box::new(a), annotation: Box::new(b), value: option_box(c), simple: 0 })
        }
        // TODO: assign augassign

    rule annotated_rhs() -> Expr = yield_expr() / star_expressions()

    rule return_stmt() -> Stmt = begin:position!() [Return] a:star_expressions()? end:position!() {
        zelf.new_located(begin, end, StmtKind::Return { value: option_box(a) })
    }

    rule raise_stmt() -> Stmt =
        begin:position!() [Raise] a:expression() b:([From] z:expression() { z })? end:position!() {
            zelf.new_located(begin, end, StmtKind::Raise { exc: Some(Box::new(a)), cause: option_box(b) })
        } /
        begin:position!() [Raise] {
            zelf.new_located_single(begin, StmtKind::Raise { exc: None, cause: None })
        }

    rule global_stmt() -> Stmt = begin:position!() [Global] a:([Name { name }] { name.clone() }) ++ [Comma] end:position!() {
        zelf.new_located(begin, end, StmtKind::Global { names: a })
    }

    rule nonlocal_stmt() -> Stmt = begin:position!() [Nonlocal] a:([Name { name }] { name.clone() }) ++ [Comma] end:position!() {
        zelf.new_located(begin, end, StmtKind::Nonlocal { names: a })
    }

    rule del_stmt() -> Stmt = begin:position!() [Del] a:del_targets() &[Comma | Newline] end:position!() {
        zelf.new_located(begin, end, StmtKind::Delete { targets: a })
    }

    rule yield_stmt() -> Stmt = begin:position!() a:yield_expr() end:position!() {
        zelf.new_located(begin, end, StmtKind::Expr { value: Box::new(a) })
    }

    rule assert_stmt() -> Stmt = begin:position!() [Assert] a:expression() b:([Comma] z:expression() { z })? end:position!() {
        zelf.new_located(begin, end, StmtKind::Assert { test: Box::new(a), msg: b.map(|x| Box::new(x)) })
    }

    rule import_stmt() -> Stmt = import_name() / import_from()

    rule import_name() -> Stmt = begin:position!() [Import] a:dotted_as_names() end:position!() {
        zelf.new_located(begin, end, StmtKind::Import { names: a })
    }

    rule import_from() -> Stmt =
        begin:position!() [From] a:[Dot | Ellipsis]* b:dotted_name() [Import] c:import_from_targets() end:position!() {
            zelf.new_located(begin, end, StmtKind::ImportFrom { module: Some(b), names: c, level: count_dots(a) })
        } /
        begin:position!() [From] a:[Dot | Ellipsis]+ [Import] b:import_from_targets() end:position!() {
            zelf.new_located(begin, end, StmtKind::ImportFrom { module: None, names: b, level: count_dots(a) })
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

    rule dotted_as_name() -> ast::Alias = begin:position!() a:dotted_name() b:([As] [Name { name }] { name })? end:position!() {
        zelf.new_located(begin, end, ast::AliasData { name: a, asname: b.cloned() })
    }

    #[cache_left_rec]
    rule dotted_name() -> std::string::String =
        a:dotted_name() [Dot] [Name { name }] {
            format!("{}.{}", a, name)
        } /
        [Name { name }] { name.clone() }

    rule block() -> Vec<Stmt> =
        [Newline] [Indent] a:statements() [Dedent] { a } /
        simple_stmts()

    rule decorators() -> Vec<Expr> = ([At] f:named_expression() [Newline] { f })+

    // rule class_def() -> Stmt =
    //     a:decorators() b:class_def_raw() {

    //     } /
    //     class_def_raw()

    // rule class_def_raw() -> StmtKind =
    //     begin:position!() [Class] [Name { name }] b:([Lpar] z:arguments()? [Rpar]) [Colon] c:block() end:position!() {
    //         zelf.new_located(begin, end, StmtKind::ClassDef { name: name.clone(), bases: b, keywords: b, body: c, decorator_list: vec![] })
    //     }

    rule if_stmt() -> Stmt =
        begin:position!() [If] a:named_expression() [Colon] b:block() c:elif_stmt() end:position!() {
            zelf.new_located(begin, end, StmtKind::If { test: Box::new(a), body: b, orelse: vec![c] })
        } /
        begin:position!() [If] a:named_expression() [Colon] b:block() c:else_block()? end:position!() {
            zelf.new_located(begin, end, StmtKind::If { test: Box::new(a), body: b, orelse: none_vec(c) })
        }

    rule elif_stmt() -> Stmt =
        begin:position!() [Elif] a:named_expression() [Colon] b:block() c:elif_stmt() end:position!() {
            zelf.new_located(begin, end, StmtKind::If { test: Box::new(a), body: b, orelse: vec![c] })
        } /
        begin:position!() [Elif] a:named_expression() [Colon] b:block() c:else_block()? end:position!() {
            zelf.new_located(begin, end, StmtKind::If { test: Box::new(a), body: b, orelse: none_vec(c) })
        }

    rule else_block() -> Vec<Stmt> = [Else] b:block() { b }

    rule while_stmt() -> Stmt =
        begin:position!() [While] a:named_expression() [Colon] b:block() c:else_block()? end:position!() {
            zelf.new_located(begin, end, StmtKind::While { test: Box::new(a), body: b, orelse: none_vec(c) })
        }

    rule for_stmt() -> Stmt =
        begin:position!() [For] t:star_targets() [In] ex:star_expressions() [Colon] tc:([Name { name }] { name })? b:block() el:else_block()? end:position!() {
            zelf.new_located(begin, end, StmtKind::For { target: Box::new(t), iter: Box::new(ex), body: b, orelse: none_vec(el), type_comment: tc.cloned() })
        } /
        begin:position!() [Async] [For] t:star_targets() [In] ex:star_expressions() [Colon] tc:([Name { name }] { name })? b:block() el:else_block()? end:position!() {
            zelf.new_located(begin, end, StmtKind::AsyncFor { target: Box::new(t), iter: Box::new(ex), body: b, orelse: none_vec(el), type_comment: tc.cloned() })
        }

    rule with_stmt() -> Stmt =
        begin:position!() [With] [Lpar] a:with_item() ++ [Comma] [Comma]? [Rpar] [Colon] b:block() end:position!() {
            zelf.new_located(begin, end, StmtKind::With { items: a, body: b, type_comment: None })
        } /
        begin:position!() [With] a:with_item() ++ [Comma] [Colon] tc:([Name { name }] { name })? b:block() end:position!() {
            zelf.new_located(begin, end, StmtKind::With { items: a, body: b, type_comment: tc.cloned() })
        } /
        begin:position!() [Async] [With] [Lpar] a:with_item() ++ [Comma] [Comma]? [Rpar] [Colon] b:block() end:position!() {
            zelf.new_located(begin, end, StmtKind::AsyncWith { items: a, body: b, type_comment: None })
        } /
        begin:position!() [Async] [With] a:with_item() ++ [Comma] [Colon] tc:([Name { name }] { name })? b:block() end:position!() {
            zelf.new_located(begin, end, StmtKind::AsyncWith { items: a, body: b, type_comment: tc.cloned() })
        }

    rule with_item() -> Withitem =
        e:expression() [As] t:star_target() &[Comma | Rpar | Colon] {
            Withitem { context_expr: e, optional_vars: Some(Box::new(t)) }
        } /
        e:expression() {
            Withitem { context_expr: e, optional_vars: None }
        }

    rule expressions() -> Expr =
        begin:position!() a:expression() **<2,> [Comma] [Comma]? end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Load })
        } /
        begin:position!() a:expression() [Comma] end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: vec![a], ctx: ExprContext::Load })
        } /
        expression()

    rule expression() -> Expr =
        begin:position!() a:disjunction() [If] b:disjunction() [Else] c:expression() end:position!() {
            zelf.new_located(begin, end, ExprKind::IfExp { test: Box::new(b), body: Box::new(a), orelse: Box::new(c) })
        } /
        disjunction()
        // TODO: lambdef

    rule yield_expr() -> Expr =
        begin:position!() [Yield] [From] a:expression() end:position!() {
            zelf.new_located(begin, end, ExprKind::YieldFrom { value: Box::new(a) })
        } /
        begin:position!() [Yield] a:expression()? end:position!() {
            zelf.new_located(begin, end, ExprKind::Yield { value: a.map(|x| Box::new(x)) })
        }

    rule star_expressions() -> Expr =
        begin:position!() a:star_expression() **<2,> [Comma] [Comma]? end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Load })
        } /
        begin:position!() a:star_expression() [Comma] end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: vec![a], ctx: ExprContext::Load })
        } /
        star_expression()

    rule star_expression() -> Expr =
        begin:position!() [Star] a:bitwise_or() end:position!() {
            zelf.new_located(begin, end, ExprKind::Starred { value: Box::new(a), ctx: ExprContext::Load })
        } /
        expression()

    rule star_named_expressions() -> Vec<Expr> =
        a:star_named_expression() ++ [Comma] [Comma]? { a }

    rule star_named_expression() -> Expr =
        begin:position!() [Star] a:bitwise_or() end:position!() {
            zelf.new_located(begin, end, ExprKind::Starred { value: Box::new(a), ctx: ExprContext::Load })
        }

    rule assignment_expression() -> Expr =
        begin:position!() [Name { name }] [ColonEqual] b:expression() end:position!() {
            let target = zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Store });
            zelf.new_located(begin, end, ExprKind::NamedExpr { target: Box::new(target), value: Box::new(b) })
        }

    rule named_expression() -> Expr =
        assignment_expression() /
        a:expression() ![ColonEqual] { a }

    rule disjunction() -> Expr = begin:position!() a:conjunction() ++ [Or] end:position!() {
        zelf.new_located(begin, end, ExprKind::BoolOp { op: ast::Boolop::Or, values: a })
    }

    rule conjunction() -> Expr = begin:position!() a:inversion() ++ [And] end:position!() {
        zelf.new_located(begin, end, ExprKind::BoolOp { op: ast::Boolop::And, values: a })
    }

    rule inversion() -> Expr =
        begin:position!() [Not] a:inversion() end:position!() {
            zelf.new_located(begin, end, ExprKind::UnaryOp { op: ast::Unaryop::Not, operand: Box::new(a) })
        } /
        comparison()

    rule comparison() -> Expr =
        begin:position!() a:bitwise_or() b:compare_op_bitwise_or_pair()+ end:position!() {
            let (ops, comparators) = comparison_ops_comparators(b);
            zelf.new_located(begin, end, ExprKind::Compare { left: Box::new(a), ops, comparators })
        }

    rule compare_op_bitwise_or_pair() -> (Cmpop, Expr) =
        eq_bitwise_or() /
        noteq_bitwise_or() /
        lte_bitwise_or() /
        lt_bitwise_or() /
        gte_bitwise_or() /
        gt_bitwise_or() /
        notin_bitwise_or() /
        in_bitwise_or() /
        isnot_bitwise_or() /
        is_bitwise_or()

    rule eq_bitwise_or() -> (Cmpop, Expr) = [EqEqual] a:bitwise_or() { (Cmpop::Eq, a) }
    rule noteq_bitwise_or() -> (Cmpop, Expr) = [NotEqual] a:bitwise_or() { (Cmpop::NotEq, a) }
    rule lte_bitwise_or() -> (Cmpop, Expr) = [LessEqual] a:bitwise_or() { (Cmpop::LtE, a) }
    rule lt_bitwise_or() -> (Cmpop, Expr) = [Less] a:bitwise_or() { (Cmpop::Lt, a) }
    rule gte_bitwise_or() -> (Cmpop, Expr) = [GreaterEqual] a:bitwise_or() { (Cmpop::GtE, a) }
    rule gt_bitwise_or() -> (Cmpop, Expr) = [Greater] a:bitwise_or() { (Cmpop::Gt, a) }
    rule notin_bitwise_or() -> (Cmpop, Expr) = [Not] [In] a:bitwise_or() { (Cmpop::NotIn, a) }
    rule in_bitwise_or() -> (Cmpop, Expr) = [In] a:bitwise_or() { (Cmpop::In, a) }
    rule isnot_bitwise_or() -> (Cmpop, Expr) = [Is] [Not] a:bitwise_or() { (Cmpop::IsNot, a) }
    rule is_bitwise_or() -> (Cmpop, Expr) = [Is] a:bitwise_or() { (Cmpop::Is, a) }

    #[cache_left_rec]
    rule bitwise_or() -> Expr =
        begin:position!() a:bitwise_or() [Or] b:bitwise_xor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitOr, right: Box::new(b) })
        } /
        bitwise_xor()

    #[cache_left_rec]
    rule bitwise_xor() -> Expr =
        begin:position!() a:bitwise_xor() [CircumFlex] b:bitwise_and() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitXor, right: Box::new(b) })
        } /
        bitwise_and()

    #[cache_left_rec]
    rule bitwise_and() -> Expr =
        begin:position!() a:bitwise_and() [Amper] b:shift_expr() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::BitAnd, right: Box::new(b) })
        } /
        shift_expr()

    #[cache_left_rec]
    rule shift_expr() -> Expr =
        begin:position!() a:shift_expr() [LeftShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::LShift, right: Box::new(b) })
        } /
        begin:position!() a:shift_expr() [RightShift] b:sum() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::RShift, right: Box::new(b) })
        } /
        sum()

    #[cache_left_rec]
    rule sum() -> Expr =
        begin:position!() a:sum() [Plus] b:term() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Add, right: Box::new(b) })
        } /
        begin:position!() a:sum() [Minus] b:term() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Sub, right: Box::new(b) })
        } /
        term()

    #[cache_left_rec]
    rule term() -> Expr =
        begin:position!() a:term() [Star] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mult, right: Box::new(b) })
        } /
        begin:position!() a:term() [Slash] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Div, right: Box::new(b) })
        } /
        begin:position!() a:term() [DoubleSlash] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::FloorDiv, right: Box::new(b) })
        } /
        begin:position!() a:term() [Percent] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Mod, right: Box::new(b) })
        } /
        begin:position!() a:term() [At] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::MatMult, right: Box::new(b) })
        } /
        factor()

    rule factor() -> Expr =
        begin:position!() [Plus] a:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::UnaryOp { op: ast::Unaryop::UAdd, operand: Box::new(a) })
        } /
        begin:position!() [Minus] a:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::UnaryOp { op: ast::Unaryop::USub, operand: Box::new(a) })
        } /
        begin:position!() [Tilde] a:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::UnaryOp { op: ast::Unaryop::Invert, operand: Box::new(a) })
        } /
        power()

    rule power() -> Expr =
        begin:position!() a:await_primary() [DoubleStar] b:factor() end:position!() {
            zelf.new_located(begin, end, ExprKind::BinOp { left: Box::new(a), op: ast::Operator::Pow, right: Box::new(b) })
        } /
        await_primary()

    rule await_primary() -> Expr =
        begin:position!() [Await] a:primary() end:position!() {
            zelf.new_located(begin, end, ExprKind::Await { value: Box::new(a) })
        } /
        primary()

    #[cache_left_rec]
    rule primary() -> Expr =
        begin:position!() a:primary() [Dot] [Name { name }] end:position!() {
            zelf.new_located(begin, end, ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ExprContext::Load })
        } /
        begin:position!() a:primary() b:genexp() end:position!() {
            zelf.new_located(begin, end, ExprKind::Call { func: Box::new(a), args: vec![b], keywords: vec![] })
        } /
        begin:position!() a:primary() [Lpar] b:arguments()? [Rpar] end:position!() {
            let (args, keywords) = if let Some(b) = b {
                (b.0, b.1)
            } else {
                (vec![], vec![])
            };
            zelf.new_located(begin, end, ExprKind::Call { func: Box::new(a), args, keywords })
        } /
        begin:position!() a:primary() [Lsqb] b:slices() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ExprContext::Load })
        } /
        atom()

    rule slices() -> Expr =
        a:slice() ![Comma] { a } /
        begin:position!() a:(slice() / starred_expression()) ++ [Comma] [Comma]? end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Load })
        }

    rule slice() -> Expr =
        begin:position!() a:expression()? [Colon] b:expression()? c:([Colon] d:expression() { d })? end:position!() {
            zelf.new_located(begin, end, ExprKind::Slice { lower: option_box(a), upper: option_box(b), step: option_box(c) })
        } /
        named_expression()

    rule atom() -> Expr =
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Load })
        } /
        begin:position!() [True] {
            zelf.new_located_single(begin, ExprKind::Constant { value: ast::Constant::Bool(true), kind: None })
        } /
        begin:position!() [False] {
            zelf.new_located_single(begin, ExprKind::Constant { value: ast::Constant::Bool(false), kind: None })
        } /
        begin:position!() [Tok::None] {
            zelf.new_located_single(begin, ExprKind::Constant { value: ast::Constant::None, kind: None })
        }
        // TODO: string

    // rule bitwise() -> Expr = precedence!{
    //     begin:position!() a:@ [BitOr] b:@ { zelf.new_located() }
    // }

    // rule compound_stmt() -> StmtKind = [Def]

    rule strings() -> Expr = a:string()+ {?
        // TODO: error handling
        crate::string::parse_strings(a).map_err(|_| "string format error")
    }

    rule string() -> (Location, (String, StringKind, bool), Location) =
        begin:position!() [Tok::String { value, kind, triple_quoted }] end:position!() {
            (zelf.locations[begin].0, (value.clone(), kind.clone(), triple_quoted.clone()), zelf.locations[end - 1].1)
        }

    rule list() -> Expr =
        begin:position!() [Lsqb] a:star_named_expressions()? [Rsqb] end:position!() {
            zelf.new_located(begin, end, ExprKind::List { elts: none_vec(a), ctx: ExprContext::Load })
        }

    rule tuple() -> Expr =
        begin:position!() [Lpar] a:star_named_expressions()? [Rpar] end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: none_vec(a), ctx: ExprContext::Load })
        }

    rule set() -> Expr =
        begin:position!() [Lbrace] a:star_named_expressions() [Rbrace] end:position!() {
            zelf.new_located(begin, end, ExprKind::Set { elts: a })
        }

    rule dict() -> Expr =
        begin:position!() [Lbrace] a:double_starred_kvpairs()? [Rbrace] end:position!() {
            let (keys, values) = if let Some(a) = a {
                dict_kvpairs(a)
            } else {
                (vec![], vec![])
            };
            zelf.new_located(begin, end, ExprKind::Dict { keys, values })
        }

    rule double_starred_kvpairs() -> Vec<(Option<Expr>, Expr)> =
        a:double_starred_kvpair() ++ [Comma] [Comma]? { a }

    rule double_starred_kvpair() -> (Option<Expr>, Expr) =
        [DoubleStar] a:bitwise_or() { (None, a) } /
        a:kvpair() { (Some(a.0), a.1) }

    rule kvpair() -> (Expr, Expr) =
        a:expression() [Colon] b:expression() { (a, b) }

    rule for_if_clauses() -> Vec<Comprehension> = for_if_clause()+

    rule for_if_clause() -> Comprehension =
        [Async] [For] a:star_targets() [In] b:disjunction() c:([If] z:disjunction() { z })* {
            Comprehension { target: a, iter: b, ifs: c, is_async: 1 }
        } /
        [For] a:star_targets() [In] b:disjunction() c:([If] z:disjunction() { z })* {
            Comprehension { target: a, iter: b, ifs: c, is_async: 0 }
        }

    rule listcomp() -> Expr =
        begin:position!() [Lsqb] a:named_expression() b:for_if_clauses() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ExprKind::ListComp { elt: Box::new(a), generators: b })
        }

    rule setcomp() -> Expr =
        begin:position!() [Lbrace] a:named_expression() b:for_if_clauses() [Rbrace] end:position!() {
            zelf.new_located(begin, end, ExprKind::SetComp { elt: Box::new(a), generators: b })
        }

    rule genexp() -> Expr =
        begin:position!() [Lpar] a:(assignment_expression() / z:expression() ![ColonEqual] { z }) b:for_if_clauses() [Rpar] end:position!() {
            zelf.new_located(begin, end, ExprKind::GeneratorExp { elt: Box::new(a), generators: b })
        }

    rule dictcomp() -> Expr =
        begin:position!() [Lbrace] a:kvpair() b:for_if_clauses() [Rbrace] end:position!() {
            zelf.new_located(begin, end, ExprKind::DictComp { key: Box::new(a.0), value: Box::new(a.1), generators: b })
        }

    rule arguments() -> (Vec<Expr>, Vec<Keyword>) = a:args() [Comma]? &[Rpar] { a }

    rule args() -> (Vec<Expr>, Vec<Keyword>) =
        // a:(starred_expression() / (assignment_expression() / expression() ![ColonEqual]) ![Equal]) ++ [Comma] b:([Comma] k:kwargs() { k })? {
        //     (a, none_vec(b))
        // } /
        a:kwargs() {
            keyword_or_starred_partition(a)
        }

    rule kwargs() -> Vec<KeywordOrStarred> =
        a:kwarg_or_starred() ++ [Comma] b:kwarg_or_double_starred() ++ [Comma] {
            let mut a = a;
            let mut b = b;
            a.append(&mut b);
            a
        } /
        kwarg_or_starred() ++ [Comma] /
        kwarg_or_double_starred() ++ [Comma]

    rule starred_expression() -> Expr =
        begin:position!() [Star] a:expression() end:position!() {
            zelf.new_located(begin, end, ExprKind::Starred { value: Box::new(a), ctx: ExprContext::Load })
        }

    rule kwarg_or_starred() -> KeywordOrStarred =
        begin:position!() [Name { name }] [Equal] b:expression() end:position!() {
            KeywordOrStarred::Keyword(zelf.new_located(begin, end, KeywordData { arg: Some(name.clone()), value: b }))
        } /
        a:starred_expression() {
            KeywordOrStarred::Starred(a)
        }

    rule kwarg_or_double_starred() -> KeywordOrStarred =
        begin:position!() [Name { name }] [Equal] b:expression() end:position!() {
            KeywordOrStarred::Keyword(zelf.new_located(begin, end, KeywordData { arg: Some(name.clone()), value: b }))
        } /
        begin:position!() [DoubleStar] a:expression() end:position!() {
            KeywordOrStarred::Keyword(zelf.new_located(begin, end, KeywordData { arg: None, value: a }))
        }

    rule star_targets() -> Expr =
        a:star_target() ![Comma] { a } /
        begin:position!() a:star_target() ++ [Comma] [Comma]? end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Store })
        }

    rule star_targets_list() -> Vec<Expr> = a:star_target() ++ [Comma] [Comma]? { a }

    rule star_targets_tuple() -> Vec<Expr> =
        a:star_target() **<2,> [Comma] [Comma]? { a } /
        a:star_target() [Comma] { vec![a] }

    rule star_target() -> Expr =
        begin:position!() [Star] ![Star] a:star_target() end:position!() {
            zelf.new_located(begin, end, ExprKind::Starred { value: Box::new(a), ctx: ExprContext::Store })
        } /
        target_with_star_atom()

    rule target_with_star_atom() -> Expr =
        single_subscript_attribute_target() /
        star_atom()

    rule star_atom() -> Expr =
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Store })
        } /
        [Lpar] a:target_with_star_atom() [Rpar] { a } /
        begin:position!() [Lpar] a:star_targets_tuple() [Rpar] end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Store })
        } /
        begin:position!() [Lsqb] a:star_targets_list() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ExprKind::List { elts: a, ctx: ExprContext::Store })
        }

    rule single_target() -> Expr =
        single_subscript_attribute_target() /
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Store })
        } /
        [Lpar] a:single_target() [Rpar] { a }

    rule single_subscript_attribute_target() -> Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ExprContext::Store })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ExprContext::Store })
        }

    #[cache_left_rec]
    rule t_primary() -> Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ExprContext::Load })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] &t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ExprContext::Load })
        }
        // TODO:

    rule t_lookahead() = [Lpar] / [Lsqb] / [Dot]

    rule del_targets() -> Vec<Expr> = a:del_target() ++ [Comma] [Comma]? { a }

    rule del_target() -> Expr =
        begin:position!() a:t_primary() [Dot] [Name { name }] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Attribute { value: Box::new(a), attr: name.clone(), ctx: ExprContext::Del })
        } /
        begin:position!() a:t_primary() [Lsqb] b:slices() [Rsqb] !t_lookahead() end:position!() {
            zelf.new_located(begin, end, ExprKind::Subscript { value: Box::new(a), slice: Box::new(b), ctx: ExprContext::Del })
        } /
        del_t_atom()

    rule del_t_atom() -> Expr =
        begin:position!() [Name { name }] {
            zelf.new_located_single(begin, ExprKind::Name { id: name.clone(), ctx: ExprContext::Del })
        } /
        begin:position!() [Lpar] a:del_target() [Rpar] end:position!() { a } /
        begin:position!() [Lpar] a:del_targets() [Rpar] end:position!() {
            zelf.new_located(begin, end, ExprKind::Tuple { elts: a, ctx: ExprContext::Del })
        } /
        begin:position!() [Lsqb] a:del_targets() [Rsqb] end:position!() {
            zelf.new_located(begin, end, ExprKind::List { elts: a, ctx: ExprContext::Del })
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

fn none_vec<T>(v: Option<Vec<T>>) -> Vec<T> {
    if let Some(v) = v {
        v
    } else {
        vec![]
    }
}

fn option_box<T>(val: Option<T>) -> Option<Box<T>> {
    val.map(|x| Box::new(x))
}

enum KeywordOrStarred {
    Keyword(ast::Keyword),
    Starred(ast::Expr),
}

fn keyword_or_starred_partition(v: Vec<KeywordOrStarred>) -> (Vec<ast::Expr>, Vec<ast::Keyword>) {
    let mut ex_vec = vec![];
    let mut kw_vec = vec![];
    for x in v {
        match x {
            KeywordOrStarred::Keyword(kw) => kw_vec.push(kw),
            KeywordOrStarred::Starred(ex) => ex_vec.push(ex),
        }
    }
    (ex_vec, kw_vec)
}

fn dict_kvpairs(v: Vec<(Option<ast::Expr>, ast::Expr)>) -> (Vec<ast::Expr>, Vec<ast::Expr>) {
    let mut keys = Vec::with_capacity(v.len());
    let mut values = Vec::with_capacity(v.len());

    let (packed, unpacked) = v.into_iter().partition::<Vec<_>, _>(|x| x.0.is_some());
    for x in packed {
        keys.push(x.0.unwrap());
        values.push(x.1);
    }
    for x in unpacked {
        values.push(x.1);
    }
    (keys, values)
}

fn comparison_ops_comparators(
    v: Vec<(ast::Cmpop, ast::Expr)>,
) -> (Vec<ast::Cmpop>, Vec<ast::Expr>) {
    let mut ops = Vec::with_capacity(v.len());
    let mut comparators = Vec::with_capacity(v.len());

    for x in v {
        ops.push(x.0);
        comparators.push(x.1);
    }
    (ops, comparators)
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
