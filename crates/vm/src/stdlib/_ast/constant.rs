use super::*;
use crate::builtins::{PyComplex, PyFrozenSet, PyTuple};
use ast::str_prefix::StringLiteralPrefix;
use rustpython_codegen::compile::ruff_int_to_bigint;
use rustpython_compiler_core::{SourceFile, bytecode::ConstantData};

#[derive(Debug)]
pub(super) struct Constant {
    pub(super) range: TextRange,
    pub(super) value: ConstantLiteral,
    kind: Option<Box<str>>,
    invalid_type: Option<String>,
}

impl Constant {
    pub(super) fn new_str(
        value: impl Into<Box<str>>,
        prefix: StringLiteralPrefix,
        range: TextRange,
    ) -> Self {
        let value = value.into();
        Self {
            range,
            value: ConstantLiteral::Str { value, prefix },
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_int(value: ast::Int, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Int(value),
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_float(value: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Float(value),
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_complex(real: f64, imag: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Complex { real, imag },
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_bytes(value: Box<[u8]>, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bytes(value),
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_bool(value: bool, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bool(value),
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_none(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::None,
            kind: None,
            invalid_type: None,
        }
    }

    pub(super) const fn new_ellipsis(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Ellipsis,
            kind: None,
            invalid_type: None,
        }
    }

    pub(crate) fn into_expr(self) -> ast::Expr {
        let Self {
            range,
            value,
            kind,
            invalid_type,
        } = self;
        ast::Expr::Constant(ast::ExprConstant {
            node_index: Default::default(),
            range,
            value: constant_data_to_ast_constant_value(constant_literal_to_constant_data(&value)),
            kind: kind.or_else(|| constant_literal_kind(&value)),
            invalid_type: invalid_type.map(String::into_boxed_str),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ConstantLiteral {
    None,
    Bool(bool),
    Str {
        value: Box<str>,
        prefix: StringLiteralPrefix,
    },
    Bytes(Box<[u8]>),
    Int(ast::Int),
    Tuple(Vec<Self>),
    FrozenSet(Vec<Self>),
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    Ellipsis,
}

pub(super) fn invalid_constant_type(expr: &ast::Expr) -> Option<Box<str>> {
    match expr {
        ast::Expr::Constant(expr) => expr.invalid_type.clone(),
        _ => None,
    }
}

pub(super) fn runtime_string_from_pyobject(
    vm: &VirtualMachine,
    object: PyObjectRef,
) -> (Option<Box<str>>, Option<Vec<u8>>) {
    runtime_string_from_object(vm, object)
}

pub(super) fn runtime_string_object(
    vm: &VirtualMachine,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    runtime_string_to_object(vm, value, bytes)
}

pub(super) fn expr_constant_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    expr: ast::ExprConstant,
) -> PyObjectRef {
    let ast::ExprConstant {
        node_index: _,
        range,
        value,
        kind,
        invalid_type: _,
    } = expr;
    let constant = ast_constant_value_to_constant_data(value);
    let node = NodeAst
        .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();
    dict.set_item("value", constant_data_to_object(vm, constant), vm)
        .unwrap();
    let kind = kind.map_or_else(|| vm.ctx.none(), |kind| vm.ctx.new_str(kind).into());
    dict.set_item("kind", kind, vm).unwrap();
    node_add_location(&dict, range, vm, source_file);
    node.into()
}

pub(super) fn runtime_interpolation_object(
    vm: &VirtualMachine,
    str: Option<ast::ConstantValue>,
    format_spec: Option<Box<ast::Expr>>,
) -> Option<(PyObjectRef, Option<Box<ast::Expr>>)> {
    let str = str?;
    Some((
        constant_data_to_object(vm, ast_constant_value_to_constant_data(str)),
        format_spec,
    ))
}

pub(super) fn runtime_stmt_type_comment_object(
    vm: &VirtualMachine,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    runtime_string_object(vm, value, bytes)
}

fn constant_literal_to_constant_data(value: &ConstantLiteral) -> ConstantData {
    match value {
        ConstantLiteral::None => ConstantData::None,
        ConstantLiteral::Bool(value) => ConstantData::Boolean { value: *value },
        ConstantLiteral::Str { value, .. } => ConstantData::Str {
            value: value.as_ref().into(),
        },
        ConstantLiteral::Bytes(value) => ConstantData::Bytes {
            value: value.to_vec(),
        },
        ConstantLiteral::Int(value) => ConstantData::Integer {
            value: ruff_int_to_bigint(value).unwrap(),
        },
        ConstantLiteral::Tuple(value) => ConstantData::Tuple {
            elements: value
                .iter()
                .map(constant_literal_to_constant_data)
                .collect(),
        },
        ConstantLiteral::FrozenSet(value) => ConstantData::Frozenset {
            elements: value
                .iter()
                .map(constant_literal_to_constant_data)
                .collect(),
        },
        ConstantLiteral::Float(value) => ConstantData::Float { value: *value },
        ConstantLiteral::Complex { real, imag } => ConstantData::Complex {
            value: num_complex::Complex::new(*real, *imag),
        },
        ConstantLiteral::Ellipsis => ConstantData::Ellipsis,
    }
}

fn constant_literal_kind(value: &ConstantLiteral) -> Option<Box<str>> {
    match value {
        ConstantLiteral::Str {
            prefix: StringLiteralPrefix::Unicode,
            ..
        } => Some("u".into()),
        _ => None,
    }
}

pub(super) fn constant_data_to_ast_constant_value(value: ConstantData) -> ast::ConstantValue {
    match value {
        ConstantData::None => ast::ConstantValue::None,
        ConstantData::Boolean { value } => ast::ConstantValue::Boolean(value),
        ConstantData::Str { value } => ast::ConstantValue::Str(value.to_string().into_boxed_str()),
        ConstantData::Bytes { value } => ast::ConstantValue::Bytes(value.into_boxed_slice()),
        ConstantData::Integer { value } => ast::ConstantValue::Integer(value.to_string().into()),
        ConstantData::Tuple { elements } => ast::ConstantValue::Tuple(
            elements
                .into_iter()
                .map(constant_data_to_ast_constant_value)
                .collect(),
        ),
        ConstantData::Frozenset { elements } => ast::ConstantValue::Frozenset(
            elements
                .into_iter()
                .map(constant_data_to_ast_constant_value)
                .collect(),
        ),
        ConstantData::Float { value } => ast::ConstantValue::Float(value),
        ConstantData::Complex { value } => ast::ConstantValue::Complex {
            real: value.re,
            imag: value.im,
        },
        ConstantData::Ellipsis => ast::ConstantValue::Ellipsis,
        ConstantData::Code { .. } | ConstantData::Slice { .. } => {
            unreachable!("ast.Constant values cannot contain code objects or slices")
        }
    }
}

pub(super) fn ast_constant_value_to_constant_data(value: ast::ConstantValue) -> ConstantData {
    match value {
        ast::ConstantValue::None => ConstantData::None,
        ast::ConstantValue::Boolean(value) => ConstantData::Boolean { value },
        ast::ConstantValue::Str(value) => ConstantData::Str {
            value: value.to_string().into(),
        },
        ast::ConstantValue::Bytes(value) => ConstantData::Bytes {
            value: value.into_vec(),
        },
        ast::ConstantValue::Integer(value) => ConstantData::Integer {
            value: value
                .parse()
                .expect("RustPython ast.Constant integer values are decimal integers"),
        },
        ast::ConstantValue::Tuple(elements) => ConstantData::Tuple {
            elements: elements
                .into_iter()
                .map(ast_constant_value_to_constant_data)
                .collect(),
        },
        ast::ConstantValue::Frozenset(elements) => ConstantData::Frozenset {
            elements: elements
                .into_iter()
                .map(ast_constant_value_to_constant_data)
                .collect(),
        },
        ast::ConstantValue::Float(value) => ConstantData::Float { value },
        ast::ConstantValue::Complex { real, imag } => ConstantData::Complex {
            value: num_complex::Complex::new(real, imag),
        },
        ast::ConstantValue::Ellipsis => ConstantData::Ellipsis,
    }
}

pub(super) fn constant_object_to_constant_data(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    value_object: PyObjectRef,
) -> PyResult<ConstantData> {
    let value = ConstantLiteral::ast_from_object(vm, source_file, value_object)?;
    Ok(constant_literal_to_constant_data(&value))
}

fn runtime_string_from_object(
    vm: &VirtualMachine,
    object: PyObjectRef,
) -> (Option<Box<str>>, Option<Vec<u8>>) {
    if object.class().is(vm.ctx.types.str_type) {
        (
            Some(
                object
                    .try_to_value::<String>(vm)
                    .expect("AST string field was validated as str")
                    .into_boxed_str(),
            ),
            None,
        )
    } else {
        (
            None,
            Some(
                object
                    .try_to_value::<Vec<u8>>(vm)
                    .expect("AST string field was validated as bytes"),
            ),
        )
    }
}

fn runtime_string_to_object(
    vm: &VirtualMachine,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    if let Some(bytes) = bytes {
        Some(vm.ctx.new_bytes(bytes).into())
    } else {
        value.map(|value| vm.ctx.new_str(value).into())
    }
}

fn first_invalid_constant_type(vm: &VirtualMachine, value_object: PyObjectRef) -> PyResult<String> {
    let cls = value_object.class();
    let class_name = cls.name().to_owned();
    if cls.is(vm.ctx.types.tuple_type) {
        vm.with_recursion(" during compilation", || {
            let tuple = value_object.clone().downcast::<PyTuple>().map_err(|obj| {
                vm.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            for item in tuple.iter() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(vm, item.clone())? {
                    return Ok(invalid_type);
                }
            }
            Ok(class_name)
        })
    } else if cls.is(vm.ctx.types.frozenset_type) {
        vm.with_recursion(" during compilation", || {
            let set = value_object.clone().downcast::<PyFrozenSet>().unwrap();
            for item in set.elements() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(vm, item)? {
                    return Ok(invalid_type);
                }
            }
            Ok(class_name)
        })
    } else {
        Ok(class_name)
    }
}

fn first_invalid_constant_type_opt(
    vm: &VirtualMachine,
    value_object: PyObjectRef,
) -> PyResult<Option<String>> {
    let cls = value_object.class();
    if cls.is(vm.ctx.types.none_type)
        || cls.is(vm.ctx.types.bool_type)
        || cls.is(vm.ctx.types.str_type)
        || cls.is(vm.ctx.types.bytes_type)
        || cls.is(vm.ctx.types.int_type)
        || cls.is(vm.ctx.types.float_type)
        || cls.is(vm.ctx.types.complex_type)
        || cls.is(vm.ctx.types.ellipsis_type)
    {
        return Ok(None);
    }
    if cls.is(vm.ctx.types.tuple_type) || cls.is(vm.ctx.types.frozenset_type) {
        return first_invalid_constant_type(vm, value_object).map(Some);
    }
    Ok(Some(cls.name().to_owned()))
}

fn constant_data_to_object(vm: &VirtualMachine, constant: ConstantData) -> PyObjectRef {
    match constant {
        ConstantData::None => vm.ctx.none(),
        ConstantData::Boolean { value } => vm.ctx.new_bool(value).to_pyobject(vm),
        ConstantData::Str { value } => vm.ctx.new_str(value.to_string()).to_pyobject(vm),
        ConstantData::Bytes { value } => vm.ctx.new_bytes(value).to_pyobject(vm),
        ConstantData::Integer { value } => vm.ctx.new_int(value).into(),
        ConstantData::Tuple { elements } => {
            let value = elements
                .into_iter()
                .map(|c| constant_data_to_object(vm, c))
                .collect();
            vm.ctx.new_tuple(value).to_pyobject(vm)
        }
        ConstantData::Frozenset { elements } => PyFrozenSet::from_iter(
            vm,
            elements.into_iter().map(|c| constant_data_to_object(vm, c)),
        )
        .unwrap()
        .into_pyobject(vm),
        ConstantData::Float { value } => vm.ctx.new_float(value).into_pyobject(vm),
        ConstantData::Complex { value } => vm.ctx.new_complex(value).into_pyobject(vm),
        ConstantData::Ellipsis => vm.ctx.ellipsis.clone().into(),
        ConstantData::Code { .. } | ConstantData::Slice { .. } => {
            unreachable!("ast.Constant values cannot contain code objects or slices")
        }
    }
}

// constructor
pub(super) fn constant_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<Constant> {
    let value_object = get_node_field(vm, &object, "value", "Constant")?;
    let (value, invalid_type) =
        match ConstantLiteral::ast_from_object(vm, source_file, value_object.clone()) {
            Ok(value) => (value, None),
            Err(_) => (
                ConstantLiteral::None,
                Some(first_invalid_constant_type(vm, value_object)?),
            ),
        };
    let kind = get_node_field_opt(vm, &object, "kind")?
        .map(|object| {
            if !object.class().is(vm.ctx.types.str_type) {
                return Err(vm.new_type_error("AST string must be of type str"));
            }
            Ok(object.try_to_value::<String>(vm)?.into_boxed_str())
        })
        .transpose()?;

    Ok(Constant {
        range,
        value,
        kind,
        invalid_type,
    })
}

impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            range,
            value,
            kind,
            invalid_type: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let kind = kind
            .or_else(|| constant_literal_kind(&value))
            .map_or_else(|| vm.ctx.none(), |kind| vm.ctx.new_str(kind).into());
        let value = value.ast_to_object(vm, source_file);
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value, vm).unwrap();
        dict.set_item("kind", kind, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "Constant")?;
        constant_from_object_with_range(vm, source_file, object, range)
    }
}

impl Node for ConstantLiteral {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::None => vm.ctx.none(),
            Self::Bool(value) => vm.ctx.new_bool(value).to_pyobject(vm),
            Self::Str { value, .. } => vm.ctx.new_str(value).to_pyobject(vm),
            Self::Bytes(value) => vm.ctx.new_bytes(value.into()).to_pyobject(vm),
            Self::Int(value) => value.ast_to_object(vm, source_file),
            Self::Tuple(value) => {
                let value = value
                    .into_iter()
                    .map(|c| c.ast_to_object(vm, source_file))
                    .collect();
                vm.ctx.new_tuple(value).to_pyobject(vm)
            }
            Self::FrozenSet(value) => PyFrozenSet::from_iter(
                vm,
                value.into_iter().map(|c| c.ast_to_object(vm, source_file)),
            )
            .unwrap()
            .into_pyobject(vm),
            Self::Float(value) => vm.ctx.new_float(value).into_pyobject(vm),
            Self::Complex { real, imag } => vm
                .ctx
                .new_complex(num_complex::Complex::new(real, imag))
                .into_pyobject(vm),
            Self::Ellipsis => vm.ctx.ellipsis.clone().into(),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        value_object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = value_object.class();
        let value = if cls.is(vm.ctx.types.none_type) {
            Self::None
        } else if cls.is(vm.ctx.types.bool_type) {
            Self::Bool(if value_object.is(&vm.ctx.true_value) {
                true
            } else if value_object.is(&vm.ctx.false_value) {
                false
            } else {
                value_object.try_to_value(vm)?
            })
        } else if cls.is(vm.ctx.types.str_type) {
            Self::Str {
                value: value_object.try_to_value::<String>(vm)?.into(),
                prefix: StringLiteralPrefix::Empty,
            }
        } else if cls.is(vm.ctx.types.bytes_type) {
            Self::Bytes(value_object.try_to_value::<Vec<u8>>(vm)?.into())
        } else if cls.is(vm.ctx.types.int_type) {
            Self::Int(Node::ast_from_object(vm, source_file, value_object)?)
        } else if cls.is(vm.ctx.types.tuple_type) {
            let tuple = value_object.downcast::<PyTuple>().map_err(|obj| {
                vm.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            let tuple = tuple
                .into_iter()
                .map(|object| {
                    let object = object.clone();
                    vm.with_recursion(" during compilation", || {
                        Node::ast_from_object(vm, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::Tuple(tuple)
        } else if cls.is(vm.ctx.types.frozenset_type) {
            let set = value_object.downcast::<PyFrozenSet>().unwrap();
            let elements = set
                .elements()
                .into_iter()
                .map(|object| {
                    vm.with_recursion(" during compilation", || {
                        Node::ast_from_object(vm, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::FrozenSet(elements)
        } else if cls.is(vm.ctx.types.float_type) {
            let float = value_object.try_into_value(vm)?;
            Self::Float(float)
        } else if cls.is(vm.ctx.types.complex_type) {
            let complex = value_object.try_complex(vm)?;
            let complex = match complex {
                None => {
                    return Err(vm.new_type_error(format!(
                        "Expected type {}, not {}",
                        PyComplex::static_type().name(),
                        value_object.class().name()
                    )));
                }
                Some((value, _was_coerced)) => value,
            };
            Self::Complex {
                real: complex.re,
                imag: complex.im,
            }
        } else if cls.is(vm.ctx.types.ellipsis_type) {
            Self::Ellipsis
        } else {
            return Err(vm.new_type_error(format!(
                "got an invalid type in Constant: {}",
                value_object.class().name()
            )));
        };
        Ok(value)
    }
}

pub(super) fn number_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprNumberLiteral,
) -> PyObjectRef {
    let ast::ExprNumberLiteral {
        node_index: _,
        range,
        value,
        ..
    } = constant;
    let c = match value {
        ast::Number::Int(n) => Constant::new_int(n, range),
        ast::Number::Float(n) => Constant::new_float(n, range),
        ast::Number::Complex { real, imag } => Constant::new_complex(real, imag, range),
    };
    c.ast_to_object(vm, source_file)
}

pub(super) fn string_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprStringLiteral,
) -> PyObjectRef {
    let ast::ExprStringLiteral {
        node_index: _,
        range,
        value,
        ..
    } = constant;
    let prefix = value
        .iter()
        .next()
        .map_or(StringLiteralPrefix::Empty, |part| part.flags.prefix());
    let c = Constant::new_str(value.to_str(), prefix, range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn bytes_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprBytesLiteral,
) -> PyObjectRef {
    let ast::ExprBytesLiteral {
        node_index: _,
        range,
        value,
        ..
    } = constant;
    let bytes = value.as_slice().iter().flat_map(|b| b.value.iter());
    let c = Constant::new_bytes(bytes.copied().collect(), range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn boolean_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprBooleanLiteral,
) -> PyObjectRef {
    let ast::ExprBooleanLiteral {
        node_index: _,
        range,
        value,
        ..
    } = constant;
    let c = Constant::new_bool(value, range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn none_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprNoneLiteral,
) -> PyObjectRef {
    let ast::ExprNoneLiteral {
        node_index: _,
        range,
        ..
    } = constant;
    let c = Constant::new_none(range);
    c.ast_to_object(vm, source_file)
}

pub(super) fn ellipsis_literal_to_object(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    constant: ast::ExprEllipsisLiteral,
) -> PyObjectRef {
    let ast::ExprEllipsisLiteral {
        node_index: _,
        range,
        ..
    } = constant;
    let c = Constant::new_ellipsis(range);
    c.ast_to_object(vm, source_file)
}
