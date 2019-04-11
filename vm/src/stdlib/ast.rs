//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

use std::ops::Deref;

use num_complex::Complex64;

use rustpython_parser::{ast, parser};

use crate::obj::objlist::PyListRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
struct AstNode;
type AstNodeRef = PyRef<AstNode>;

impl PyValue for AstNode {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("ast", "AST")
    }
}

macro_rules! node {
    ( $vm: expr, $node_name:ident, { $($attr_name:ident => $attr_value:expr),* $(,)* }) => {
        {
        let node = create_node($vm, stringify!($node_name));
        $(
            $vm.set_attr(node.as_object(), stringify!($attr_name), $attr_value).unwrap();
        )*
        node
        }
    };
    ( $vm: expr, $node_name:ident) => {
        create_node($vm, stringify!($node_name))
    }
}

fn program_to_ast(vm: &VirtualMachine, program: &ast::Program) -> AstNodeRef {
    let py_body = statements_to_ast(vm, &program.statements);
    node!(vm, Module, { body => py_body })
}

// Create a node class instance
fn create_node(vm: &VirtualMachine, name: &str) -> AstNodeRef {
    AstNode
        .into_ref_with_type(vm, vm.class("ast", name))
        .unwrap()
}

fn statements_to_ast(vm: &VirtualMachine, statements: &[ast::LocatedStatement]) -> PyListRef {
    let body = statements
        .iter()
        .map(|statement| statement_to_ast(&vm, statement).into_object())
        .collect();
    vm.ctx.new_list(body).downcast().unwrap()
}

fn statement_to_ast(vm: &VirtualMachine, statement: &ast::LocatedStatement) -> AstNodeRef {
    let node = match &statement.node {
        ast::Statement::ClassDef {
            name,
            body,
            decorator_list,
            ..
        } => node!(vm, ClassDef, {
            name => vm.ctx.new_str(name.to_string()),
            body => statements_to_ast(vm, body),
            decorator_list => expressions_to_ast(vm, decorator_list),
        }),
        ast::Statement::FunctionDef {
            name,
            args,
            body,
            decorator_list,
            returns,
        } => {
            let py_returns = if let Some(hint) = returns {
                expression_to_ast(vm, hint).into_object()
            } else {
                vm.ctx.none()
            };
            node!(vm, FunctionDef, {
                name => vm.ctx.new_str(name.to_string()),
                args => parameters_to_ast(vm, args),
                body => statements_to_ast(vm, body),
                decorator_list => expressions_to_ast(vm, decorator_list),
                returns => py_returns
            })
        }
        ast::Statement::Continue => node!(vm, Continue),
        ast::Statement::Break => node!(vm, Break),
        ast::Statement::Pass => node!(vm, Pass),
        ast::Statement::Assert { test, msg } => {
            let py_msg = match msg {
                Some(msg) => expression_to_ast(vm, msg).into_object(),
                None => vm.ctx.none(),
            };
            node!(vm, Assert, {
                test => expression_to_ast(vm, test),
                msg => py_msg
            })
        }
        ast::Statement::Delete { targets } => {
            let py_targets = vm.ctx.new_tuple(
                targets
                    .iter()
                    .map(|v| expression_to_ast(vm, v).into_object())
                    .collect(),
            );

            node!(vm, Delete, { targets => py_targets })
        }
        ast::Statement::Return { value } => {
            let py_value = if let Some(value) = value {
                expression_to_ast(vm, value).into_object()
            } else {
                vm.ctx.none()
            };

            node!(vm, Return, {
                value => py_value
            })
        }
        ast::Statement::If { test, body, orelse } => node!(vm, If, {
            test => expression_to_ast(vm, test),
            body => statements_to_ast(vm, body),
            orelse => if let Some(orelse) = orelse {
                    statements_to_ast(vm, orelse).into_object()
                } else {
                    vm.ctx.none()
                }
        }),
        ast::Statement::For {
            target,
            iter,
            body,
            orelse,
        } => node!(vm, For, {
            target => expression_to_ast(vm, target),
            iter => expression_to_ast(vm, iter),
            body => statements_to_ast(vm, body),
            or_else => if let Some(orelse) = orelse {
                statements_to_ast(vm, orelse).into_object()
            } else {
                vm.ctx.none()
            }
        }),
        ast::Statement::While { test, body, orelse } => node!(vm, While, {
            test => expression_to_ast(vm, test),
            body => statements_to_ast(vm, body),
            orelse => if let Some(orelse) = orelse {
                statements_to_ast(vm, orelse).into_object()
            } else {
                vm.ctx.none()
            }
        }),
        ast::Statement::Expression { expression } => node!(vm, Expr, {
            value => expression_to_ast(vm, expression)
        }),
        x => {
            unimplemented!("{:?}", x);
        }
    };

    // set lineno on node:
    let lineno = vm.ctx.new_int(statement.location.get_row());
    vm.set_attr(node.as_object(), "lineno", lineno).unwrap();

    node
}

