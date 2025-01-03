//! `ast` standard module for abstract syntax trees.
//!
//! This module makes use of the parser logic, and translates all ast nodes
//! into python ast.AST objects.

mod gen;

use crate::builtins::{PyInt, PyStr};
use crate::{
    builtins::PyIntRef,
    builtins::{self, PyDict, PyModule, PyStrRef, PyType},
    class::{PyClassImpl, StaticType},
    compiler::core::bytecode::OpArgType,
    compiler::CompileError,
    convert::ToPyException,
    convert::ToPyObject,
    source::SourceLocation,
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
    TryFromObject, VirtualMachine,
};
use malachite_bigint::BigInt;
use num_complex::Complex64;
use num_traits::{ToPrimitive, Zero};
use ruff_python_ast as ruff;
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustpython_codegen::compile;
use rustpython_compiler_source::SourceCode;

#[cfg(feature = "parser")]
use ruff_python_parser as parser;
#[cfg(feature = "codegen")]
use rustpython_codegen as codegen;

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

fn get_int_field(
    vm: &VirtualMachine,
    obj: &PyObject,
    field: &'static str,
) -> PyResult<Option<PyRefExact<PyInt>>> {
    Ok(get_node_field_opt(vm, &obj, field)?
        .map(|obj| obj.downcast_exact(vm))
        .transpose()
        .unwrap())
}

trait Node: Sized {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef;
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self>;
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

struct BoxedSlice<T>(Box<[T]>);

impl<T: Node> Node for BoxedSlice<T> {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        self.0.into_vec().ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self(
            <Vec<T> as Node>::ast_from_object(vm, object)?.into_boxed_slice(),
        ))
    }
}

struct SourceRange {
    start: SourceLocation,
    end: SourceLocation,
}

fn source_location_to_text_size(source_location: SourceLocation) -> TextSize {
    // TODO: Maybe implement this?
    TextSize::default()
}

fn text_range_to_source_range(text_range: TextRange) -> SourceRange {
    // TODO: Maybe implement this?
    SourceRange {
        start: SourceLocation::default(),
        end: SourceLocation::default(),
    }
}

fn range_from_object(vm: &VirtualMachine, object: PyObjectRef, name: &str) -> PyResult<TextRange> {
    fn make_location(row: PyIntRef, column: PyIntRef) -> Option<SourceLocation> {
        // TODO: Maybe implement this?
        // let row = row.to_u64().unwrap().try_into().unwrap();
        // let column = column.to_u64().unwrap().try_into().unwrap();
        // Some(SourceLocation {
        //     row: LineNumber::new(row)?,
        //     column: LineNumber::from_zero_indexed(column),
        // })

        None
    }

    let row = get_node_field(vm, &object, "lineno", name)?;
    let row = row.downcast_exact::<PyInt>(vm).unwrap().into_pyref();
    let column = get_node_field(vm, &object, "col_offset", name)?;
    let column = column.downcast_exact::<PyInt>(vm).unwrap().into_pyref();
    let location = make_location(row, column);
    let end_row = get_int_field(vm, &object, "end_lineno")?;
    let end_column = get_int_field(vm, &object, "end_col_offset")?;
    let end_location = if let (Some(row), Some(column)) = (end_row, end_column) {
        make_location(row.into_pyref(), column.into_pyref())
    } else {
        None
    };
    let range = TextRange::new(
        source_location_to_text_size(location.unwrap_or_default()),
        source_location_to_text_size(end_location.unwrap_or_default()),
    );
    Ok(range)
}

fn node_add_location(dict: &Py<PyDict>, range: TextRange, vm: &VirtualMachine) {
    let range = text_range_to_source_range(range);
    dict.set_item("lineno", vm.ctx.new_int(range.start.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "col_offset",
        vm.ctx.new_int(range.start.column.to_zero_indexed()).into(),
        vm,
    )
    .unwrap();
    dict.set_item("end_lineno", vm.ctx.new_int(range.end.row.get()).into(), vm)
        .unwrap();
    dict.set_item(
        "end_col_offset",
        vm.ctx.new_int(range.end.column.to_zero_indexed()).into(),
        vm,
    )
    .unwrap();
}

impl Node for ruff::Identifier {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let id = self.as_str();
        vm.ctx.new_str(id).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let py_str = PyStrRef::try_from_object(vm, object)?;
        Ok(ruff::Identifier::new(py_str.as_str(), TextRange::default()))
    }
}

impl Node for ruff::Int {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(int) = self.as_i32() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_u32() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_i64() {
            vm.ctx.new_int(int)
        } else if let Some(int) = self.as_u64() {
            vm.ctx.new_int(int)
        } else {
            // FIXME: performance
            let int = self.to_string().parse().unwrap();
            vm.ctx.new_bigint(&int)
        }
        .into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        // FIXME: performance
        let value: PyIntRef = object.try_into_value(vm)?;
        let value = value.as_bigint().to_string();
        Ok(value.parse().unwrap())
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

pub enum Constant {
    None,
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Int(BigInt),
    Tuple(Vec<Constant>),
    Float(f64),
    Complex { real: f64, imag: f64 },
    Ellipsis,
}

impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Constant::None => vm.ctx.none(),
            Constant::Bool(b) => vm.ctx.new_bool(b).into(),
            Constant::Str(s) => vm.ctx.new_str(s).into(),
            Constant::Bytes(b) => vm.ctx.new_bytes(b).into(),
            Constant::Int(i) => vm.ctx.new_int(i).into(),
            Constant::Tuple(t) => vm
                .ctx
                .new_tuple(t.into_iter().map(|c| c.ast_to_object(vm)).collect())
                .into(),
            Constant::Float(f) => vm.ctx.new_float(f).into(),
            Constant::Complex { real, imag } => vm.new_pyobj(Complex64::new(real, imag)),
            Constant::Ellipsis => vm.ctx.ellipsis(),
        }
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let constant = match_class!(match object {
            ref i @ builtins::int::PyInt => {
                let value = i.as_bigint();
                if object.class().is(vm.ctx.types.bool_type) {
                    Constant::Bool(!value.is_zero())
                } else {
                    Constant::Int(value.clone())
                }
            }
            ref f @ builtins::float::PyFloat => Constant::Float(f.to_f64()),
            ref c @ builtins::complex::PyComplex => {
                let c = c.to_complex();
                Constant::Complex {
                    real: c.re,
                    imag: c.im,
                }
            }
            ref s @ builtins::pystr::PyStr => Constant::Str(s.as_str().to_owned()),
            ref b @ builtins::bytes::PyBytes => Constant::Bytes(b.as_bytes().to_owned()),
            ref t @ builtins::tuple::PyTuple => {
                Constant::Tuple(
                    t.iter()
                        .map(|elt| Self::ast_from_object(vm, elt.clone()))
                        .collect::<Result<_, _>>()?,
                )
            }
            builtins::singletons::PyNone => Constant::None,
            builtins::slice::PyEllipsis => Constant::Ellipsis,
            obj =>
                return Err(vm.new_type_error(format!(
                    "invalid type in Constant: type '{}'",
                    obj.class().name()
                ))),
        });
        Ok(constant)
    }
}

impl Node for ruff::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object)?
            .to_u32()
            .and_then(ruff::ConversionFlag::from_op_arg)
            .ok_or_else(|| vm.new_value_error("invalid conversion flag".to_owned()))
    }
}

