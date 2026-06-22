use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) fn ast_to_object(
    clause: ast::ElifElseClause,
    mut rest: alloc::vec::IntoIter<ast::ElifElseClause>,
    vm: &VirtualMachine,
    source_file: &SourceFile,
) -> PyObjectRef {
    let ast::ElifElseClause {
        node_index,
        range,
        test,
        body,
    } = clause;
    let Some(test) = test else {
        assert!(rest.len() == 0);
        return body.ast_to_object(vm, source_file);
    };
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeStmtIf::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();

    dict.set_item("test", test.ast_to_object(vm, source_file), vm)
        .unwrap();
    let body = super::constant::public_ast_stmt_list_object(
        node_index.load(),
        super::constant::PublicAstStmtListField::Body,
    )
    .map_or_else(
        || body.ast_to_object(vm, source_file),
        |values| values.values.ast_to_object(vm, source_file),
    );
    dict.set_item("body", body, vm).unwrap();

    let orelse = if let Some(values) = super::constant::public_ast_stmt_list_object(
        node_index.load(),
        super::constant::PublicAstStmtListField::Orelse,
    ) {
        values.values.ast_to_object(vm, source_file)
    } else if let Some(next) = rest.next() {
        if next.test.is_some() {
            let next = ast::ElifElseClause {
                range: TextRange::new(next.range.start(), range.end()),
                ..next
            };
            vm.ctx
                .new_list(vec![ast_to_object(next, rest, vm, source_file)])
                .into()
        } else {
            next.body.ast_to_object(vm, source_file)
        }
    } else {
        vm.ctx.new_list(vec![]).into()
    };
    dict.set_item("orelse", orelse, vm).unwrap();

    node_add_location(&dict, range, vm, source_file);
    node.into()
}

pub(super) fn ast_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtIf> {
    let test = get_required_node_field(vm, source_file, &object, "test", "If")?;
    let body: Vec<Option<ast::Stmt>> = get_node_list_field(vm, source_file, &object, "body", "If")?;
    let orelse: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "orelse", "If")?;
    let node_index = public_stmt_lists_node_index([
        (super::constant::PublicAstStmtListField::Body, &body),
        (super::constant::PublicAstStmtListField::Orelse, &orelse),
    ]);
    let body = lower_public_stmt_list(body);
    let orelse = lower_public_stmt_list(orelse);

    let elif_else_clauses = if orelse.is_empty() {
        vec![]
    } else if let [ast::Stmt::If(_)] = &*orelse {
        let Some(ast::Stmt::If(ast::StmtIf {
            node_index,
            range,
            test,
            body,
            mut elif_else_clauses,
        })) = orelse.into_iter().next()
        else {
            unreachable!()
        };
        elif_else_clauses.insert(
            0,
            ast::ElifElseClause {
                node_index,
                range,
                test: Some(*test),
                body,
            },
        );
        elif_else_clauses
    } else {
        vec![ast::ElifElseClause {
            node_index: Default::default(),
            range,
            test: None,
            body: orelse,
        }]
    };

    Ok(ast::StmtIf {
        node_index,
        test,
        body,
        elif_else_clauses,
        range,
    })
}
