/*
 * Ast standard module
 *
 * This module makes use of the parser logic, and translates all ast nodes
 * into python ast.AST objects.
 */

extern crate rustpython_parser;

use self::rustpython_parser::{ast, parser};
use super::super::obj::{objdict, objfloat, objint, objlist, objstr, objtuple, objtype};
use super::super::objbool;
use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

fn program_to_ast(ctx: &PyContext, program: &ast::Program) -> PyObjectRef {
    let mut body = vec![];
    for statement in &program.statements {
        body.push(statement_to_ast(ctx, statement));
    }
    // TODO: create Module node and set attributes:
    let ast_node = ctx.new_list(body);
    ast_node
}

fn statement_to_ast(ctx: &PyContext, statement: &ast::LocatedStatement) -> PyObjectRef {
    match &statement.node {
        ast::Statement::FunctionDef { name, args, body } => {
            // TODO: create ast.FunctionDef object and set attributes
            // let node = ctx.new_object();
            let mut py_body = vec![];
            py_body.push(ctx.new_str(format!("{:?}", name)));
            // node.set_attr("name", new_str(name));
            for statement in body {
                py_body.push(statement_to_ast(ctx, statement));
            }

            ctx.new_list(py_body)
        },
        _ => {
            ctx.new_str(format!("{:?}", statement))
        }
    }

    // TODO: set lineno on object
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
    ast_mod
}