impl Node for ruff::Arguments {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct PositionalArguments {
    pub range: TextRange,
    pub args: Box<[ruff::Expr]>,
}

impl Node for PositionalArguments {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct KeywordArguments {
    pub range: TextRange,
    pub keywords: Box<[ruff::Keyword]>,
}

impl Node for KeywordArguments {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

fn merge_function_call_arguments(
    pos_args: PositionalArguments,
    key_args: KeywordArguments,
) -> ruff::Arguments {
    let range = pos_args.range.cover(key_args.range);

    ruff::Arguments {
        range,
        args: pos_args.args,
        keywords: key_args.keywords,
    }
}

fn split_function_call_arguments(args: ruff::Arguments) -> (PositionalArguments, KeywordArguments) {
    let ruff::Arguments {
        range,
        args,
        keywords,
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(range))
        .unwrap();
    debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = PositionalArguments {
        range: positional_arguments_range,
        args,
    };

    let keyword_arguments_range = keywords
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(range))
        .unwrap();
    debug_assert!(range.contains_range(keyword_arguments_range));
    let keyword_arguments = KeywordArguments {
        range: keyword_arguments_range,
        keywords,
    };

    (positional_arguments, keyword_arguments)
}

fn split_class_def_args(
    args: Option<Box<ruff::Arguments>>,
) -> (Option<PositionalArguments>, Option<KeywordArguments>) {
    let args = match args {
        None => return (None, None),
        Some(args) => *args,
    };
    let ruff::Arguments {
        range,
        args,
        keywords,
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(range))
        .unwrap();
    debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = PositionalArguments {
        range: positional_arguments_range,
        args,
    };

    let keyword_arguments_range = keywords
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(range))
        .unwrap();
    debug_assert!(range.contains_range(keyword_arguments_range));
    let keyword_arguments = KeywordArguments {
        range: keyword_arguments_range,
        keywords,
    };

    (Some(positional_arguments), Some(keyword_arguments))
}

fn merge_class_def_args(
    positional_arguments: Option<PositionalArguments>,
    keyword_arguments: Option<KeywordArguments>,
) -> Option<Box<ruff::Arguments>> {
    if positional_arguments.is_none() && keyword_arguments.is_none() {
        return None;
    }

    let args = if let Some(positional_arguments) = positional_arguments {
        positional_arguments.args
    } else {
        vec![].into_boxed_slice()
    };
    let keywords = if let Some(keyword_arguments) = keyword_arguments {
        keyword_arguments.keywords
    } else {
        vec![].into_boxed_slice()
    };

    Some(Box::new(ruff::Arguments {
        range: Default::default(), // TODO
        args,
        keywords,
    }))
}

struct PositionalParameters {
    pub range: TextRange,
    pub args: Box<[ruff::Parameter]>,
}

impl Node for PositionalParameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct KeywordParameters {
    pub range: TextRange,
    pub keywords: Box<[ruff::Parameter]>,
}

impl Node for KeywordParameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct ParameterDefaults {
    pub range: TextRange,
    defaults: Box<[Option<Box<ruff::Expr>>]>,
}

impl Node for ParameterDefaults {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

fn extract_positional_parameter_defaults(
    pos_only_args: &[ruff::ParameterWithDefault],
    args: &[ruff::ParameterWithDefault],
) -> (
    PositionalParameters,
    PositionalParameters,
    ParameterDefaults,
) {
    let mut defaults = vec![];
    defaults.extend(pos_only_args.iter().map(|item| item.default.clone()));
    defaults.extend(args.iter().map(|item| item.default.clone()));
    // If some positional parameters have no default value,
    // the "defaults" list contains only the defaults of the last "n" parameters.
    // Remove all positional parameters without a default value.
    defaults.retain(Option::is_some);
    let defaults = ParameterDefaults {
        range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap(),
        defaults: defaults.into_boxed_slice(),
    };

    let pos_only_args = PositionalParameters {
        range: pos_only_args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap(),
        args: {
            let pos_only_args: Vec<_> = pos_only_args
                .iter()
                .map(|item| item.parameter.clone())
                .collect();
            pos_only_args.into_boxed_slice()
        },
    };

    let args = PositionalParameters {
        range: args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap(),
        args: {
            let args: Vec<_> = args.iter().map(|item| item.parameter.clone()).collect();
            args.into_boxed_slice()
        },
    };

    (pos_only_args, args, defaults)
}

fn extract_keyword_parameter_defaults(
    kw_only_args: &[ruff::ParameterWithDefault],
) -> (KeywordParameters, ParameterDefaults) {
    let mut defaults = vec![];
    defaults.extend(kw_only_args.iter().map(|item| item.default.clone()));
    let defaults = ParameterDefaults {
        range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap(),
        defaults: defaults.into_boxed_slice(),
    };

    let kw_only_args = KeywordParameters {
        range: kw_only_args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap(),
        keywords: {
            let kw_only_args: Vec<_> = kw_only_args
                .iter()
                .map(|item| item.parameter.clone())
                .collect();
            kw_only_args.into_boxed_slice()
        },
    };

    (kw_only_args, defaults)
}

/// Represents the different types of Python module structures.
///
/// This enum is used to represent the various possible forms of a Python module
/// in an Abstract Syntax Tree (AST). It can correspond to:
///
/// - `Module`: A standard Python script, containing a sequence of statements
///   (e.g., assignments, function calls), possibly with type ignores.
/// - `Interactive`: A representation of code executed in an interactive
///   Python session (e.g., the REPL or Jupyter notebooks), where statements
///   are evaluated one at a time.
/// - `Expression`: A single expression without any surrounding statements.
///   This is typically used in scenarios like `eval()` or in expression-only
///   contexts.
/// - `FunctionType`: A function signature with argument and return type
///   annotations, representing the type hints of a function (e.g., `def add(x: int, y: int) -> int`).
enum Mod {
    Module(ruff::ModModule),
    Interactive(ModInteractive),
    Expression(ruff::ModExpression),
    FunctionType(ModFunctionType),
}

// sum
impl Node for Mod {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Module(cons) => cons.ast_to_object(vm),
            Self::Interactive(cons) => cons.ast_to_object(vm),
            Self::Expression(cons) => cons.ast_to_object(vm),
            Self::FunctionType(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeModModule::static_type()) {
            Self::Module(ruff::ModModule::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModInteractive::static_type()) {
            Self::Interactive(ModInteractive::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModExpression::static_type()) {
            Self::Expression(ruff::ModExpression::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModFunctionType::static_type()) {
            Self::FunctionType(ModFunctionType::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of mod, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ModModule {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ModModule {
            body,
            // type_ignores,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeModModule::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm), vm).unwrap();
        // TODO: ruff ignores type_ignore comments currently.
        // dict.set_item("type_ignores", type_ignores.ast_to_object(_vm), _vm)
        //     .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ModModule {
            body: Node::ast_from_object(vm, get_node_field(vm, &object, "body", "Module")?)?,
            // type_ignores: Node::ast_from_object(
            //     _vm,
            //     get_node_field(_vm, &_object, "type_ignores", "Module")?,
            // )?,
            range: Default::default(),
        })
    }
}

struct ModInteractive {
    range: TextRange,
    body: Vec<ruff::Stmt>,
}

// constructor
impl Node for ModInteractive {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeModInteractive::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "body", "Interactive")?,
            )?,
            range: Default::default(),
        })
    }
}
// constructor
impl Node for ruff::ModExpression {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeModExpression::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "Expression")?)?,
            range: Default::default(),
        })
    }
}

struct ModFunctionType {
    argtypes: Box<[ruff::Expr]>,
    returns: ruff::Expr,
    range: TextRange,
}

