use super::*;

impl Node for Vec<ruff::ElifElseClause> {
    fn ast_to_object(self, _vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        todo!()
    }
}

pub(super) fn ast_to_object(
    clause: ruff::ElifElseClause,
    mut rest: std::vec::IntoIter<ruff::ElifElseClause>,
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
) -> PyObjectRef {
    let ruff::ElifElseClause { range, test, body } = clause;
    let Some(test) = test else {
        assert!(rest.len() == 0);
        return body.ast_to_object(vm, source_code);
    };
    let node = NodeAst
        .into_ref_with_type(vm, gen::NodeStmtIf::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();

    dict.set_item("test", test.ast_to_object(vm, source_code), vm)
        .unwrap();
    dict.set_item("body", body.ast_to_object(vm, source_code), vm)
        .unwrap();

    let orelse = if let Some(next) = rest.next() {
        if next.test.is_some() {
            vm.ctx
                .new_list(vec![ast_to_object(next, rest, vm, source_code)])
                .into()
        } else {
            next.body.ast_to_object(vm, source_code)
        }
    } else {
        vm.ctx.new_list(vec![]).into()
    };
    dict.set_item("orelse", orelse, vm).unwrap();

    node_add_location(&dict, range, vm, source_code);
    node.into()
}

pub(super) fn ast_from_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    object: PyObjectRef,
) -> PyResult<ruff::StmtIf> {
    let test = Node::ast_from_object(vm, source_code, get_node_field(vm, &object, "test", "If")?)?;
    let body = Node::ast_from_object(vm, source_code, get_node_field(vm, &object, "body", "If")?)?;
    let orelse: Vec<ruff::Stmt> = Node::ast_from_object(
        vm,
        source_code,
        get_node_field(vm, &object, "orelse", "If")?,
    )?;
    let range = range_from_object(vm, source_code, object, "If")?;

    let elif_else_clauses = if let [ruff::Stmt::If(_)] = &*orelse {
        let Some(ruff::Stmt::If(ruff::StmtIf {
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
            ruff::ElifElseClause {
                range,
                test: Some(*test),
                body,
            },
        );
        elif_else_clauses
    } else {
        vec![ruff::ElifElseClause {
            range,
            test: None,
            body: orelse,
        }]
    };

    Ok(ruff::StmtIf {
        test,
        body,
        elif_else_clauses,
        range,
    })
}
