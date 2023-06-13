//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

mod gen;

use crate::{
    builtins::{self, PyDict, PyModule, PyStrRef, PyType},
    class::{PyClassImpl, StaticType},
    compiler::core::bytecode::OpArgType,
    compiler::CompileError,
    convert::ToPyException,
    source_code::{OneIndexed, SourceLocation, SourceLocator, SourceRange},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
    VirtualMachine,
};
use num_complex::Complex64;
use num_traits::{ToPrimitive, Zero};
use rustpython_ast::{self as ast, fold::Fold};
#[cfg(feature = "rustpython-codegen")]
use rustpython_codegen as codegen;
#[cfg(feature = "rustpython-parser")]
use rustpython_parser as parser;

#[pymodule]
mod _ast {
    use crate::{
        builtins::{PyStrRef, PyTupleRef},
        function::FuncArgs,
        AsObject, Context, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    };
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[pyslot]
        #[pymethod(magic)]
        fn init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let fields = zelf.get_attr("_fields", vm)?;
            let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
            let numargs = args.args.len();
            if numargs > fields.len() {
                return Err(vm.new_type_error(format!(
                    "{} constructor takes at most {} positional argument{}",
                    zelf.class().name(),
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                )));
            }
            for (name, arg) in fields.iter().zip(args.args) {
                zelf.set_attr(name, arg, vm)?;
            }
            for (key, value) in args.kwargs {
                if let Some(pos) = fields.iter().position(|f| f.as_str() == key) {
                    if pos < numargs {
                        return Err(vm.new_type_error(format!(
                            "{} got multiple values for argument '{}'",
                            zelf.class().name(),
                            key
                        )));
                    }
                }
                zelf.set_attr(vm.ctx.intern_str(key), value, vm)?;
            }
            Ok(())
        }

        #[pyattr(name = "_fields")]
        fn fields(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }
    }

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_COMPILE_FLAG_AST_ONLY;
}

fn get_node_field(vm: &VirtualMachine, obj: &PyObject, field: &'static str, typ: &str) -> PyResult {
    vm.get_attribute_opt(obj.to_owned(), field)?
        .ok_or_else(|| vm.new_type_error(format!("required field \"{field}\" missing from {typ}")))
}

fn get_node_field_opt(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
) -> PyResult<Option<PyObjectRef>> {
    Ok(vm
        .get_attribute_opt(obj.to_owned(), field)?
        .filter(|obj| !vm.is_none(obj)))
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
        vm.ctx
            .new_list(
                self.into_iter()
                    .map(|node| node.ast_to_object(vm))
                    .collect(),
            )
            .into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        vm.extract_elements_with(&object, |obj| Node::ast_from_object(vm, obj))
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

fn range_from_object(
    vm: &VirtualMachine,
    object: PyObjectRef,
    name: &str,
) -> PyResult<SourceRange> {
    fn make_location(row: u32, column: u32) -> Option<SourceLocation> {
        Some(SourceLocation {
            row: OneIndexed::new(row)?,
            column: OneIndexed::from_zero_indexed(column),
        })
    }
    let row = ast::Int::ast_from_object(vm, get_node_field(vm, &object, "lineno", name)?)?;
    let column = ast::Int::ast_from_object(vm, get_node_field(vm, &object, "col_offset", name)?)?;
    let location = make_location(row.to_u32(), column.to_u32());
    let end_row = get_node_field_opt(vm, &object, "end_lineno")?
        .map(|obj| ast::Int::ast_from_object(vm, obj))
        .transpose()?;
    let end_column = get_node_field_opt(vm, &object, "end_col_offset")?
        .map(|obj| ast::Int::ast_from_object(vm, obj))
        .transpose()?;
    let end_location = if let (Some(row), Some(column)) = (end_row, end_column) {
        make_location(row.to_u32(), column.to_u32())
    } else {
        None
    };
    let range = SourceRange {
        start: location.unwrap_or(SourceLocation::default()),
        end: end_location,
    };
    Ok(range)
}

fn node_add_location(dict: &Py<PyDict>, range: SourceRange, vm: &VirtualMachine) {
    dict.set_item("lineno", vm.ctx.new_int(range.start.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "col_offset",
        vm.ctx.new_int(range.start.column.to_zero_indexed()).into(),
        vm,
    )
    .unwrap();
    if let Some(end_location) = range.end {
        dict.set_item(
            "end_lineno",
            vm.ctx.new_int(end_location.row.get()).into(),
            vm,
        )
        .unwrap();
        dict.set_item(
            "end_col_offset",
            vm.ctx.new_int(end_location.column.to_zero_indexed()).into(),
            vm,
        )
        .unwrap();
    };
}

impl Node for ast::String {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(self).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let py_str = PyStrRef::try_from_object(vm, object)?;
        Ok(py_str.as_str().to_owned())
    }
}

impl Node for ast::Identifier {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let id: String = self.into();
        vm.ctx.new_str(id).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let py_str = PyStrRef::try_from_object(vm, object)?;
        Ok(ast::Identifier::new(py_str.as_str()))
    }
}