// constructor
impl Node for ModFunctionType {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ModFunctionType {
            argtypes,
            returns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeModFunctionType::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("argtypes", BoxedSlice(argtypes).ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ModFunctionType {
            argtypes: {
                let argtypes: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    get_node_field(vm, &object, "argtypes", "FunctionType")?,
                )?;
                argtypes.0
            },
            returns: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "returns", "FunctionType")?,
            )?,
            range: Default::default(),
        })
    }
}
// sum
impl Node for ruff::Stmt {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::Stmt::FunctionDef(cons) => cons.ast_to_object(vm),
            ruff::Stmt::ClassDef(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Return(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Delete(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Assign(cons) => cons.ast_to_object(vm),
            ruff::Stmt::TypeAlias(cons) => cons.ast_to_object(vm),
            ruff::Stmt::AugAssign(cons) => cons.ast_to_object(vm),
            ruff::Stmt::AnnAssign(cons) => cons.ast_to_object(vm),
            ruff::Stmt::For(cons) => cons.ast_to_object(vm),
            ruff::Stmt::While(cons) => cons.ast_to_object(vm),
            ruff::Stmt::If(cons) => cons.ast_to_object(vm),
            ruff::Stmt::With(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Match(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Raise(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Try(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Assert(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Import(cons) => cons.ast_to_object(vm),
            ruff::Stmt::ImportFrom(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Global(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Nonlocal(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Expr(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Pass(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Break(cons) => cons.ast_to_object(vm),
            ruff::Stmt::Continue(cons) => cons.ast_to_object(vm),
            ruff::Stmt::IpyEscapeCommand(_) => todo!(),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeStmtFunctionDef::static_type()) {
            ruff::Stmt::FunctionDef(ruff::StmtFunctionDef::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAsyncFunctionDef::static_type()) {
            ruff::Stmt::FunctionDef(ruff::StmtFunctionDef::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtClassDef::static_type()) {
            ruff::Stmt::ClassDef(ruff::StmtClassDef::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtReturn::static_type()) {
            ruff::Stmt::Return(ruff::StmtReturn::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtDelete::static_type()) {
            ruff::Stmt::Delete(ruff::StmtDelete::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAssign::static_type()) {
            ruff::Stmt::Assign(ruff::StmtAssign::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtTypeAlias::static_type()) {
            ruff::Stmt::TypeAlias(ruff::StmtTypeAlias::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAugAssign::static_type()) {
            ruff::Stmt::AugAssign(ruff::StmtAugAssign::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAnnAssign::static_type()) {
            ruff::Stmt::AnnAssign(ruff::StmtAnnAssign::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtFor::static_type()) {
            ruff::Stmt::For(ruff::StmtFor::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAsyncFor::static_type()) {
            ruff::Stmt::For(ruff::StmtFor::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtWhile::static_type()) {
            ruff::Stmt::While(ruff::StmtWhile::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtIf::static_type()) {
            ruff::Stmt::If(ruff::StmtIf::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtWith::static_type()) {
            ruff::Stmt::With(ruff::StmtWith::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAsyncWith::static_type()) {
            ruff::Stmt::With(ruff::StmtWith::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtMatch::static_type()) {
            ruff::Stmt::Match(ruff::StmtMatch::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtRaise::static_type()) {
            ruff::Stmt::Raise(ruff::StmtRaise::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtTry::static_type()) {
            ruff::Stmt::Try(ruff::StmtTry::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtTryStar::static_type()) {
            ruff::Stmt::Try(ruff::StmtTry::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtAssert::static_type()) {
            ruff::Stmt::Assert(ruff::StmtAssert::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtImport::static_type()) {
            ruff::Stmt::Import(ruff::StmtImport::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtImportFrom::static_type()) {
            ruff::Stmt::ImportFrom(ruff::StmtImportFrom::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtGlobal::static_type()) {
            ruff::Stmt::Global(ruff::StmtGlobal::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtNonlocal::static_type()) {
            ruff::Stmt::Nonlocal(ruff::StmtNonlocal::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtExpr::static_type()) {
            ruff::Stmt::Expr(ruff::StmtExpr::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtPass::static_type()) {
            ruff::Stmt::Pass(ruff::StmtPass::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtBreak::static_type()) {
            ruff::Stmt::Break(ruff::StmtBreak::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeStmtContinue::static_type()) {
            ruff::Stmt::Continue(ruff::StmtContinue::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of stmt, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::StmtFunctionDef {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            parameters,
            body,
            decorator_list,
            returns,
            // type_comment,
            type_params,
            is_async,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtFunctionDef::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncFunctionDef::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", vm.ctx.new_str(name.as_str()).to_pyobject(vm), vm)
            .unwrap();
        dict.set_item("args", parameters.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm), vm).unwrap();
        dict.set_item("decorator_list", decorator_list.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm), vm)
            .unwrap();
        // TODO: Ruff ignores type_comment during parsing
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        dict.set_item("type_params", type_params.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        let is_async = _cls.is(gen::NodeStmtAsyncFunctionDef::static_type());
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "name", "FunctionDef")?,
            )?,
            parameters: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "args", "FunctionDef")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "body", "FunctionDef")?,
            )?,
            decorator_list: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "decorator_list", "FunctionDef")?,
            )?,
            returns: get_node_field_opt(_vm, &_object, "returns")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            // TODO: Ruff ignores type_comment during parsing
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            type_params: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "type_params", "FunctionDef")?,
            )?,
            range: range_from_object(_vm, _object, "FunctionDef")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtClassDef {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            arguments,
            body,
            decorator_list,
            type_params,
            range: _range,
        } = self;
        let (bases, keywords) = split_class_def_args(arguments);
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtClassDef::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("bases", bases.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("keywords", keywords.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("decorator_list", decorator_list.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("type_params", type_params.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let bases =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "bases", "ClassDef")?)?;
        let keywords =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "keywords", "ClassDef")?)?;
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "ClassDef")?)?,
            arguments: merge_class_def_args(bases, keywords),
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "ClassDef")?)?,
            decorator_list: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "decorator_list", "ClassDef")?,
            )?,
            type_params: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "type_params", "ClassDef")?,
            )?,
            range: range_from_object(_vm, _object, "ClassDef")?,
        })
    }
}
// constructor
impl Node for ruff::StmtReturn {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtReturn {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtReturn::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtReturn {
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Return")?,
        })
    }
}
// constructor
impl Node for ruff::StmtDelete {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtDelete {
            targets,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtDelete::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtDelete {
            targets: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "targets", "Delete")?,
            )?,
            range: range_from_object(_vm, _object, "Delete")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAssign {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            targets,
            value,
            // type_comment,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            targets: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "targets", "Assign")?,
            )?,
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Assign")?)?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, _object, "Assign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtTypeAlias {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtTypeAlias {
            name,
            type_params,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtTypeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("type_params", type_params.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtTypeAlias {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "TypeAlias")?)?,
            type_params: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "type_params", "TypeAlias")?,
            )?,
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "TypeAlias")?,
            )?,
            range: range_from_object(_vm, _object, "TypeAlias")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAugAssign {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            target,
            op,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAugAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "target", "AugAssign")?,
            )?,
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "AugAssign")?)?,
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "AugAssign")?,
            )?,
            range: range_from_object(_vm, _object, "AugAssign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAnnAssign {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            target,
            annotation,
            value,
            simple,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAnnAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("annotation", annotation.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("simple", simple.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "target", "AnnAssign")?,
            )?,
            annotation: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "annotation", "AnnAssign")?,
            )?,
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            simple: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "simple", "AnnAssign")?,
            )?,
            range: range_from_object(_vm, _object, "AnnAssign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtFor {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            is_async,
            target,
            iter,
            body,
            orelse,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtFor::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncFor::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtFor::static_type())
                || _cls.is(gen::NodeStmtAsyncFor::static_type())
        );
        let is_async = _cls.is(gen::NodeStmtAsyncFor::static_type());
        Ok(Self {
            target: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "target", "For")?)?,
            iter: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "iter", "For")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "For")?)?,
            orelse: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "orelse", "For")?)?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, _object, "For")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtWhile {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtWhile::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "test", "While")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "While")?)?,
            orelse: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "orelse", "While")?)?,
            range: range_from_object(_vm, _object, "While")?,
        })
    }
}
// constructor
impl Node for ruff::StmtIf {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            test,
            body,
            range: _range,
            elif_else_clauses,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtIf::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("orelse", elif_else_clauses.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "test", "If")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "If")?)?,
            elif_else_clauses: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "orelse", "If")?,
            )?,
            range: range_from_object(_vm, _object, "If")?,
        })
    }
}
// constructor
impl Node for ruff::StmtWith {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            is_async,
            items,
            body,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtWith::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncWith::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("items", items.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtWith::static_type())
                || _cls.is(gen::NodeStmtAsyncWith::static_type())
        );
        let is_async = _cls.is(gen::NodeStmtAsyncWith::static_type());
        Ok(Self {
            items: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "items", "With")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "With")?)?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, _object, "With")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtMatch {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            subject,
            cases,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtMatch::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("subject", subject.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("cases", cases.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            subject: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "subject", "Match")?,
            )?,
            cases: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "cases", "Match")?)?,
            range: range_from_object(_vm, _object, "Match")?,
        })
    }
}
// constructor
impl Node for ruff::StmtRaise {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            exc,
            cause,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtRaise::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("exc", exc.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("cause", cause.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            exc: get_node_field_opt(_vm, &_object, "exc")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            cause: get_node_field_opt(_vm, &_object, "cause")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Raise")?,
        })
    }
}
// constructor
impl Node for ruff::StmtTry {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            body,
            handlers,
            orelse,
            finalbody,
            range: _range,
            is_star,
        } = self;

        // let cls = gen::NodeStmtTry::static_type().to_owned();
        let cls = if is_star {
            gen::NodeStmtTryStar::static_type()
        } else {
            gen::NodeStmtTry::static_type()
        }
        .to_owned();

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("handlers", handlers.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("finalbody", finalbody.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        let is_star = _cls.is(gen::NodeStmtTryStar::static_type());
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtTry::static_type())
                || _cls.is(gen::NodeStmtTryStar::static_type())
        );

        Ok(Self {
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "Try")?)?,
            handlers: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "handlers", "Try")?,
            )?,
            orelse: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "orelse", "Try")?)?,
            finalbody: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "finalbody", "Try")?,
            )?,
            range: range_from_object(_vm, _object, "Try")?,
            is_star,
        })
    }
}
// constructor
impl Node for ruff::StmtAssert {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtAssert {
            test,
            msg,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAssert::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("msg", msg.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtAssert {
            test: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "test", "Assert")?)?,
            msg: get_node_field_opt(_vm, &_object, "msg")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Assert")?,
        })
    }
}
// constructor
impl Node for ruff::StmtImport {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtImport {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtImport::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtImport {
            names: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "names", "Import")?)?,
            range: range_from_object(_vm, _object, "Import")?,
        })
    }
}
// constructor
impl Node for ruff::StmtImportFrom {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            module,
            names,
            level,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeStmtImportFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("module", module.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("names", names.ast_to_object(vm), vm).unwrap();
        dict.set_item("level", vm.ctx.new_int(level).to_pyobject(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            module: get_node_field_opt(vm, &_object, "module")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            names: Node::ast_from_object(vm, get_node_field(vm, &_object, "names", "ImportFrom")?)?,
            level: get_node_field(vm, &_object, "level", "ImportFrom")?
                .downcast_exact::<PyInt>(vm)
                .unwrap()
                .try_to_primitive::<u32>(vm)?,
            range: range_from_object(vm, _object, "ImportFrom")?,
        })
    }
}
// constructor
impl Node for ruff::StmtGlobal {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtGlobal {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtGlobal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtGlobal {
            names: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "names", "Global")?)?,
            range: range_from_object(_vm, _object, "Global")?,
        })
    }
}
// constructor
impl Node for ruff::StmtNonlocal {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtNonlocal {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtNonlocal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtNonlocal {
            names: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "names", "Nonlocal")?)?,
            range: range_from_object(_vm, _object, "Nonlocal")?,
        })
    }
}
// constructor
impl Node for ruff::StmtExpr {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtExpr {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtExpr {
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Expr")?)?,
            range: range_from_object(_vm, _object, "Expr")?,
        })
    }
}
// constructor
impl Node for ruff::StmtPass {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtPass { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtPass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtPass {
            range: range_from_object(_vm, _object, "Pass")?,
        })
    }
}
// constructor
impl Node for ruff::StmtBreak {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtBreak { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtBreak::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtBreak {
            range: range_from_object(_vm, _object, "Break")?,
        })
    }
}
// constructor
impl Node for ruff::StmtContinue {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::StmtContinue { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtContinue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::StmtContinue {
            range: range_from_object(_vm, _object, "Continue")?,
        })
    }
}
// sum
impl Node for ruff::Expr {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::Expr::BoolOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Name(cons) => cons.ast_to_object(vm),
            ruff::Expr::BinOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::UnaryOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Lambda(cons) => cons.ast_to_object(vm),
            ruff::Expr::If(cons) => cons.ast_to_object(vm),
            ruff::Expr::Dict(cons) => cons.ast_to_object(vm),
            ruff::Expr::Set(cons) => cons.ast_to_object(vm),
            ruff::Expr::ListComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::SetComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::DictComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Generator(cons) => cons.ast_to_object(vm),
            ruff::Expr::Await(cons) => cons.ast_to_object(vm),
            ruff::Expr::Yield(cons) => cons.ast_to_object(vm),
            ruff::Expr::YieldFrom(cons) => cons.ast_to_object(vm),
            ruff::Expr::Compare(cons) => cons.ast_to_object(vm),
            ruff::Expr::Call(cons) => cons.ast_to_object(vm),
            // ruff::Expr::FormattedValue(cons) => cons.ast_to_object(vm),
            // ruff::Expr::JoinedStr(cons) => cons.ast_to_object(vm),
            // ruff::Expr::Constant(cons) => cons.ast_to_object(vm),
            ruff::Expr::Attribute(cons) => cons.ast_to_object(vm),
            ruff::Expr::Subscript(cons) => cons.ast_to_object(vm),
            ruff::Expr::Starred(cons) => cons.ast_to_object(vm),
            ruff::Expr::List(cons) => cons.ast_to_object(vm),
            ruff::Expr::Tuple(cons) => cons.ast_to_object(vm),
            ruff::Expr::Slice(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeExprBoolOp::static_type()) {
            ruff::Expr::BoolOp(ruff::ExprBoolOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprNamedExpr::static_type()) {
            ruff::Expr::Named(ruff::ExprNamed::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprBinOp::static_type()) {
            ruff::Expr::BinOp(ruff::ExprBinOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprUnaryOp::static_type()) {
            ruff::Expr::UnaryOp(ruff::ExprUnaryOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprLambda::static_type()) {
            ruff::Expr::Lambda(ruff::ExprLambda::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprIfExp::static_type()) {
            ruff::Expr::If(ruff::ExprIf::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprDict::static_type()) {
            ruff::Expr::Dict(ruff::ExprDict::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSet::static_type()) {
            ruff::Expr::Set(ruff::ExprSet::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprListComp::static_type()) {
            ruff::Expr::ListComp(ruff::ExprListComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSetComp::static_type()) {
            ruff::Expr::SetComp(ruff::ExprSetComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprDictComp::static_type()) {
            ruff::Expr::DictComp(ruff::ExprDictComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprGeneratorExp::static_type()) {
            ruff::Expr::Generator(ruff::ExprGenerator::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprAwait::static_type()) {
            ruff::Expr::Await(ruff::ExprAwait::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprYield::static_type()) {
            ruff::Expr::Yield(ruff::ExprYield::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprYieldFrom::static_type()) {
            ruff::Expr::YieldFrom(ruff::ExprYieldFrom::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprCompare::static_type()) {
            ruff::Expr::Compare(ruff::ExprCompare::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprCall::static_type()) {
            ruff::Expr::Call(ruff::ExprCall::ast_from_object(_vm, _object)?)
        // } else if _cls.is(gen::NodeExprFormattedValue::static_type()) {
        //     ruff::Expr::FormattedValue(ruff::ExprFormattedValue::ast_from_object(_vm, _object)?)
        // } else if _cls.is(gen::NodeExprJoinedStr::static_type()) {
        //     ruff::Expr::JoinedStr(ruff::ExprJoinedStr::ast_from_object(_vm, _object)?)
        // } else if _cls.is(gen::NodeExprConstant::static_type()) {
        //     ruff::Expr::Constant(Constant::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprAttribute::static_type()) {
            ruff::Expr::Attribute(ruff::ExprAttribute::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSubscript::static_type()) {
            ruff::Expr::Subscript(ruff::ExprSubscript::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprStarred::static_type()) {
            ruff::Expr::Starred(ruff::ExprStarred::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprName::static_type()) {
            ruff::Expr::Name(ruff::ExprName::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprList::static_type()) {
            ruff::Expr::List(ruff::ExprList::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprTuple::static_type()) {
            ruff::Expr::Tuple(ruff::ExprTuple::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSlice::static_type()) {
            ruff::Expr::Slice(ruff::ExprSlice::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ExprBoolOp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprBoolOp {
            op,
            values,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("values", values.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprBoolOp {
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "BoolOp")?)?,
            values: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "values", "BoolOp")?)?,
            range: range_from_object(_vm, _object, "BoolOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprNamed {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            target,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprNamedExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "target", "NamedExpr")?,
            )?,
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "NamedExpr")?,
            )?,
            range: range_from_object(_vm, _object, "NamedExpr")?,
        })
    }
}
// constructor
impl Node for ruff::ExprBinOp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            op,
            right,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("right", right.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "left", "BinOp")?)?,
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "BinOp")?)?,
            right: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "right", "BinOp")?)?,
            range: range_from_object(_vm, _object, "BinOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprUnaryOp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            op,
            operand,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("operand", operand.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "UnaryOp")?)?,
            operand: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "operand", "UnaryOp")?,
            )?,
            range: range_from_object(_vm, _object, "UnaryOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprLambda {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("args", parameters.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            parameters: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "args", "Lambda")?,
            )?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "Lambda")?)?,
            range: range_from_object(_vm, _object, "Lambda")?,
        })
    }
}
// constructor
impl Node for ruff::ExprIf {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "test", "IfExp")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "IfExp")?)?,
            orelse: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "orelse", "IfExp")?)?,
            range: range_from_object(_vm, _object, "IfExp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDict {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            items,
            range: _range,
        } = self;
        let (keys, values) =
            items
                .into_iter()
                .fold((vec![], vec![]), |(mut keys, mut values), item| {
                    keys.push(item.key);
                    values.push(item.value);
                    (keys, values)
                });
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprDict::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("values", values.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let keys: Vec<Option<ruff::Expr>> =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "keys", "Dict")?)?;
        let values: Vec<_> =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "values", "Dict")?)?;
        let items = keys
            .into_iter()
            .zip(values.into_iter())
            .map(|(key, value)| ruff::DictItem { key, value })
            .collect();
        Ok(Self {
            items,
            range: range_from_object(_vm, _object, "Dict")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSet {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSet {
            elts,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSet {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "Set")?)?,
            range: range_from_object(_vm, _object, "Set")?,
        })
    }
}
// constructor
impl Node for ruff::ExprListComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprListComp {
            elt,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprListComp {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "ListComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "ListComp")?,
            )?,
            range: range_from_object(_vm, _object, "ListComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSetComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSetComp {
            elt,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSetComp {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "SetComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "SetComp")?,
            )?,
            range: range_from_object(_vm, _object, "SetComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDictComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprDictComp {
            key,
            value,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprDictComp {
            key: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "key", "DictComp")?)?,
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "DictComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "DictComp")?,
            )?,
            range: range_from_object(_vm, _object, "DictComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprGenerator {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "GeneratorExp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "GeneratorExp")?,
            )?,
            range: range_from_object(_vm, _object, "GeneratorExp")?,
            // TODO: Is this correct?
            parenthesized: true,
        })
    }
}
// constructor
impl Node for ruff::ExprAwait {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprAwait {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprAwait {
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Await")?)?,
            range: range_from_object(_vm, _object, "Await")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYield {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprYield {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprYield {
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Yield")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYieldFrom {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprYieldFrom {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprYieldFrom {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "YieldFrom")?,
            )?,
            range: range_from_object(_vm, _object, "YieldFrom")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCompare {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            ops,
            comparators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item(
            "comparators",
            BoxedSlice(comparators).ast_to_object(_vm),
            _vm,
        )
        .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "left", "Compare")?)?,
            ops: {
                let ops: BoxedSlice<_> =
                    Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ops", "Compare")?)?;
                ops.0
            },
            comparators: {
                let comparators: BoxedSlice<_> = Node::ast_from_object(
                    _vm,
                    get_node_field(_vm, &_object, "comparators", "Compare")?,
                )?;
                comparators.0
            },
            range: range_from_object(_vm, _object, "Compare")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCall {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            func,
            arguments: args,
            range: _range,
        } = self;
        let (pos_args, key_args) = split_function_call_arguments(args);
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprCall::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("func", func.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("args", pos_args.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("keywords", key_args.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            func: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "func", "Call")?)?,
            arguments: merge_function_call_arguments(
                Node::ast_from_object(_vm, get_node_field(_vm, &_object, "args", "Call")?)?,
                Node::ast_from_object(_vm, get_node_field(_vm, &_object, "keywords", "Call")?)?,
            ),
            range: range_from_object(_vm, _object, "Call")?,
        })
    }
}
// // constructor
// impl Node for ruff::ExprFormattedValue {
//     fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
//         let ruff::ExprFormattedValue {
//             value,
//             conversion,
//             format_spec,
//             range: _range,
//         } = self;
//         let node = NodeAst
//             .into_ref_with_type(_vm, gen::NodeExprFormattedValue::static_type().to_owned())
//             .unwrap();
//         let dict = node.as_object().dict().unwrap();
//         dict.set_item("value", value.ast_to_object(_vm), _vm)
//             .unwrap();
//         dict.set_item("conversion", conversion.ast_to_object(_vm), _vm)
//             .unwrap();
//         dict.set_item("format_spec", format_spec.ast_to_object(_vm), _vm)
//             .unwrap();
//         node_add_location(&dict, _range, _vm);
//         node.into()
//     }
//     fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
//         Ok(ruff::ExprFormattedValue {
//             value: Node::ast_from_object(
//                 _vm,
//                 get_node_field(_vm, &_object, "value", "FormattedValue")?,
//             )?,
//             conversion: Node::ast_from_object(
//                 _vm,
//                 get_node_field(_vm, &_object, "conversion", "FormattedValue")?,
//             )?,
//             format_spec: get_node_field_opt(_vm, &_object, "format_spec")?
//                 .map(|obj| Node::ast_from_object(_vm, obj))
//                 .transpose()?,
//             range: range_from_object(_vm, _object, "FormattedValue")?,
//         })
//     }
// }
// // constructor
// impl Node for ruff::ExprJoinedStr {
//     fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
//         let ruff::ExprJoinedStr {
//             values,
//             range: _range,
//         } = self;
//         let node = NodeAst
//             .into_ref_with_type(_vm, gen::NodeExprJoinedStr::static_type().to_owned())
//             .unwrap();
//         let dict = node.as_object().dict().unwrap();
//         dict.set_item("values", values.ast_to_object(_vm), _vm)
//             .unwrap();
//         node_add_location(&dict, _range, _vm);
//         node.into()
//     }
//     fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
//         Ok(ruff::ExprJoinedStr {
//             values: Node::ast_from_object(
//                 _vm,
//                 get_node_field(_vm, &_object, "values", "JoinedStr")?,
//             )?,
//             range: range_from_object(_vm, _object, "JoinedStr")?,
//         })
//     }
// }
// // constructor
// impl Node for ruff::ExprConstant {
//     fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
//         let Self {
//             value,
//             kind,
//             range: _range,
//         } = self;
//         let node = NodeAst
//             .into_ref_with_type(_vm, gen::NodeExprConstant::static_type().to_owned())
//             .unwrap();
//         let dict = node.as_object().dict().unwrap();
//         dict.set_item("value", value.ast_to_object(_vm), _vm)
//             .unwrap();
//         dict.set_item("kind", kind.ast_to_object(_vm), _vm).unwrap();
//         node_add_location(&dict, _range, _vm);
//         node.into()
//     }
//     fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
//         Ok(Self {
//             value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Constant")?)?,
//             kind: get_node_field_opt(_vm, &_object, "kind")?
//                 .map(|obj| Node::ast_from_object(_vm, obj))
//                 .transpose()?,
//             range: range_from_object(_vm, _object, "Constant")?,
//         })
//     }
// }
// constructor
impl Node for ruff::ExprAttribute {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            attr,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("attr", attr.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "Attribute")?,
            )?,
            attr: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "attr", "Attribute")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Attribute")?)?,
            range: range_from_object(_vm, _object, "Attribute")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSubscript {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSubscript {
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("slice", slice.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSubscript {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "Subscript")?,
            )?,
            slice: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "slice", "Subscript")?,
            )?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Subscript")?)?,
            range: range_from_object(_vm, _object, "Subscript")?,
        })
    }
}
// constructor
impl Node for ruff::ExprStarred {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprStarred {
            value,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprStarred {
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Starred")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Starred")?)?,
            range: range_from_object(_vm, _object, "Starred")?,
        })
    }
}
// constructor
impl Node for ruff::ExprName {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprName {
            id,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprName {
            id: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "id", "Name")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Name")?)?,
            range: range_from_object(_vm, _object, "Name")?,
        })
    }
}
// constructor
impl Node for ruff::ExprList {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprList {
            elts,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprList {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "List")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "List")?)?,
            range: range_from_object(_vm, _object, "List")?,
        })
    }
}
// constructor
impl Node for ruff::ExprTuple {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprTuple {
            elts,
            ctx,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprTuple::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprTuple {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "Tuple")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Tuple")?)?,
            range: range_from_object(_vm, _object, "Tuple")?,
            parenthesized: true, // TODO: is this correct?
        })
    }
}
// constructor
impl Node for ruff::ExprSlice {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSlice {
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("upper", upper.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("step", step.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSlice {
            lower: get_node_field_opt(_vm, &_object, "lower")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            upper: get_node_field_opt(_vm, &_object, "upper")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            step: get_node_field_opt(_vm, &_object, "step")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Slice")?,
        })
    }
}
// sum
impl Node for ruff::ExprContext {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::ExprContext::Load => gen::NodeExprContextLoad::static_type(),
            ruff::ExprContext::Store => gen::NodeExprContextStore::static_type(),
            ruff::ExprContext::Del => gen::NodeExprContextDel::static_type(),
            ruff::ExprContext::Invalid => todo!(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeExprContextLoad::static_type()) {
            ruff::ExprContext::Load
        } else if _cls.is(gen::NodeExprContextStore::static_type()) {
            ruff::ExprContext::Store
        } else if _cls.is(gen::NodeExprContextDel::static_type()) {
            ruff::ExprContext::Del
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of expr_context, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::BoolOp::And => gen::NodeBoolOpAnd::static_type(),
            ruff::BoolOp::Or => gen::NodeBoolOpOr::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeBoolOpAnd::static_type()) {
            ruff::BoolOp::And
        } else if _cls.is(gen::NodeBoolOpOr::static_type()) {
            ruff::BoolOp::Or
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of boolop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::Operator {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::Operator::Add => gen::NodeOperatorAdd::static_type(),
            ruff::Operator::Sub => gen::NodeOperatorSub::static_type(),
            ruff::Operator::Mult => gen::NodeOperatorMult::static_type(),
            ruff::Operator::MatMult => gen::NodeOperatorMatMult::static_type(),
            ruff::Operator::Div => gen::NodeOperatorDiv::static_type(),
            ruff::Operator::Mod => gen::NodeOperatorMod::static_type(),
            ruff::Operator::Pow => gen::NodeOperatorPow::static_type(),
            ruff::Operator::LShift => gen::NodeOperatorLShift::static_type(),
            ruff::Operator::RShift => gen::NodeOperatorRShift::static_type(),
            ruff::Operator::BitOr => gen::NodeOperatorBitOr::static_type(),
            ruff::Operator::BitXor => gen::NodeOperatorBitXor::static_type(),
            ruff::Operator::BitAnd => gen::NodeOperatorBitAnd::static_type(),
            ruff::Operator::FloorDiv => gen::NodeOperatorFloorDiv::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeOperatorAdd::static_type()) {
            ruff::Operator::Add
        } else if _cls.is(gen::NodeOperatorSub::static_type()) {
            ruff::Operator::Sub
        } else if _cls.is(gen::NodeOperatorMult::static_type()) {
            ruff::Operator::Mult
        } else if _cls.is(gen::NodeOperatorMatMult::static_type()) {
            ruff::Operator::MatMult
        } else if _cls.is(gen::NodeOperatorDiv::static_type()) {
            ruff::Operator::Div
        } else if _cls.is(gen::NodeOperatorMod::static_type()) {
            ruff::Operator::Mod
        } else if _cls.is(gen::NodeOperatorPow::static_type()) {
            ruff::Operator::Pow
        } else if _cls.is(gen::NodeOperatorLShift::static_type()) {
            ruff::Operator::LShift
        } else if _cls.is(gen::NodeOperatorRShift::static_type()) {
            ruff::Operator::RShift
        } else if _cls.is(gen::NodeOperatorBitOr::static_type()) {
            ruff::Operator::BitOr
        } else if _cls.is(gen::NodeOperatorBitXor::static_type()) {
            ruff::Operator::BitXor
        } else if _cls.is(gen::NodeOperatorBitAnd::static_type()) {
            ruff::Operator::BitAnd
        } else if _cls.is(gen::NodeOperatorFloorDiv::static_type()) {
            ruff::Operator::FloorDiv
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of operator, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::UnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::UnaryOp::Invert => gen::NodeUnaryOpInvert::static_type(),
            ruff::UnaryOp::Not => gen::NodeUnaryOpNot::static_type(),
            ruff::UnaryOp::UAdd => gen::NodeUnaryOpUAdd::static_type(),
            ruff::UnaryOp::USub => gen::NodeUnaryOpUSub::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeUnaryOpInvert::static_type()) {
            ruff::UnaryOp::Invert
        } else if _cls.is(gen::NodeUnaryOpNot::static_type()) {
            ruff::UnaryOp::Not
        } else if _cls.is(gen::NodeUnaryOpUAdd::static_type()) {
            ruff::UnaryOp::UAdd
        } else if _cls.is(gen::NodeUnaryOpUSub::static_type()) {
            ruff::UnaryOp::USub
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of unaryop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::CmpOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::CmpOp::Eq => gen::NodeCmpOpEq::static_type(),
            ruff::CmpOp::NotEq => gen::NodeCmpOpNotEq::static_type(),
            ruff::CmpOp::Lt => gen::NodeCmpOpLt::static_type(),
            ruff::CmpOp::LtE => gen::NodeCmpOpLtE::static_type(),
            ruff::CmpOp::Gt => gen::NodeCmpOpGt::static_type(),
            ruff::CmpOp::GtE => gen::NodeCmpOpGtE::static_type(),
            ruff::CmpOp::Is => gen::NodeCmpOpIs::static_type(),
            ruff::CmpOp::IsNot => gen::NodeCmpOpIsNot::static_type(),
            ruff::CmpOp::In => gen::NodeCmpOpIn::static_type(),
            ruff::CmpOp::NotIn => gen::NodeCmpOpNotIn::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeCmpOpEq::static_type()) {
            ruff::CmpOp::Eq
        } else if _cls.is(gen::NodeCmpOpNotEq::static_type()) {
            ruff::CmpOp::NotEq
        } else if _cls.is(gen::NodeCmpOpLt::static_type()) {
            ruff::CmpOp::Lt
        } else if _cls.is(gen::NodeCmpOpLtE::static_type()) {
            ruff::CmpOp::LtE
        } else if _cls.is(gen::NodeCmpOpGt::static_type()) {
            ruff::CmpOp::Gt
        } else if _cls.is(gen::NodeCmpOpGtE::static_type()) {
            ruff::CmpOp::GtE
        } else if _cls.is(gen::NodeCmpOpIs::static_type()) {
            ruff::CmpOp::Is
        } else if _cls.is(gen::NodeCmpOpIsNot::static_type()) {
            ruff::CmpOp::IsNot
        } else if _cls.is(gen::NodeCmpOpIn::static_type()) {
            ruff::CmpOp::In
        } else if _cls.is(gen::NodeCmpOpNotIn::static_type()) {
            ruff::CmpOp::NotIn
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of cmpop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// product
impl Node for ruff::Comprehension {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::Comprehension {
            target,
            iter,
            ifs,
            is_async,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ifs", ifs.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("is_async", is_async.ast_to_object(_vm), _vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::Comprehension {
            target: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "target", "comprehension")?,
            )?,
            iter: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "iter", "comprehension")?,
            )?,
            ifs: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "ifs", "comprehension")?,
            )?,
            is_async: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "is_async", "comprehension")?,
            )?,
            range: Default::default(),
        })
    }
}
// sum
impl Node for ruff::ExceptHandler {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::ExceptHandler::ExceptHandler(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(
            if _cls.is(gen::NodeExceptHandlerExceptHandler::static_type()) {
                ruff::ExceptHandler::ExceptHandler(
                    ruff::ExceptHandlerExceptHandler::ast_from_object(_vm, _object)?,
                )
            } else {
                return Err(_vm.new_type_error(format!(
                    "expected some sort of excepthandler, but got {}",
                    _object.repr(_vm)?
                )));
            },
        )
    }
}
// constructor
impl Node for ruff::ExceptHandlerExceptHandler {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExceptHandlerExceptHandler {
            type_,
            name,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodeExceptHandlerExceptHandler::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("type", type_.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExceptHandlerExceptHandler {
            type_: get_node_field_opt(_vm, &_object, "type")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            body: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "body", "ExceptHandler")?,
            )?,
            range: range_from_object(_vm, _object, "ExceptHandler")?,
        })
    }
}
// product
impl Node for ruff::Parameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kwarg,
            range: _range,
        } = self;
        let (posonlyargs, args, defaults) =
            extract_positional_parameter_defaults(&posonlyargs, &args);
        let (kwonlyargs, kw_defaults) = extract_keyword_parameter_defaults(&kwonlyargs);
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeArguments::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("posonlyargs", posonlyargs.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("args", args.ast_to_object(vm), vm).unwrap();
        dict.set_item("vararg", vararg.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("kwonlyargs", kwonlyargs.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("kw_defaults", kw_defaults.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("kwarg", kwarg.ast_to_object(vm), vm).unwrap();
        dict.set_item("defaults", defaults.ast_to_object(vm), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            posonlyargs: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "posonlyargs", "arguments")?,
            )?,
            args: Node::ast_from_object(vm, get_node_field(vm, &object, "args", "arguments")?)?,
            vararg: get_node_field_opt(vm, &object, "vararg")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            kwonlyargs: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "kwonlyargs", "arguments")?,
            )?,
            kw_defaults: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "kw_defaults", "arguments")?,
            )?,
            kwarg: get_node_field_opt(vm, &object, "kwarg")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            defaults: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "defaults", "arguments")?,
            )?,
            range: Default::default(),
        })
    }
}
// product
impl Node for ruff::Parameter {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            annotation,
            // type_comment,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeArg::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("annotation", annotation.ast_to_object(_vm), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "arg", "arg")?)?,
            annotation: get_node_field_opt(_vm, &_object, "annotation")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, _object, "arg")?,
        })
    }
}
// product
impl Node for ruff::Keyword {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            arg,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeKeyword::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", arg.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            arg: get_node_field_opt(_vm, &_object, "arg")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "keyword")?)?,
            range: range_from_object(_vm, _object, "keyword")?,
        })
    }
}
// product
impl Node for ruff::Alias {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            asname,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("asname", asname.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "alias")?)?,
            asname: get_node_field_opt(_vm, &_object, "asname")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "alias")?,
        })
    }
}
// product
impl Node for ruff::WithItem {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            context_expr,
            optional_vars,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeWithItem::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("context_expr", context_expr.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("optional_vars", optional_vars.ast_to_object(_vm), _vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            context_expr: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "context_expr", "withitem")?,
            )?,
            optional_vars: get_node_field_opt(_vm, &_object, "optional_vars")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: Default::default(),
        })
    }
}
// product
impl Node for ruff::MatchCase {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            pattern,
            guard,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeMatchCase::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("guard", guard.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            pattern: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "pattern", "match_case")?,
            )?,
            guard: get_node_field_opt(_vm, &_object, "guard")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "match_case")?)?,
            range: Default::default(),
        })
    }
}
// sum
impl Node for ruff::Pattern {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::Pattern::MatchValue(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchSingleton(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchSequence(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchMapping(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchClass(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchStar(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchAs(cons) => cons.ast_to_object(vm),
            ruff::Pattern::MatchOr(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodePatternMatchValue::static_type()) {
            ruff::Pattern::MatchValue(ruff::PatternMatchValue::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchSingleton::static_type()) {
            ruff::Pattern::MatchSingleton(ruff::PatternMatchSingleton::ast_from_object(
                _vm, _object,
            )?)
        } else if _cls.is(gen::NodePatternMatchSequence::static_type()) {
            ruff::Pattern::MatchSequence(ruff::PatternMatchSequence::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchMapping::static_type()) {
            ruff::Pattern::MatchMapping(ruff::PatternMatchMapping::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchClass::static_type()) {
            ruff::Pattern::MatchClass(ruff::PatternMatchClass::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchStar::static_type()) {
            ruff::Pattern::MatchStar(ruff::PatternMatchStar::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchAs::static_type()) {
            ruff::Pattern::MatchAs(ruff::PatternMatchAs::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodePatternMatchOr::static_type()) {
            ruff::Pattern::MatchOr(ruff::PatternMatchOr::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of pattern, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::PatternMatchValue {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "MatchValue")?,
            )?,
            range: range_from_object(_vm, _object, "MatchValue")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchSingleton {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodePatternMatchSingleton::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "MatchSingleton")?,
            )?,
            range: range_from_object(_vm, _object, "MatchSingleton")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchSequence {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchSequence::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchSequence")?,
            )?,
            range: range_from_object(_vm, _object, "MatchSequence")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchMapping {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            keys,
            patterns,
            rest,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchMapping::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("rest", rest.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            keys: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "keys", "MatchMapping")?,
            )?,
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchMapping")?,
            )?,
            rest: get_node_field_opt(_vm, &_object, "rest")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchMapping")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchClass {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            cls,
            patterns,
            kwd_attrs,
            kwd_patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchClass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("cls", cls.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("kwd_attrs", kwd_attrs.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("kwd_patterns", kwd_patterns.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            cls: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "cls", "MatchClass")?)?,
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchClass")?,
            )?,
            kwd_attrs: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "kwd_attrs", "MatchClass")?,
            )?,
            kwd_patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "kwd_patterns", "MatchClass")?,
            )?,
            range: range_from_object(_vm, _object, "MatchClass")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchStar {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchStar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchStar")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchAs {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            pattern,
            name,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchAs::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("pattern", pattern.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            pattern: get_node_field_opt(_vm, &_object, "pattern")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "MatchAs")?,
        })
    }
}
// constructor
impl Node for ruff::PatternMatchOr {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            patterns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodePatternMatchOr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("patterns", patterns.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            patterns: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "patterns", "MatchOr")?,
            )?,
            range: range_from_object(_vm, _object, "MatchOr")?,
        })
    }
}

enum TypeIgnore {
    TypeIgnore(TypeIgnoreTypeIgnore),
}

// sum
impl Node for TypeIgnore {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            TypeIgnore::TypeIgnore(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeTypeIgnoreTypeIgnore::static_type()) {
            TypeIgnore::TypeIgnore(TypeIgnoreTypeIgnore::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of type_ignore, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

struct TypeIgnoreTypeIgnore {
    range: TextRange,
    lineno: PyRefExact<PyInt>,
    tag: PyRefExact<PyStr>,
}

// constructor
impl Node for TypeIgnoreTypeIgnore {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            lineno,
            tag,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeTypeIgnoreTypeIgnore::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lineno", lineno.to_pyobject(vm), vm).unwrap();
        dict.set_item("tag", tag.to_pyobject(vm), vm).unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            lineno: get_node_field(vm, &object, "lineno", "TypeIgnore")?
                .downcast_exact(vm)
                .unwrap(),
            tag: get_node_field(vm, &object, "tag", "TypeIgnore")?
                .downcast_exact(vm)
                .unwrap(),
            range: Default::default(),
        })
    }
}
impl Node for ruff::TypeParams {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
// sum
impl Node for ruff::TypeParam {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::TypeVar(cons) => cons.ast_to_object(vm),
            Self::ParamSpec(cons) => cons.ast_to_object(vm),
            Self::TypeVarTuple(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeTypeParamTypeVar::static_type()) {
            ruff::TypeParam::TypeVar(ruff::TypeParamTypeVar::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeTypeParamParamSpec::static_type()) {
            ruff::TypeParam::ParamSpec(ruff::TypeParamParamSpec::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeTypeParamTypeVarTuple::static_type()) {
            ruff::TypeParam::TypeVarTuple(ruff::TypeParamTypeVarTuple::ast_from_object(
                _vm, _object,
            )?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::TypeParamTypeVar {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            bound,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeTypeParamTypeVar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("bound", bound.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "TypeVar")?)?,
            bound: get_node_field_opt(_vm, &_object, "bound")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "TypeVar")?,
            )?,
            range: range_from_object(_vm, _object, "TypeVar")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamParamSpec {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("default_value", default.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "ParamSpec")?)?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "ParamSpec")?,
            )?,
            range: range_from_object(_vm, _object, "ParamSpec")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamTypeVarTuple {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodeTypeParamTypeVarTuple::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("default_value", default.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "name", "TypeVarTuple")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "TypeVarTuple")?,
            )?,
            range: range_from_object(_vm, _object, "TypeVarTuple")?,
        })
    }
}

