use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) fn ast_to_object(
    clause: ast::ElifElseClause,
    mut rest: alloc::vec::IntoIter<ast::ElifElseClause>,
    vm: &VirtualMachine,
    source_file: &SourceFile,
) -> PyObjectRef {
    let ast::ElifElseClause {
        node_index: _,
        range,
        test,
        body,
        runtime_body,
        runtime_orelse,
    } = clause;
    let Some(test) = test else {
        assert!(rest.len() == 0);
        return runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
    };
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeStmtIf::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();

    dict.set_item("test", test.ast_to_object(vm, source_file), vm)
        .unwrap();
    let body = runtime_body.map_or_else(
        || body.ast_to_object(vm, source_file),
        |values| values.ast_to_object(vm, source_file),
    );
    dict.set_item("body", body, vm).unwrap();

    let orelse = if let Some(values) = runtime_orelse {
        values.ast_to_object(vm, source_file)
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
            next.runtime_body.map_or_else(
                || next.body.ast_to_object(vm, source_file),
                |values| values.ast_to_object(vm, source_file),
            )
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
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_orelse = runtime_stmt_list_metadata(&orelse);
    let body = lower_runtime_stmt_list(body);
    let orelse = lower_runtime_stmt_list(orelse);

    let elif_else_clauses = if orelse.is_empty() {
        vec![]
    } else if let [ast::Stmt::If(_)] = &*orelse {
        let Some(ast::Stmt::If(ast::StmtIf {
            node_index,
            range,
            test,
            body,
            mut elif_else_clauses,
            runtime_body,
        })) = orelse.into_iter().next()
        else {
            unreachable!()
        };
        debug_assert!(runtime_orelse.is_none());
        elif_else_clauses.insert(
            0,
            ast::ElifElseClause {
                node_index,
                range,
                test: Some(*test),
                body,
                runtime_body,
                runtime_orelse: None,
            },
        );
        elif_else_clauses
    } else {
        vec![ast::ElifElseClause {
            node_index: Default::default(),
            range,
            test: None,
            body: orelse,
            runtime_body: runtime_orelse,
            runtime_orelse: None,
        }]
    };

    Ok(ast::StmtIf {
        node_index: Default::default(),
        test,
        body,
        elif_else_clauses,
        range,
        runtime_body,
    })
}
