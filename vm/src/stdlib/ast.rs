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

fn statement_to_ast(ctx: &PyContext, statement: &ast::LocatedStatement) -> PyObjectRef {
    let node = match &statement.node {
        ast::Statement::FunctionDef {
            name,
            args: _,
            body,
        } => {
            // TODO: create ast.FunctionDef object:
            let node = ctx.new_object();

            // Set name:
            node.set_attr("name", ctx.new_str(name.to_string()));

            // Set body:
            let mut py_body = vec![];
            for statement in body {
                py_body.push(statement_to_ast(ctx, statement));
            }

            node.set_attr("body", ctx.new_list(py_body));
            node
        }
        ast::Statement::Expression { expression } => {
            let value = expression_to_ast(ctx, expression);
            // TODO: create proper class:
            let node = ctx.new_object();
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

fn expression_to_ast(ctx: &PyContext, expression: &ast::Expression) -> PyObjectRef {
    let node = match &expression {
        ast::Expression::Call { function, args } => {
            // TODO: create ast.Call instance
            let node = ctx.new_object();

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
        ast::Expression::Identifier { name } => {
            // TODO: create ast.Identifier instance
            let node = ctx.new_object();
            let py_name = ctx.new_str(name.clone());
            node.set_attr("id", py_name);
            node
        }
        ast::Expression::String { value } => {
            let node = ctx.new_object();
            node.set_attr("s", ctx.new_str(value.clone()));
            node
        }
        n => {
            unimplemented!("{:?}", n);
        }
    };

    // TODO: set lineno on object
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