impl Node for ruff::name::Name {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::Decorator {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ElifElseClause {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

#[cfg(feature = "parser")]
pub(crate) fn parse(
    vm: &VirtualMachine,
    source: &str,
    mode: parser::Mode,
) -> Result<PyObjectRef, CompileError> {
    let top = parser::parse(source, mode)?.into_syntax();
    Ok(top.ast_to_object(vm))
}

#[cfg(feature = "codegen")]
pub(crate) fn compile(
    vm: &VirtualMachine,
    object: PyObjectRef,
    filename: &str,
    mode: crate::compiler::Mode,
    optimize: Option<u8>,
) -> PyResult {
    let mut opts = vm.compile_opts();
    if let Some(optimize) = optimize {
        opts.optimize = optimize;
    }

    let ast: self::Mod = Node::ast_from_object(vm, object)?;
    let ast = match ast {
        self::Mod::Module(m) => ruff::Mod::Module(m),
        self::Mod::Interactive(ModInteractive { range, body }) => {
            ruff::Mod::Module(ruff::ModModule { range, body })
        }
        self::Mod::Expression(e) => ruff::Mod::Expression(e),
        self::Mod::FunctionType(_) => todo!(),
    };
    // TODO: create a textual representation of the ast
    let text = "";
    let source_code = SourceCode::new(filename, text);
    let code = compile::compile_top(ast, source_code, mode, opts)
        .map_err(|e| e.into())
        .map_err(|err| (CompileError::from(err), None).to_pyexception(vm))?; // FIXME source
    Ok(vm.ctx.new_code(code).into())
}

// Required crate visibility for inclusion by gen.rs
pub(crate) use _ast::NodeAst;

// Used by builtins::compile()
pub const PY_COMPILE_FLAG_AST_ONLY: i32 = 0x0400;

// The following flags match the values from Include/cpython/compile.h
// Caveat emptor: These flags are undocumented on purpose and depending
// on their effect outside the standard library is **unsupported**.
const PY_CF_DONT_IMPLY_DEDENT: i32 = 0x200;
const PY_CF_ALLOW_INCOMPLETE_INPUT: i32 = 0x4000;

// __future__ flags - sync with Lib/__future__.py
// TODO: These flags aren't being used in rust code
//       CO_FUTURE_ANNOTATIONS does make a difference in the codegen,
//       so it should be used in compile().
//       see compiler/codegen/src/compile.rs
const CO_NESTED: i32 = 0x0010;
const CO_GENERATOR_ALLOWED: i32 = 0;
const CO_FUTURE_DIVISION: i32 = 0x20000;
const CO_FUTURE_ABSOLUTE_IMPORT: i32 = 0x40000;
const CO_FUTURE_WITH_STATEMENT: i32 = 0x80000;
const CO_FUTURE_PRINT_FUNCTION: i32 = 0x100000;
const CO_FUTURE_UNICODE_LITERALS: i32 = 0x200000;
const CO_FUTURE_BARRY_AS_BDFL: i32 = 0x400000;
const CO_FUTURE_GENERATOR_STOP: i32 = 0x800000;
const CO_FUTURE_ANNOTATIONS: i32 = 0x1000000;

// Used by builtins::compile() - the summary of all flags
pub const PY_COMPILE_FLAGS_MASK: i32 = PY_COMPILE_FLAG_AST_ONLY
    | PY_CF_DONT_IMPLY_DEDENT
    | PY_CF_ALLOW_INCOMPLETE_INPUT
    | CO_NESTED
    | CO_GENERATOR_ALLOWED
    | CO_FUTURE_DIVISION
    | CO_FUTURE_ABSOLUTE_IMPORT
    | CO_FUTURE_WITH_STATEMENT
    | CO_FUTURE_PRINT_FUNCTION
    | CO_FUTURE_UNICODE_LITERALS
    | CO_FUTURE_BARRY_AS_BDFL
    | CO_FUTURE_GENERATOR_STOP
    | CO_FUTURE_ANNOTATIONS;

pub fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ast::make_module(vm);
    gen::extend_module_nodes(vm, &module);
    module
}
