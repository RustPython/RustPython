//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

use num_complex::Complex64;
use num_traits::{ToPrimitive, Zero};

use rustpython_ast as ast;

#[cfg(feature = "rustpython-parser")]
use rustpython_parser::parser;

#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler as compile;

use crate::builtins::{self, PyStrRef, PyTypeRef};
use crate::function::FuncArgs;
use crate::pyobject::{
    BorrowValue, IdProtocol, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyResult, PyValue,
    StaticType, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[rustfmt::skip]
#[allow(clippy::all)]
mod gen;


fn get_node_field(vm: &VirtualMachine, obj: &PyObjectRef, field: &str, typ: &str) -> PyResult {
    vm.get_attribute_opt(obj.clone(), field)?.ok_or_else(|| {
        vm.new_type_error(format!("required field \"{}\" missing from {}", field, typ))
    })
}

fn get_node_field_opt(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    field: &str,
) -> PyResult<Option<PyObjectRef>> {
    Ok(vm
        .get_attribute_opt(obj.clone(), field)?
        .filter(|obj| !vm.is_none(obj)))
}

#[pyclass(module = "_ast", name = "AST")]
#[derive(Debug)]
pub(crate) struct AstNode;

#[pyimpl(flags(HAS_DICT))]
impl AstNode {
    #[pymethod(magic)]
    fn init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let fields = vm.get_attribute(zelf.clone_class().into_object(), "_fields")?;
        let fields = vm.extract_elements::<PyStrRef>(&fields)?;
        let numargs = args.args.len();
        if numargs > fields.len() {
            return Err(vm.new_type_error(format!(
                "{} constructor takes at most {} positional argument{}",
                zelf.class().name,
                fields.len(),
                if fields.len() == 1 { "" } else { "s" },
            )));
        }
        for (name, arg) in fields.iter().zip(args.args) {
            vm.set_attr(&zelf, name.clone(), arg)?;
        }
        for (key, value) in args.kwargs {
            if let Some(pos) = fields.iter().position(|f| f.borrow_value() == key) {
                if pos < numargs {
                    return Err(vm.new_type_error(format!(
                        "{} got multiple values for argument '{}'",
                        zelf.class().name,
                        key
                    )));
                }
            }
            vm.set_attr(&zelf, key, value)?;
        }
        Ok(())
    }
}

const MODULE_NAME: &str = "_ast";
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

impl PyValue for AstNode {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

trait Node: Sized {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef;
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self>;
}

trait NamedNode: Node {
    const NAME: &'static str;
}

impl<T: Node> Node for Vec<T> {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_list(
            self.into_iter()
                .map(|node| node.ast_to_object(vm))
                .collect(),
        )
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        vm.extract_elements_func(&object, |obj| Node::ast_from_object(vm, obj))
    }
}

impl<T: Node> Node for Box<T> {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        (*self).ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        T::ast_from_object(vm, object).map(Box::new)
    }
}

impl<T: Node> Node for Option<T> {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Some(node) => node.ast_to_object(vm),
            None => vm.ctx.none(),
        }
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        if vm.is_none(&object) {
            Ok(None)
        } else {
            Ok(Some(T::ast_from_object(vm, object)?))
        }
    }
}

impl<T: NamedNode> Node for ast::Located<T> {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let obj = self.node.ast_to_object(vm);
        node_add_location(&obj, self.location, vm);
        obj
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let location = ast::Location::new(
            Node::ast_from_object(vm, get_node_field(vm, &object, "lineno", T::NAME)?)?,
            Node::ast_from_object(vm, get_node_field(vm, &object, "col_offset", T::NAME)?)?,
        );
        let node = T::ast_from_object(vm, object)?;
        Ok(ast::Located::new(location, node))
    }
}

fn node_add_location(node: &PyObjectRef, location: ast::Location, vm: &VirtualMachine) {
    let dict = node.dict().unwrap();
    dict.set_item("lineno", vm.ctx.new_int(location.row()), vm)
        .unwrap();
    dict.set_item("col_offset", vm.ctx.new_int(location.column()), vm)
        .unwrap();
}

