use crate::{
    AsObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{PyList, PyTuple},
    class::PyClassImpl,
    stdlib::ast::NodeAst,
};

fn repr_ast_list(vm: &VirtualMachine, items: Vec<PyObjectRef>, depth: usize) -> PyResult<String> {
    if items.is_empty() {
        let empty_list: PyObjectRef = vm.ctx.new_list(vec![]).into();
        return Ok(empty_list.repr(vm)?.to_string());
    }

    let mut parts: Vec<String> = Vec::new();
    let first = &items[0];
    let last = items.last().unwrap();

    for (idx, item) in [first, last].iter().enumerate() {
        if idx == 1 && items.len() == 1 {
            break;
        }
        let repr = if item.fast_isinstance(&NodeAst::make_class(&vm.ctx)) {
            repr_ast_node(vm, item, depth.saturating_sub(1))?
        } else {
            item.repr(vm)?.to_string()
        };
        parts.push(repr);
    }

    let mut rendered = String::from("[");
    if !parts.is_empty() {
        rendered.push_str(&parts[0]);
    }
    if items.len() > 2 {
        if !parts[0].is_empty() {
            rendered.push_str(", ...");
        }
        if parts.len() > 1 {
            rendered.push_str(", ");
            rendered.push_str(&parts[1]);
        }
    } else if parts.len() > 1 {
        rendered.push_str(", ");
        rendered.push_str(&parts[1]);
    }
    rendered.push(']');
    Ok(rendered)
}

fn repr_ast_tuple(vm: &VirtualMachine, items: Vec<PyObjectRef>, depth: usize) -> PyResult<String> {
    if items.is_empty() {
        let empty_tuple: PyObjectRef = vm.ctx.empty_tuple.clone().into();
        return Ok(empty_tuple.repr(vm)?.to_string());
    }

    let mut parts: Vec<String> = Vec::new();
    let first = &items[0];
    let last = items.last().unwrap();

    for (idx, item) in [first, last].iter().enumerate() {
        if idx == 1 && items.len() == 1 {
            break;
        }
        let repr = if item.fast_isinstance(&NodeAst::make_class(&vm.ctx)) {
            repr_ast_node(vm, item, depth.saturating_sub(1))?
        } else {
            item.repr(vm)?.to_string()
        };
        parts.push(repr);
    }

    let mut rendered = String::from("(");
    if !parts.is_empty() {
        rendered.push_str(&parts[0]);
    }
    if items.len() > 2 {
        if !parts[0].is_empty() {
            rendered.push_str(", ...");
        }
        if parts.len() > 1 {
            rendered.push_str(", ");
            rendered.push_str(&parts[1]);
        }
    } else if parts.len() > 1 {
        rendered.push_str(", ");
        rendered.push_str(&parts[1]);
    }
    if items.len() == 1 {
        rendered.push(',');
    }
    rendered.push(')');
    Ok(rendered)
}

pub(crate) fn repr_ast_node(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    depth: usize,
) -> PyResult<String> {
    let cls = obj.class();
    if depth == 0 {
        return Ok(format!("{}(...)", cls.name()));
    }

    let fields = cls.get_attr(vm.ctx.intern_str("_fields"));
    let fields = match fields {
        Some(fields) => fields.try_to_value::<Vec<crate::builtins::PyStrRef>>(vm)?,
        None => return Ok(format!("{}(...)", cls.name())),
    };

    if fields.is_empty() {
        return Ok(format!("{}()", cls.name()));
    }

    let mut rendered = String::new();
    rendered.push_str(&cls.name());
    rendered.push('(');

    for (idx, field) in fields.iter().enumerate() {
        let value = obj.get_attr(field, vm)?;
        let value_repr = if value.fast_isinstance(vm.ctx.types.list_type) {
            let list = value
                .downcast::<PyList>()
                .expect("list type should downcast");
            repr_ast_list(vm, list.borrow_vec().to_vec(), depth)?
        } else if value.fast_isinstance(vm.ctx.types.tuple_type) {
            let tuple = value
                .downcast::<PyTuple>()
                .expect("tuple type should downcast");
            repr_ast_tuple(vm, tuple.as_slice().to_vec(), depth)?
        } else if value.fast_isinstance(&NodeAst::make_class(&vm.ctx)) {
            repr_ast_node(vm, &value, depth.saturating_sub(1))?
        } else {
            value.repr(vm)?.to_string()
        };

        if idx > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(field.as_str());
        rendered.push('=');
        rendered.push_str(&value_repr);
    }

    rendered.push(')');
    Ok(rendered)
}
