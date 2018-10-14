/*
 * Ast standard module
 *
 * This module makes use of the parser logic, and translates all ast nodes
 * into python ast.AST objects.
 */

extern crate rustpython_parser;

use self::rustpython_parser::{ast, parser};
use super::super::obj::{objstr, objtype};
use super::super::pyobject::{
    AttributeProtocol, DictProtocol, PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

/*
 * Idea: maybe we can create a sort of struct with some helper functions?
struct AstToPyAst {
    ctx: &PyContext,
}

impl AstToPyAst {
    fn new(ctx: &PyContext) -> Self {
        AstToPyAst {
            ctx: ctx,
        }
    }

}
*/

fn program_to_ast(ctx: &PyContext, program: &ast::Program) -> PyObjectRef {
    let mut body = vec![];
    for statement in &program.statements {
        body.push(statement_to_ast(ctx, statement));
    }
    // TODO: create Module node:
    // let ast_node = ctx.new_instance(this.Module);
    let ast_node = ctx.new_object();
    let py_body = ctx.new_list(body);
    ast_node.set_attr("body", py_body);
    ast_node
}

// Create a node class instance
fn create_node(ctx: &PyContext, _name: &str) -> PyObjectRef {
    // TODO: instantiate a class of type given by name
    // TODO: lookup in the current module?
    let node = ctx.new_object();
    node
}

fn statements_to_ast(ctx: &PyContext, statements: &[ast::LocatedStatement]) -> PyObjectRef {
    let mut py_statements = vec![];
    for statement in statements {
        py_statements.push(statement_to_ast(ctx, statement));
    }
    ctx.new_list(py_statements)
}

fn statement_to_ast(ctx: &PyContext, statement: &ast::LocatedStatement) -> PyObjectRef {
    let node = match &statement.node {
        ast::Statement::ClassDef {
            name,
            body,
            args: _,
            decorator_list,
        } => {
            let node = create_node(ctx, "ClassDef");

            // Set name:
            node.set_attr("name", ctx.new_str(name.to_string()));

            // Set body:
            let py_body = statements_to_ast(ctx, body);
            node.set_attr("body", py_body);

            let py_decorator_list = expressions_to_ast(ctx, decorator_list);
            node.set_attr("decorator_list", py_decorator_list);
            node
        }
        ast::Statement::FunctionDef {
            name,
            args: _,
            body,
            decorator_list,
        } => {
            let node = create_node(ctx, "FunctionDef");

            // Set name:
            node.set_attr("name", ctx.new_str(name.to_string()));

            // Set body:
            let py_body = statements_to_ast(ctx, body);
            node.set_attr("body", py_body);

            let py_decorator_list = expressions_to_ast(ctx, decorator_list);
            node.set_attr("decorator_list", py_decorator_list);
            node
        }
        ast::Statement::Continue => {
            let node = create_node(ctx, "Continue");
            node
        }
        ast::Statement::Break => {
            let node = create_node(ctx, "Break");
            node
        }
        ast::Statement::Pass => {
            let node = create_node(ctx, "Pass");
            node
        }
        ast::Statement::Delete { targets } => {
            let node = create_node(ctx, "Delete");

            let py_targets = ctx.new_tuple(
                targets
                    .into_iter()
                    .map(|v| expression_to_ast(ctx, v))
                    .collect(),
            );
            node.set_attr("targets", py_targets);

            node
        }
        ast::Statement::Return { value } => {
            let node = create_node(ctx, "Return");

            let py_value = if let Some(value) = value {
                ctx.new_tuple(
                    value
                        .into_iter()
                        .map(|v| expression_to_ast(ctx, v))
                        .collect(),
                )
            } else {
                ctx.none()
            };
            node.set_attr("value", py_value);

            node
        }
        ast::Statement::If { test, body, orelse } => {
            let node = create_node(ctx, "If");

            let py_test = expression_to_ast(ctx, test);
            node.set_attr("test", py_test);

            let py_body = statements_to_ast(ctx, body);
            node.set_attr("body", py_body);

            let py_orelse = if let Some(orelse) = orelse {
                statements_to_ast(ctx, orelse)
            } else {
                ctx.none()
            };
            node.set_attr("orelse", py_orelse);

            node
        }
        ast::Statement::For {
            target: _,
            iter: _,
            body,
            orelse,
        } => {
            let node = create_node(ctx, "For");

            /*
            let py_target = expression_to_ast(ctx, target);
            node.set_attr("target", py_target);
            
            let py_iter = expression_to_ast(ctx, iter);
            node.set_attr("iter", py_iter);
            */

            let py_body = statements_to_ast(ctx, body);
            node.set_attr("body", py_body);

            let py_orelse = if let Some(orelse) = orelse {
                statements_to_ast(ctx, orelse)
            } else {
                ctx.none()
            };
            node.set_attr("orelse", py_orelse);

            node
        }
        ast::Statement::While { test, body, orelse } => {
            let node = create_node(ctx, "While");

            let py_test = expression_to_ast(ctx, test);
            node.set_attr("test", py_test);

            let py_body = statements_to_ast(ctx, body);
            node.set_attr("body", py_body);

            let py_orelse = if let Some(orelse) = orelse {
                statements_to_ast(ctx, orelse)
            } else {
                ctx.none()
            };
            node.set_attr("orelse", py_orelse);

            node
        }
        ast::Statement::Expression { expression } => {
            let node = create_node(ctx, "Expr");

            let value = expression_to_ast(ctx, expression);
            node.set_attr("value", value);

            node
        }
        x => {
            unimplemented!("{:?}", x);
        }
    };

    // set lineno on node:
    let lineno = ctx.new_int(statement.location.get_row() as i32);
    node.set_attr("lineno", lineno);

    node
}

fn expressions_to_ast(ctx: &PyContext, expressions: &Vec<ast::Expression>) -> PyObjectRef {
    let mut py_expression_nodes = vec![];
    for expression in expressions {
        py_expression_nodes.push(expression_to_ast(ctx, expression));
    }
    ctx.new_list(py_expression_nodes)
}

fn expression_to_ast(ctx: &PyContext, expression: &ast::Expression) -> PyObjectRef {
    let node = match &expression {
        ast::Expression::Call { function, args } => {
            let node = create_node(ctx, "Call");

            let py_func_ast = expression_to_ast(ctx, function);
            node.set_attr("func", py_func_ast);

            let mut py_args = vec![];
            for arg in args {
                py_args.push(expression_to_ast(ctx, &arg.1));
            }
            let py_args = ctx.new_list(py_args);
            node.set_attr("args", py_args);

            node
        }
        ast::Expression::Binop { a, op, b } => {
            let node = create_node(ctx, "BinOp");

            let py_a = expression_to_ast(ctx, a);
            node.set_attr("left", py_a);

            // Operator:
            let str_op = match op {
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
            let py_op = ctx.new_str(str_op.to_string());
            node.set_attr("op", py_op);

            let py_b = expression_to_ast(ctx, b);
            node.set_attr("right", py_b);
            node
        }
        ast::Expression::Identifier { name } => {
            let node = create_node(ctx, "Identifier");

            // Id:
            let py_name = ctx.new_str(name.clone());
            node.set_attr("id", py_name);
            node
        }
        ast::Expression::String { value } => {
            let node = create_node(ctx, "Str");
            node.set_attr("s", ctx.new_str(value.clone()));
            node
        }
        n => {
            unimplemented!("{:?}", n);
        }
    };

    // TODO: retrieve correct lineno:
    let lineno = ctx.new_int(1);
    node.set_attr("lineno", lineno);

    node
}

fn ast_parse(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(source, Some(vm.ctx.str_type()))]);

    let source_string = objstr::get_value(source);
    let internal_ast = match parser::parse_program(&source_string) {
        Ok(ast) => ast,
        Err(msg) => {
            return Err(vm.new_value_error(msg));
        }
    };

    // source.clone();
    let ast_node = program_to_ast(&vm.ctx, &internal_ast);
    Ok(ast_node)
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let ast_mod = ctx.new_module(&"ast".to_string(), ctx.new_scope(None));
    ast_mod.set_item("parse", ctx.new_rustfunc(ast_parse));
    ast_mod.set_item(
        "Module",
        ctx.new_class(&"_ast.Module".to_string(), ctx.object()),
    );

    // TODO: maybe we can use some clever macro to generate this?
    ast_mod.set_item(
        "FunctionDef",
        ctx.new_class(&"_ast.FunctionDef".to_string(), ctx.object()),
    );
    ast_mod.set_item(
        "Call",
        ctx.new_class(&"_ast.Call".to_string(), ctx.object()),
    );
    ast_mod
}
