//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

use std::ops::Deref;

use num_complex::Complex64;

use rustpython_parser::{ast, mode::Mode, parser};

use crate::builtins::list::PyListRef;
use crate::builtins::pytype::PyTypeRef;
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
};
use crate::vm::VirtualMachine;

#[pyclass(module = "_ast", name = "AST")]
#[derive(Debug)]
struct AstNode;
type AstNodeRef = PyRef<AstNode>;

#[pyimpl(flags(HAS_DICT))]
impl AstNode {}

const MODULE_NAME: &str = "_ast";
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

impl PyValue for AstNode {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

macro_rules! node {
    ( $vm: expr, $node_name:ident, { $($attr_name:ident => $attr_value:expr),* $(,)* }) => {
        {
        let node = create_node($vm, stringify!($node_name))?;
        let mut field_names = vec![];
        $(
            let field_name = stringify!($attr_name);
            $vm.set_attr(node.as_object(), field_name, $attr_value)?;
            field_names.push($vm.ctx.new_str(field_name));
        )*
        $vm.set_attr(node.as_object(), "_fields", $vm.ctx.new_tuple(field_names))?;
        node
        }
    };
    ( $vm: expr, $node_name:ident) => {
        {
        let node = create_node($vm, stringify!($node_name))?;
        $vm.set_attr(node.as_object(), "_fields", $vm.ctx.new_tuple(vec![]))?;
        node
        }
    }
}

fn top_to_ast(vm: &VirtualMachine, top: &ast::Top) -> PyResult<PyListRef> {
    match top {
        ast::Top::Program(program) => statements_to_ast(vm, &program.statements),
        ast::Top::Statement(statements) => statements_to_ast(vm, statements),
        ast::Top::Expression(_) => unimplemented!("top_to_ast unimplemented ast::Top::Expression"),
    }
}

// Create a node class instance
fn create_node(vm: &VirtualMachine, name: &str) -> PyResult<AstNodeRef> {
    AstNode.into_ref_with_type(vm, vm.class(MODULE_NAME, name))
}

fn statements_to_ast(vm: &VirtualMachine, statements: &[ast::Statement]) -> PyResult<PyListRef> {
    let body: PyResult<Vec<_>> = statements
        .iter()
        .map(|statement| Ok(statement_to_ast(&vm, statement)?.into_object()))
        .collect();
    Ok(vm.ctx.new_list(body?).downcast().unwrap())
}

fn statement_to_ast(vm: &VirtualMachine, statement: &ast::Statement) -> PyResult<AstNodeRef> {
    use ast::StatementType::*;
    let node = match &statement.node {
        ClassDef {
            name,
            body,
            keywords,
            decorator_list,
            ..
        } => node!(vm, ClassDef, {
            name => vm.ctx.new_str(name),
            keywords => map_ast(keyword_to_ast, vm, keywords)?,
            body => statements_to_ast(vm, body)?,
            decorator_list => expressions_to_ast(vm, decorator_list)?,
        }),
        FunctionDef {
            is_async,
            name,
            args,
            body,
            decorator_list,
            returns,
        } => {
            if *is_async {
                node!(vm, AsyncFunctionDef, {
                    name => vm.ctx.new_str(name),
                    args => parameters_to_ast(vm, args)?,
                    body => statements_to_ast(vm, body)?,
                    decorator_list => expressions_to_ast(vm, decorator_list)?,
                    returns => optional_expression_to_ast(vm, returns)?
                })
            } else {
                node!(vm, FunctionDef, {
                    name => vm.ctx.new_str(name),
                    args => parameters_to_ast(vm, args)?,
                    body => statements_to_ast(vm, body)?,
                    decorator_list => expressions_to_ast(vm, decorator_list)?,
                    returns => optional_expression_to_ast(vm, returns)?
                })
            }
        }
        Continue => node!(vm, Continue),
        Break => node!(vm, Break),
        Pass => node!(vm, Pass),
        Assert { test, msg } => node!(vm, Assert, {
            test => expression_to_ast(vm, test)?,
            msg => optional_expression_to_ast(vm, msg)?
        }),
        Delete { targets } => {
            let targets: PyResult<_> = targets
                .iter()
                .map(|v| Ok(expression_to_ast(vm, v)?.into_object()))
                .collect();
            let py_targets = vm.ctx.new_tuple(targets?);
            node!(vm, Delete, { targets => py_targets })
        }
        Return { value } => node!(vm, Return, {
            value => optional_expression_to_ast(vm, value)?
        }),
        If { test, body, orelse } => node!(vm, If, {
            test => expression_to_ast(vm, test)?,
            body => statements_to_ast(vm, body)?,
            orelse => optional_statements_to_ast(vm, orelse)?
        }),
        For {
            is_async,
            target,
            iter,
            body,
            orelse,
        } => {
            if *is_async {
                node!(vm, AsyncFor, {
                    target => expression_to_ast(vm, target)?,
                    iter => expression_to_ast(vm, iter)?,
                    body => statements_to_ast(vm, body)?,
                    orelse => optional_statements_to_ast(vm, orelse)?
                })
            } else {
                node!(vm, For, {
                    target => expression_to_ast(vm, target)?,
                    iter => expression_to_ast(vm, iter)?,
                    body => statements_to_ast(vm, body)?,
                    orelse => optional_statements_to_ast(vm, orelse)?
                })
            }
        }
        While { test, body, orelse } => node!(vm, While, {
            test => expression_to_ast(vm, test)?,
            body => statements_to_ast(vm, body)?,
            orelse => optional_statements_to_ast(vm, orelse)?
        }),
        With {
            is_async,
            items,
            body,
        } => {
            if *is_async {
                node!(vm, AsyncWith, {
                    items => map_ast(with_item_to_ast, vm, items)?,
                    body => statements_to_ast(vm, body)?
                })
            } else {
                node!(vm, With, {
                    items => map_ast(with_item_to_ast, vm, items)?,
                    body => statements_to_ast(vm, body)?
                })
            }
        }
        Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => node!(vm, Try, {
            body => statements_to_ast(vm, body)?,
            handlers => map_ast(handler_to_ast, vm, handlers)?,
            orelse => optional_statements_to_ast(vm, orelse)?,
            finalbody => optional_statements_to_ast(vm, finalbody)?
        }),
        Expression { expression } => node!(vm, Expr, {
            value => expression_to_ast(vm, expression)?
        }),
        Import { names } => node!(vm, Import, {
            names => map_ast(alias_to_ast, vm, names)?
        }),
        ImportFrom {
            level,
            module,
            names,
        } => node!(vm, ImportFrom, {
            level => vm.ctx.new_int(*level),
            module => module.as_ref().into_pyobject(vm),
            names => map_ast(alias_to_ast, vm, names)?
        }),
        Nonlocal { names } => node!(vm, Nonlocal, {
            names => make_string_list(vm, names)
        }),
        Global { names } => node!(vm, Global, {
            names => make_string_list(vm, names)
        }),
        Assign { targets, value } => node!(vm, Assign, {
            targets => expressions_to_ast(vm, targets)?,
            value => expression_to_ast(vm, value)?,
        }),
        AugAssign { target, op, value } => node!(vm, AugAssign, {
            target => expression_to_ast(vm, target)?,
            op => vm.ctx.new_str(operator_string(op)),
            value => expression_to_ast(vm, value)?,
        }),
        AnnAssign {
            target,
            annotation,
            value,
        } => node!(vm, AnnAssign, {
            target => expression_to_ast(vm, target)?,
            annotation => expression_to_ast(vm, annotation)?,
            value => optional_expression_to_ast(vm, value)?,
        }),
        Raise { exception, cause } => node!(vm, Raise, {
            exc => optional_expression_to_ast(vm, exception)?,
            cause => optional_expression_to_ast(vm, cause)?,
        }),
    };

    // set lineno on node:
    let lineno = vm.ctx.new_int(statement.location.row());
    vm.set_attr(node.as_object(), "lineno", lineno).unwrap();

    Ok(node)
}

fn alias_to_ast(vm: &VirtualMachine, alias: &ast::ImportSymbol) -> PyResult<AstNodeRef> {
    Ok(node!(vm, alias, {
        name => vm.ctx.new_str(&alias.symbol),
        asname => alias.alias.as_ref().into_pyobject(vm),
    }))
}

fn optional_statements_to_ast(
    vm: &VirtualMachine,
    statements: &Option<Vec<ast::Statement>>,
) -> PyResult {
    let statements = if let Some(statements) = statements {
        statements_to_ast(vm, statements)?.into_object()
    } else {
        vm.ctx.new_list(vec![])
    };
    Ok(statements)
}

fn with_item_to_ast(vm: &VirtualMachine, with_item: &ast::WithItem) -> PyResult<AstNodeRef> {
    let node = node!(vm, withitem, {
        context_expr => expression_to_ast(vm, &with_item.context_expr)?,
        optional_vars => optional_expression_to_ast(vm, &with_item.optional_vars)?
    });
    Ok(node)
}

fn handler_to_ast(vm: &VirtualMachine, handler: &ast::ExceptHandler) -> PyResult<AstNodeRef> {
    let node = node!(vm, ExceptHandler, {
        typ => optional_expression_to_ast(vm, &handler.typ)?,
        name => handler.name.as_ref().into_pyobject(vm),
        body => statements_to_ast(vm, &handler.body)?,
    });
    Ok(node)
}

fn make_string_list(vm: &VirtualMachine, names: &[String]) -> PyObjectRef {
    vm.ctx
        .new_list(names.iter().map(|x| vm.ctx.new_str(x)).collect())
}

fn optional_expressions_to_ast(
    vm: &VirtualMachine,
    expressions: &[Option<ast::Expression>],
) -> PyResult<PyListRef> {
    let py_expression_nodes: PyResult<_> = expressions
        .iter()
        .map(|expression| Ok(optional_expression_to_ast(vm, expression)?))
        .collect();
    Ok(vm.ctx.new_list(py_expression_nodes?).downcast().unwrap())
}

fn optional_expression_to_ast(vm: &VirtualMachine, value: &Option<ast::Expression>) -> PyResult {
    let ast = value
        .as_ref()
        .map(|expr| expression_to_ast(vm, expr))
        .transpose()?;
    Ok(ast.into_pyobject(vm))
}

fn expressions_to_ast(vm: &VirtualMachine, expressions: &[ast::Expression]) -> PyResult<PyListRef> {
    let py_expression_nodes: PyResult<_> = expressions
        .iter()
        .map(|expression| Ok(expression_to_ast(vm, expression)?.into_object()))
        .collect();
    Ok(vm.ctx.new_list(py_expression_nodes?).downcast().unwrap())
}

fn expression_to_ast(vm: &VirtualMachine, expression: &ast::Expression) -> PyResult<AstNodeRef> {
    use ast::ExpressionType::*;
    let node = match &expression.node {
        Call {
            function,
            args,
            keywords,
        } => node!(vm, Call, {
            func => expression_to_ast(vm, function)?,
            args => expressions_to_ast(vm, args)?,
            keywords => map_ast(keyword_to_ast, vm, keywords)?,
        }),
        Binop { a, op, b } => {
            // Operator:
            node!(vm, BinOp, {
                left => expression_to_ast(vm, a)?,
                op => vm.ctx.new_str(operator_string(op)),
                right => expression_to_ast(vm, b)?,
            })
        }
        Unop { op, a } => {
            let op = match op {
                ast::UnaryOperator::Not => "Not",
                ast::UnaryOperator::Inv => "Invert",
                ast::UnaryOperator::Neg => "USub",
                ast::UnaryOperator::Pos => "UAdd",
            };
            node!(vm, UnaryOp, {
                op => vm.ctx.new_str(op),
                operand => expression_to_ast(vm, a)?,
            })
        }
        BoolOp { op, values } => {
            let py_values = expressions_to_ast(vm, values)?;

            let str_op = match op {
                ast::BooleanOperator::And => "And",
                ast::BooleanOperator::Or => "Or",
            };
            let py_op = vm.ctx.new_str(str_op);

            node!(vm, BoolOp, {
                op => py_op,
                values => py_values,
            })
        }
        Compare { vals, ops } => {
            let left = expression_to_ast(vm, &vals[0])?;

            // Operator:
            let to_operator = |op: &ast::Comparison| match op {
                ast::Comparison::Equal => "Eq",
                ast::Comparison::NotEqual => "NotEq",
                ast::Comparison::Less => "Lt",
                ast::Comparison::LessOrEqual => "LtE",
                ast::Comparison::Greater => "Gt",
                ast::Comparison::GreaterOrEqual => "GtE",
                ast::Comparison::In => "In",
                ast::Comparison::NotIn => "NotIn",
                ast::Comparison::Is => "Is",
                ast::Comparison::IsNot => "IsNot",
            };
            let ops = vm
                .ctx
                .new_list(ops.iter().map(|x| vm.ctx.new_str(to_operator(x))).collect());

            let comparators: PyResult<_> = vals
                .iter()
                .skip(1)
                .map(|x| Ok(expression_to_ast(vm, x)?.into_object()))
                .collect();
            let comparators = vm.ctx.new_list(comparators?);
            node!(vm, Compare, {
                left => left,
                ops => ops,
                comparators => comparators,
            })
        }
        Identifier { name } => node!(vm, Name, {
            id => vm.ctx.new_str(name),
            ctx => vm.ctx.none()   // TODO: add context.
        }),
        Lambda { args, body } => node!(vm, Lambda, {
            args => parameters_to_ast(vm, args)?,
            body => expression_to_ast(vm, body)?,
        }),
        IfExpression { test, body, orelse } => node!(vm, IfExp, {
            text => expression_to_ast(vm, test)?,
            body => expression_to_ast(vm, body)?,
            or_else => expression_to_ast(vm, orelse)?,
        }),
        Number { value } => {
            let py_n = match value {
                ast::Number::Integer { value } => vm.ctx.new_bigint(value),
                ast::Number::Float { value } => vm.ctx.new_float(*value),
                ast::Number::Complex { real, imag } => {
                    vm.ctx.new_complex(Complex64::new(*real, *imag))
                }
            };
            node!(vm, Num, {
                n => py_n
            })
        }
        True => node!(vm, NameConstant, {
            value => vm.ctx.new_bool(true)
        }),
        False => node!(vm, NameConstant, {
            value => vm.ctx.new_bool(false)
        }),
        None => node!(vm, NameConstant, {
            value => vm.ctx.none()
        }),
        Ellipsis => node!(vm, Ellipsis),
        List { elements } => node!(vm, List, {
            elts => expressions_to_ast(vm, &elements)?
        }),
        Tuple { elements } => node!(vm, Tuple, {
            elts => expressions_to_ast(vm, &elements)?
        }),
        Set { elements } => node!(vm, Set, {
            elts => expressions_to_ast(vm, &elements)?
        }),
        Dict { elements } => {
            let mut keys = Vec::new();
            let mut values = Vec::new();
            for (k, v) in elements {
                let k = k.as_ref().map(|k| expression_to_ast(vm, k)).transpose()?;
                keys.push(k.into_pyobject(vm));
                values.push(expression_to_ast(vm, v)?.into_object());
            }

            node!(vm, Dict, {
                keys => vm.ctx.new_list(keys),
                values => vm.ctx.new_list(values),
            })
        }
        Comprehension { kind, generators } => {
            let py_generators = map_ast(comprehension_to_ast, vm, generators)?;

            match kind.deref() {
                ast::ComprehensionKind::GeneratorExpression { element } => {
                    node!(vm, GeneratorExp, {
                        elt => expression_to_ast(vm, element)?,
                        generators => py_generators
                    })
                }
                ast::ComprehensionKind::List { element } => node!(vm, ListComp, {
                    elt => expression_to_ast(vm, element)?,
                    generators => py_generators
                }),
                ast::ComprehensionKind::Set { element } => node!(vm, SetComp, {
                    elt => expression_to_ast(vm, element)?,
                    generators => py_generators
                }),
                ast::ComprehensionKind::Dict { key, value } => node!(vm, DictComp, {
                    key => expression_to_ast(vm, key)?,
                    value => expression_to_ast(vm, value)?,
                    generators => py_generators
                }),
            }
        }
        Await { value } => {
            let py_value = expression_to_ast(vm, value)?;
            node!(vm, Await, {
                value => py_value
            })
        }
        Yield { value } => {
            let py_value = value
                .as_ref()
                .map(|v| expression_to_ast(vm, v))
                .transpose()?;
            node!(vm, Yield, {
                value => py_value.into_pyobject(vm)
            })
        }
        YieldFrom { value } => {
            let py_value = expression_to_ast(vm, value)?;
            node!(vm, YieldFrom, {
                value => py_value
            })
        }
        Subscript { a, b } => node!(vm, Subscript, {
            value => expression_to_ast(vm, a)?,
            slice => expression_to_ast(vm, b)?,
        }),
        Attribute { value, name } => node!(vm, Attribute, {
            value => expression_to_ast(vm, value)?,
            attr => vm.ctx.new_str(name),
            ctx => vm.ctx.none()
        }),
        Starred { value } => node!(vm, Starred, {
            value => expression_to_ast(vm, value)?
        }),
        Slice { elements } => node!(vm, Slice, {
            bounds => expressions_to_ast(vm, elements)?
        }),
        String { value } => string_to_ast(vm, value)?,
        Bytes { value } => node!(vm, Bytes, { s => vm.ctx.new_bytes(value.clone()) }),
        NamedExpression { left, right } => {
            node!(vm, NamedExpression, { left => expression_to_ast(vm, left)?, right => expression_to_ast(vm, right)? })
        }
    };

    let lineno = vm.ctx.new_int(expression.location.row());
    vm.set_attr(node.as_object(), "lineno", lineno).unwrap();
    Ok(node)
}

fn operator_string(op: &ast::Operator) -> String {
    use ast::Operator::*;
    match op {
        Add => "Add",
        Sub => "Sub",
        Mult => "Mult",
        MatMult => "MatMult",
        Div => "Div",
        Mod => "Mod",
        Pow => "Pow",
        LShift => "LShift",
        RShift => "RShift",
        BitOr => "BitOr",
        BitXor => "BitXor",
        BitAnd => "BitAnd",
        FloorDiv => "FloorDiv",
    }
    .to_owned()
}

fn parameters_to_ast(vm: &VirtualMachine, args: &ast::Parameters) -> PyResult<AstNodeRef> {
    Ok(node!(vm, arguments, {
        args => map_ast(parameter_to_ast, vm, &args.args)?,
        vararg => vararg_to_ast(vm, &args.vararg)?,
        kwonlyargs => map_ast(parameter_to_ast, vm, &args.kwonlyargs)?,
        kw_defaults => optional_expressions_to_ast(vm, &args.kw_defaults)?,
        kwarg => vararg_to_ast(vm, &args.kwarg)?,
        defaults => expressions_to_ast(vm, &args.defaults)?
    }))
}

fn vararg_to_ast(vm: &VirtualMachine, vararg: &ast::Varargs) -> PyResult {
    let py_node = match vararg {
        ast::Varargs::None => vm.ctx.none(),
        ast::Varargs::Unnamed => vm.ctx.none(),
        ast::Varargs::Named(parameter) => parameter_to_ast(vm, parameter)?.into_object(),
    };
    Ok(py_node)
}

fn parameter_to_ast(vm: &VirtualMachine, parameter: &ast::Parameter) -> PyResult<AstNodeRef> {
    let py_annotation = parameter
        .annotation
        .as_ref()
        .map(|expr| expression_to_ast(vm, expr))
        .transpose()?
        .into_pyobject(vm);
    let py_node = node!(vm, arg, {
        arg => vm.ctx.new_str(&parameter.arg),
        annotation => py_annotation
    });

    let lineno = vm.ctx.new_int(parameter.location.row());
    vm.set_attr(py_node.as_object(), "lineno", lineno)?;

    Ok(py_node)
}

fn keyword_to_ast(vm: &VirtualMachine, keyword: &ast::Keyword) -> PyResult<AstNodeRef> {
    Ok(node!(vm, keyword, {
        arg => keyword.name.as_ref().into_pyobject(vm),
        value => expression_to_ast(vm, &keyword.value)?
    }))
}

fn map_ast<T>(
    f: fn(vm: &VirtualMachine, &T) -> PyResult<AstNodeRef>,
    vm: &VirtualMachine,
    items: &[T],
) -> PyResult {
    let list: PyResult<Vec<PyObjectRef>> =
        items.iter().map(|x| Ok(f(vm, x)?.into_object())).collect();
    Ok(vm.ctx.new_list(list?))
}

fn comprehension_to_ast(
    vm: &VirtualMachine,
    comprehension: &ast::Comprehension,
) -> PyResult<AstNodeRef> {
    Ok(node!(vm, comprehension, {
        target => expression_to_ast(vm, &comprehension.target)?,
        iter => expression_to_ast(vm, &comprehension.iter)?,
        ifs => expressions_to_ast(vm, &comprehension.ifs)?,
        is_async => vm.ctx.new_bool(comprehension.is_async),
    }))
}

fn string_to_ast(vm: &VirtualMachine, string: &ast::StringGroup) -> PyResult<AstNodeRef> {
    let string = match string {
        ast::StringGroup::Constant { value } => node!(vm, Str, { s => vm.ctx.new_str(value) }),
        ast::StringGroup::FormattedValue { value, .. } => {
            node!(vm, FormattedValue, { value => expression_to_ast(vm, value)? })
        }
        ast::StringGroup::Joined { values } => {
            let py_values = map_ast(string_to_ast, vm, &values)?;
            node!(vm, JoinedStr, { values => py_values })
        }
    };
    Ok(string)
}

pub(crate) fn parse(vm: &VirtualMachine, source: &str, mode: Mode) -> PyResult {
    let ast = parser::parse(source, mode).map_err(|err| vm.new_value_error(format!("{}", err)))?;
    let py_body = top_to_ast(vm, &ast)?;
    Ok(node!(vm, Module, { body => py_body }).into_object())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let ast_base = AstNode::make_class(ctx);
    py_module!(vm, MODULE_NAME, {
        // TODO: There's got to be a better way!
        "alias" => py_class!(ctx, "alias", &ast_base, {}),
        "arg" => py_class!(ctx, "arg", &ast_base, {}),
        "arguments" => py_class!(ctx, "arguments", &ast_base, {}),
        "AnnAssign" => py_class!(ctx, "AnnAssign", &ast_base, {}),
        "Assign" => py_class!(ctx, "Assign", &ast_base, {}),
        "AugAssign" => py_class!(ctx, "AugAssign", &ast_base, {}),
        "AsyncFor" => py_class!(ctx, "AsyncFor", &ast_base, {}),
        "AsyncFunctionDef" => py_class!(ctx, "AsyncFunctionDef", &ast_base, {}),
        "AsyncWith" => py_class!(ctx, "AsyncWith", &ast_base, {}),
        "Assert" => py_class!(ctx, "Assert", &ast_base, {}),
        "Attribute" => py_class!(ctx, "Attribute", &ast_base, {}),
        "Await" => py_class!(ctx, "Await", &ast_base, {}),
        "BinOp" => py_class!(ctx, "BinOp", &ast_base, {}),
        "BoolOp" => py_class!(ctx, "BoolOp", &ast_base, {}),
        "Break" => py_class!(ctx, "Break", &ast_base, {}),
        "Bytes" => py_class!(ctx, "Bytes", &ast_base, {}),
        "Call" => py_class!(ctx, "Call", &ast_base, {}),
        "ClassDef" => py_class!(ctx, "ClassDef", &ast_base, {}),
        "Compare" => py_class!(ctx, "Compare", &ast_base, {}),
        "comprehension" => py_class!(ctx, "comprehension", &ast_base, {}),
        "Continue" => py_class!(ctx, "Continue", &ast_base, {}),
        "Delete" => py_class!(ctx, "Delete", &ast_base, {}),
        "Dict" => py_class!(ctx, "Dict", &ast_base, {}),
        "DictComp" => py_class!(ctx, "DictComp", &ast_base, {}),
        "Ellipsis" => py_class!(ctx, "Ellipsis", &ast_base, {}),
        "Expr" => py_class!(ctx, "Expr", &ast_base, {}),
        "ExceptHandler" => py_class!(ctx, "ExceptHandler", &ast_base, {}),
        "For" => py_class!(ctx, "For", &ast_base, {}),
        "FormattedValue" => py_class!(ctx, "FormattedValue", &ast_base, {}),
        "FunctionDef" => py_class!(ctx, "FunctionDef", &ast_base, {}),
        "GeneratorExp" => py_class!(ctx, "GeneratorExp", &ast_base, {}),
        "Global" => py_class!(ctx, "Global", &ast_base, {}),
        "If" => py_class!(ctx, "If", &ast_base, {}),
        "IfExp" => py_class!(ctx, "IfExp", &ast_base, {}),
        "Import" => py_class!(ctx, "Import", &ast_base, {}),
        "ImportFrom" => py_class!(ctx, "ImportFrom", &ast_base, {}),
        "JoinedStr" => py_class!(ctx, "JoinedStr", &ast_base, {}),
        "keyword" => py_class!(ctx, "keyword", &ast_base, {}),
        "Lambda" => py_class!(ctx, "Lambda", &ast_base, {}),
        "List" => py_class!(ctx, "List", &ast_base, {}),
        "ListComp" => py_class!(ctx, "ListComp", &ast_base, {}),
        "Module" => py_class!(ctx, "Module", &ast_base, {}),
        "Name" => py_class!(ctx, "Name", &ast_base, {}),
        "NameConstant" => py_class!(ctx, "NameConstant", &ast_base, {}),
        "NamedExpression" => py_class!(ctx, "NamedExpression", &ast_base, {}),
        "Nonlocal" => py_class!(ctx, "Nonlocal", &ast_base, {}),
        "Num" => py_class!(ctx, "Num", &ast_base, {}),
        "Pass" => py_class!(ctx, "Pass", &ast_base, {}),
        "Raise" => py_class!(ctx, "Raise", &ast_base, {}),
        "Return" => py_class!(ctx, "Return", &ast_base, {}),
        "Set" => py_class!(ctx, "Set", &ast_base, {}),
        "SetComp" => py_class!(ctx, "SetComp", &ast_base, {}),
        "Starred" => py_class!(ctx, "Starred", &ast_base, {}),
        "Starred" => py_class!(ctx, "Starred", &ast_base, {}),
        "Str" => py_class!(ctx, "Str", &ast_base, {}),
        "Subscript" => py_class!(ctx, "Subscript", &ast_base, {}),
        "Try" => py_class!(ctx, "Try", &ast_base, {}),
        "Tuple" => py_class!(ctx, "Tuple", &ast_base, {}),
        "UnaryOp" => py_class!(ctx, "UnaryOp", &ast_base, {}),
        "While" => py_class!(ctx, "While", &ast_base, {}),
        "With" => py_class!(ctx, "With", &ast_base, {}),
        "withitem" => py_class!(ctx, "withitem", &ast_base, {}),
        "Yield" => py_class!(ctx, "Yield", &ast_base, {}),
        "YieldFrom" => py_class!(ctx, "YieldFrom", &ast_base, {}),
        "AST" => ast_base,
        "PyCF_ONLY_AST" => ctx.new_int(PY_COMPILE_FLAG_AST_ONLY),
    })
}