impl Node for ast::Int {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self.to_u32()).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let value = object.try_into_value(vm)?;
        Ok(ast::Int::new(value))
    }
}

impl Node for bool {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object).map(|i| i != 0)
    }
}

impl Node for ast::Constant {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ast::Constant::None => vm.ctx.none(),
            ast::Constant::Bool(b) => vm.ctx.new_bool(b).into(),
            ast::Constant::Str(s) => vm.ctx.new_str(s).into(),
            ast::Constant::Bytes(b) => vm.ctx.new_bytes(b).into(),
            ast::Constant::Int(i) => vm.ctx.new_int(i).into(),
            ast::Constant::Tuple(t) => vm
                .ctx
                .new_tuple(t.into_iter().map(|c| c.ast_to_object(vm)).collect())
                .into(),
            ast::Constant::Float(f) => vm.ctx.new_float(f).into(),
            ast::Constant::Complex { real, imag } => vm.new_pyobj(Complex64::new(real, imag)),
            ast::Constant::Ellipsis => vm.ctx.ellipsis(),
        }
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let constant = match_class!(match object {
            ref i @ builtins::int::PyInt => {
                let value = i.as_bigint();
                if object.class().is(vm.ctx.types.bool_type) {
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
            ref s @ builtins::pystr::PyStr => ast::Constant::Str(s.as_str().to_owned()),
            ref b @ builtins::bytes::PyBytes => ast::Constant::Bytes(b.as_bytes().to_owned()),
            ref t @ builtins::tuple::PyTuple => {
                ast::Constant::Tuple(
                    t.iter()
                        .map(|elt| Self::ast_from_object(vm, elt.clone()))
                        .collect::<Result<_, _>>()?,
                )
            }
            builtins::singletons::PyNone => ast::Constant::None,
            builtins::slice::PyEllipsis => ast::Constant::Ellipsis,
            obj =>
                return Err(vm.new_type_error(format!(
                    "invalid type in Constant: type '{}'",
                    obj.class().name()
                ))),
        });
        Ok(constant)
    }
}

impl Node for ast::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object)?
            .to_u32()
            .and_then(ast::ConversionFlag::from_op_arg)
            .ok_or_else(|| vm.new_value_error("invalid conversion flag".to_owned()))
    }
}

#[cfg(feature = "rustpython-parser")]
pub(crate) fn parse(
    vm: &VirtualMachine,
    source: &str,
    mode: parser::Mode,
) -> Result<PyObjectRef, CompileError> {
    let mut locator = SourceLocator::new(source);
    let top = parser::parse(source, mode, "<unknown>").map_err(|e| locator.locate_error(e))?;
    let top = locator.fold_mod(top).unwrap();
    Ok(top.ast_to_object(vm))
}

#[cfg(feature = "rustpython-codegen")]
pub(crate) fn compile(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    mode: crate::compiler::Mode,
) -> PyResult {
    let opts = vm.compile_opts();
    let ast = Node::ast_from_object(vm, object)?;
    let code = codegen::compile::compile_top(&ast, filename.to_owned(), mode, opts)
        .map_err(|err| (CompileError::from(err), None).to_pyexception(vm))?; // FIXME source
    Ok(vm.ctx.new_code(code).into())
}

// Required crate visibility for inclusion by gen.rs
pub(crate) use _ast::NodeAst;
// Used by builtins::compile()
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ast::make_module(vm);
    gen::extend_module_nodes(vm, &module);
    module
}