fn expressions_to_ast(vm: &VirtualMachine, expressions: &[ast::Expression]) -> PyListRef {
    let py_expression_nodes = expressions
        .iter()
        .map(|expression| expression_to_ast(vm, expression).into_object())
        .collect();
    vm.ctx.new_list(py_expression_nodes).downcast().unwrap()
}

fn expression_to_ast(vm: &VirtualMachine, expression: &ast::Expression) -> AstNodeRef {
    let node = match &expression {
        ast::Expression::Call { function, args, .. } => node!(vm, Call, {
            func => expression_to_ast(vm, function),
            args => expressions_to_ast(vm, args),
        }),
        ast::Expression::Binop { a, op, b } => {
            // Operator:
            let op = match op {
                ast::Operator::Add => "Add",
                ast::Operator::Sub => "Sub",
                ast::Operator::Mult => "Mult",
                ast::Operator::MatMult => "MatMult",
                ast::Operator::Div => "Div",
                ast::Operator::Mod => "Mod",
                ast::Operator::Pow => "Pow",
                ast::Operator::LShift => "LShift",
                ast::Operator::RShift => "RShift",
                ast::Operator::BitOr => "BitOr",
                ast::Operator::BitXor => "BitXor",
                ast::Operator::BitAnd => "BitAnd",
                ast::Operator::FloorDiv => "FloorDiv",
            };
            node!(vm, BinOp, {
                left => expression_to_ast(vm, a),
                op => vm.ctx.new_str(op.to_string()),
                right => expression_to_ast(vm, b),
            })
        }
        ast::Expression::Unop { op, a } => {
            let op = match op {
                ast::UnaryOperator::Not => "Not",
                ast::UnaryOperator::Inv => "Invert",
                ast::UnaryOperator::Neg => "USub",
                ast::UnaryOperator::Pos => "UAdd",
            };
            node!(vm, UnaryOp, {
                op => vm.ctx.new_str(op.to_string()),
                operand => expression_to_ast(vm, a),
            })
        }
        ast::Expression::BoolOp { a, op, b } => {
            // Attach values:
            let py_a = expression_to_ast(vm, a).into_object();
            let py_b = expression_to_ast(vm, b).into_object();
            let py_values = vm.ctx.new_tuple(vec![py_a, py_b]);

            let str_op = match op {
                ast::BooleanOperator::And => "And",
                ast::BooleanOperator::Or => "Or",
            };
            let py_op = vm.ctx.new_str(str_op.to_string());

            node!(vm, BoolOp, {
                op => py_op,
                values => py_values,
            })
        }
        ast::Expression::Compare { vals, ops } => {
            let py_a = expression_to_ast(vm, &vals[0]);

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
            let py_ops = vm.ctx.new_list(
                ops.iter()
                    .map(|x| vm.ctx.new_str(to_operator(x).to_string()))
                    .collect(),
            );

            let py_b = vm.ctx.new_list(
                vals.iter()
                    .skip(1)
                    .map(|x| expression_to_ast(vm, x).into_object())
                    .collect(),
            );
            node!(vm, Compare, {
                left => py_a,
                ops => py_ops,
                comparators => py_b,
            })
        }
        ast::Expression::Identifier { name } => node!(vm, Identifier, {
            id => vm.ctx.new_str(name.clone())
        }),
        ast::Expression::Lambda { args, body } => node!(vm, Lambda, {
            args => parameters_to_ast(vm, args),
            body => expression_to_ast(vm, body),
        }),
        ast::Expression::IfExpression { test, body, orelse } => node!(vm, IfExp, {
            text => expression_to_ast(vm, test),
            body => expression_to_ast(vm, body),
            or_else => expression_to_ast(vm, orelse),
        }),
        ast::Expression::Number { value } => {
            let py_n = match value {
                ast::Number::Integer { value } => vm.ctx.new_int(value.clone()),
                ast::Number::Float { value } => vm.ctx.new_float(*value),
                ast::Number::Complex { real, imag } => {
                    vm.ctx.new_complex(Complex64::new(*real, *imag))
                }
            };
            node!(vm, Num, {
                n => py_n
            })
        }
        ast::Expression::True => node!(vm, NameConstant, {
            value => vm.ctx.new_bool(true)
        }),
        ast::Expression::False => node!(vm, NameConstant, {
            value => vm.ctx.new_bool(false)
        }),
        ast::Expression::None => node!(vm, NameConstant, {
            value => vm.ctx.none()
        }),
        ast::Expression::Ellipsis => node!(vm, Ellipsis),
        ast::Expression::List { elements } => node!(vm, List, {
            elts => expressions_to_ast(vm, &elements)
        }),
        ast::Expression::Tuple { elements } => node!(vm, Tuple, {
            elts => expressions_to_ast(vm, &elements)
        }),
        ast::Expression::Set { elements } => node!(vm, Set, {
            elts => expressions_to_ast(vm, &elements)
        }),
        ast::Expression::Dict { elements } => {
            let mut keys = Vec::new();
            let mut values = Vec::new();
            for (k, v) in elements {
                keys.push(expression_to_ast(vm, k).into_object());
                values.push(expression_to_ast(vm, v).into_object());
            }

            node!(vm, Dict, {
                keys => vm.ctx.new_list(keys),
                values => vm.ctx.new_list(values),
            })
        }
        ast::Expression::Comprehension { kind, generators } => {
            let g = generators
                .iter()
                .map(|g| comprehension_to_ast(vm, g).into_object())
                .collect();
            let py_generators = vm.ctx.new_list(g);

            match kind.deref() {
                ast::ComprehensionKind::GeneratorExpression { .. } => {
                    node!(vm, GeneratorExp, {generators => py_generators})
                }
                ast::ComprehensionKind::List { .. } => {
                    node!(vm, ListComp, {generators => py_generators})
                }
                ast::ComprehensionKind::Set { .. } => {
                    node!(vm, SetComp, {generators => py_generators})
                }
                ast::ComprehensionKind::Dict { .. } => {
                    node!(vm, DictComp, {generators => py_generators})
                }
            }
        }
        ast::Expression::Yield { value } => {
            let py_value = match value {
                Some(value) => expression_to_ast(vm, value).into_object(),
                None => vm.ctx.none(),
            };
            node!(vm, Yield, {
                value => py_value
            })
        }
        ast::Expression::YieldFrom { value } => {
            let py_value = expression_to_ast(vm, value);
            node!(vm, YieldFrom, {
                value => py_value
            })
        }
        ast::Expression::Subscript { a, b } => node!(vm, Subscript, {
            value => expression_to_ast(vm, a),
            slice => expression_to_ast(vm, b),
        }),
        ast::Expression::Attribute { value, name } => node!(vm, Attribute, {
            value => expression_to_ast(vm, value),
            attr => vm.ctx.new_str(name.to_string()),
        }),
        ast::Expression::Starred { value } => node!(vm, Starred, {
            value => expression_to_ast(vm, value)
        }),
        ast::Expression::Slice { elements } => node!(vm, Slice, {
            bounds => expressions_to_ast(vm, elements)
        }),
        ast::Expression::String { value } => string_to_ast(vm, value),
        ast::Expression::Bytes { value } => {
            node!(vm, Bytes, { s => vm.ctx.new_bytes(value.clone()) })
        }
    };

    // TODO: retrieve correct lineno:
    let lineno = vm.ctx.new_int(1);
    vm.set_attr(node.as_object(), "lineno", lineno).unwrap();
    node
}