impl Node for String {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        PyStrRef::try_from_object(vm, object).map(|s| s.borrow_value().to_owned())
    }
}

impl Node for usize {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Self::try_from_object(vm, object)
    }
}

impl Node for bool {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object).map(|i| i != 0)
    }
}

impl Node for ast::Constant {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ast::Constant::None => vm.ctx.none(),
            ast::Constant::Bool(b) => vm.ctx.new_bool(b),
            ast::Constant::Str(s) => vm.ctx.new_str(s),
            ast::Constant::Bytes(b) => vm.ctx.new_bytes(b),
            ast::Constant::Int(i) => vm.ctx.new_int(i),
            ast::Constant::Tuple(t) => vm
                .ctx
                .new_tuple(t.into_iter().map(|c| c.ast_to_object(vm)).collect()),
            ast::Constant::Float(f) => vm.ctx.new_float(f),
            ast::Constant::Complex { real, imag } => vm.ctx.new_complex(Complex64::new(real, imag)),
            ast::Constant::Ellipsis => vm.ctx.ellipsis(),
        }
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let constant = match_class!(match object {
            ref i @ builtins::int::PyInt => {
                let value = i.borrow_value();
                if object.class().is(&vm.ctx.types.bool_type) {
                    ast::Constant::Bool(!value.is_zero())
                } else {
                    ast::Constant::Int(value.clone())
                }
            }
            ref f @ builtins::float::PyFloat => ast::Constant::Float(f.to_f64()),
            ref c @ builtins::complex::PyComplex => {
                let c = c.to_complex();
                ast::Constant::Complex {
                    real: c.re,
                    imag: c.im,
                }
            }
            ref s @ builtins::pystr::PyStr => ast::Constant::Str(s.borrow_value().to_owned()),
            ref b @ builtins::bytes::PyBytes => ast::Constant::Bytes(b.borrow_value().to_owned()),
            ref t @ builtins::tuple::PyTuple => {
                ast::Constant::Tuple(
                    t.borrow_value()
                        .iter()
                        .map(|elt| Self::ast_from_object(vm, elt.clone()))
                        .collect::<Result<_, _>>()?,
                )
            }
            builtins::singletons::PyNone => ast::Constant::None,
            builtins::slice::PyEllipsis => ast::Constant::Ellipsis,
            _ => return Err(vm.new_type_error("unsupported type for constant".to_owned())),
        });
        Ok(constant)
    }
}

impl Node for ast::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object)?
            .to_u8()
            .and_then(ast::ConversionFlag::try_from_byte)
            .ok_or_else(|| vm.new_value_error("invalid conversion flag".to_owned()))
    }
}

#[cfg(feature = "rustpython-parser")]
pub(crate) fn parse(vm: &VirtualMachine, source: &str, mode: parser::Mode) -> PyResult {
    // TODO: use vm.new_syntax_error()
    let top = parser::parse(source, mode).map_err(|err| vm.new_value_error(format!("{}", err)))?;
    Ok(top.ast_to_object(vm))
}

#[cfg(feature = "rustpython-compiler")]
pub(crate) fn compile(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    _mode: compile::Mode,
) -> PyResult {
    let opts = vm.compile_opts();
    let ast = Node::ast_from_object(vm, object)?;
    let code = rustpython_compiler_core::compile::compile_top(&ast, filename.to_owned(), opts)
        // TODO: use vm.new_syntax_error()
        .map_err(|err| vm.new_value_error(err.to_string()))?;
    Ok(vm.new_code_object(code).into_object())
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let ast_base = AstNode::make_class(ctx);
    let module = py_module!(vm, MODULE_NAME, {
        // TODO: There's got to be a better way!
        "AST" => ast_base,
        "PyCF_ONLY_AST" => ctx.new_int(PY_COMPILE_FLAG_AST_ONLY),
    });
    gen::extend_module_nodes(vm, &module);
    module
}