fn parameters_to_ast(vm: &VirtualMachine, args: &ast::Parameters) -> AstNodeRef {
    let args = vm.ctx.new_list(
        args.args
            .iter()
            .map(|a| parameter_to_ast(vm, a).into_object())
            .collect(),
    );

    node!(vm, arguments, { args => args })
}

fn parameter_to_ast(vm: &VirtualMachine, parameter: &ast::Parameter) -> AstNodeRef {
    let py_annotation = if let Some(annotation) = &parameter.annotation {
        expression_to_ast(vm, annotation).into_object()
    } else {
        vm.ctx.none()
    };

    node!(vm, arg, {
        arg => vm.ctx.new_str(parameter.arg.to_string()),
        annotation => py_annotation
    })
}

fn comprehension_to_ast(vm: &VirtualMachine, comprehension: &ast::Comprehension) -> AstNodeRef {
    node!(vm, comprehension, {
        target => expression_to_ast(vm, &comprehension.target),
        iter => expression_to_ast(vm, &comprehension.iter),
        ifs => expressions_to_ast(vm, &comprehension.ifs),
    })
}

fn string_to_ast(vm: &VirtualMachine, string: &ast::StringGroup) -> AstNodeRef {
    match string {
        ast::StringGroup::Constant { value } => {
            node!(vm, Str, { s => vm.ctx.new_str(value.clone()) })
        }
        ast::StringGroup::FormattedValue { value, .. } => {
            node!(vm, FormattedValue, { value => expression_to_ast(vm, value) })
        }
        ast::StringGroup::Joined { values } => {
            let py_values = vm.ctx.new_list(
                values
                    .iter()
                    .map(|value| string_to_ast(vm, value).into_object())
                    .collect(),
            );
            node!(vm, JoinedStr, { values => py_values })
        }
    }
}

fn ast_parse(source: PyStringRef, vm: &VirtualMachine) -> PyResult<AstNodeRef> {
    let internal_ast = parser::parse_program(&source.value)
        .map_err(|err| vm.new_value_error(format!("{}", err)))?;
    // source.clone();
    Ok(program_to_ast(&vm, &internal_ast))
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let ast_base = py_class!(ctx, "_ast.AST", ctx.object(), {});
    py_module!(vm, "ast", {
        "parse" => ctx.new_rustfunc(ast_parse),
        "Module" => py_class!(ctx, "_ast.Module", ast_base.clone(), {}),
        "FunctionDef" => py_class!(ctx, "_ast.FunctionDef", ast_base.clone(), {}),
        "ClassDef" => py_class!(ctx, "_ast.ClassDef", ast_base.clone(), {}),
        "Call" => py_class!(ctx, "_ast.Call", ast_base.clone(), {}),
        "AST" => ast_base,
    })
}
