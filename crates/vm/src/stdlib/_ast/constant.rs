use super::*;
use crate::builtins::{PyComplex, PyFrozenSet, PyTuple};
use ast::str_prefix::StringLiteralPrefix;
use rustpython_codegen::compile::ruff_int_to_bigint;
use rustpython_compiler_core::{SourceFile, bytecode::ConstantData};

pub(super) use ast::ConstantValue as PublicAstConstant;

pub(super) type PublicAstExceptHandlerList = Vec<Option<ast::ExceptHandler>>;
pub(super) type PublicAstExprList = Vec<ast::Expr>;
pub(super) type PublicAstExprOptionList = Vec<Option<ast::Expr>>;
pub(super) type PublicAstPatternList = Vec<Option<ast::Pattern>>;
pub(super) type PublicAstStmtList = Vec<Option<ast::Stmt>>;
pub(super) type PublicAstTypeParamList = Vec<Option<ast::TypeParam>>;

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

    pub(crate) fn into_expr(self, _ctx: Option<&AstFromObjectContext<'_>>) -> ast::Expr {
        let Self {
            range,
            value,
            kind,
            invalid_type,
        } = self;
        ast::Expr::Constant(ast::ExprConstant {
            node_index: Default::default(),
            range,
            value: constant_data_to_public_ast_constant(constant_literal_to_constant_data(&value)),
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

pub(super) fn with_public_ast_context<T>(
    vm: &VirtualMachine,
    f: impl FnOnce(&AstFromObjectContext<'_>) -> PyResult<T>,
) -> PyResult<T> {
    let from_ctx = AstFromObjectContext::new(vm);
    f(&from_ctx)
}

pub(super) fn public_ast_constant(expr: &ast::Expr) -> Option<PublicAstConstant> {
    match expr {
        ast::Expr::Constant(expr) => Some(expr.value.clone()),
        _ => None,
    }
}

pub(super) fn public_ast_invalid_constant_type(expr: &ast::Expr) -> Option<Box<str>> {
    match expr {
        ast::Expr::Constant(expr) => expr.invalid_type.clone(),
        _ => None,
    }
}

pub(super) fn public_ast_string_from_pyobject(
    ctx: &AstFromObjectContext<'_>,
    object: PyObjectRef,
) -> (Option<Box<str>>, Option<Vec<u8>>) {
    public_ast_string_from_object(ctx, object)
}

pub(super) fn public_ast_string_object(
    to_ctx: &AstToObjectContext<'_>,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    public_ast_string_to_object(to_ctx.vm, value, bytes)
}

pub(super) fn expr_constant_to_object(
    to_ctx: &AstToObjectContext<'_>,
    expr: ast::ExprConstant,
) -> PyObjectRef {
    let ctx = to_ctx.vm;
    let source_file = to_ctx.source_file;
    let ast::ExprConstant {
        node_index: _,
        range,
        value,
        kind,
        invalid_type: _,
    } = expr;
    let constant = public_ast_constant_to_constant_data(value);
    let node = NodeAst
        .into_ref_with_type(ctx, pyast::NodeExprConstant::static_type().to_owned())
        .unwrap();
    let dict = node.as_object().dict().unwrap();
    dict.set_item("value", constant_data_to_object(ctx, constant), ctx)
        .unwrap();
    let kind = kind.map_or_else(|| ctx.ctx.none(), |kind| ctx.ctx.new_str(kind).into());
    dict.set_item("kind", kind, ctx).unwrap();
    node_add_location(&dict, range, ctx, source_file);
    node.into()
}

pub(super) fn public_ast_interpolation_object(
    to_ctx: &AstToObjectContext<'_>,
    str: Option<PublicAstConstant>,
    format_spec: Option<Box<ast::Expr>>,
) -> Option<(PyObjectRef, Option<Box<ast::Expr>>)> {
    let str = str?;
    Some((
        constant_data_to_object(to_ctx.vm, public_ast_constant_to_constant_data(str)),
        format_spec,
    ))
}

pub(super) fn public_ast_joined_str_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstExprList>,
) -> Option<PublicAstExprList> {
    value
}

pub(super) fn public_ast_template_str_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstExprList>,
) -> Option<PublicAstExprList> {
    value
}

pub(super) fn public_ast_comprehension_is_async_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<i32>,
) -> Option<i32> {
    value
}

pub(super) fn public_ast_pattern_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstPatternList>,
) -> Option<PublicAstPatternList> {
    value
}

pub(super) fn public_ast_expr_option_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstExprOptionList>,
) -> Option<PublicAstExprOptionList> {
    value
}

pub(super) fn public_ast_expr_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstExprOptionList>,
) -> Option<PublicAstExprOptionList> {
    value
}

pub(super) fn public_ast_stmt_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstStmtList>,
) -> Option<PublicAstStmtList> {
    value
}

pub(super) fn public_ast_except_handler_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstExceptHandlerList>,
) -> Option<PublicAstExceptHandlerList> {
    value
}

pub(super) fn public_ast_type_param_list_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<PublicAstTypeParamList>,
) -> Option<PublicAstTypeParamList> {
    value
}

pub(super) fn public_ast_ann_assign_simple_object(
    _to_ctx: &AstToObjectContext<'_>,
    value: Option<i32>,
) -> Option<i32> {
    value
}

pub(super) fn public_ast_arg_type_comment_object(
    to_ctx: &AstToObjectContext<'_>,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    public_ast_string_object(to_ctx, value, bytes)
}

pub(super) fn public_ast_stmt_type_comment_object(
    to_ctx: &AstToObjectContext<'_>,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> Option<PyObjectRef> {
    public_ast_string_object(to_ctx, value, bytes)
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

pub(super) fn constant_data_to_public_ast_constant(value: ConstantData) -> PublicAstConstant {
    match value {
        ConstantData::None => PublicAstConstant::None,
        ConstantData::Boolean { value } => PublicAstConstant::Boolean(value),
        ConstantData::Str { value } => PublicAstConstant::Str(value.to_string().into_boxed_str()),
        ConstantData::Bytes { value } => PublicAstConstant::Bytes(value.into_boxed_slice()),
        ConstantData::Integer { value } => PublicAstConstant::Integer(value.to_string().into()),
        ConstantData::Tuple { elements } => PublicAstConstant::Tuple(
            elements
                .into_iter()
                .map(constant_data_to_public_ast_constant)
                .collect(),
        ),
        ConstantData::Frozenset { elements } => PublicAstConstant::Frozenset(
            elements
                .into_iter()
                .map(constant_data_to_public_ast_constant)
                .collect(),
        ),
        ConstantData::Float { value } => PublicAstConstant::Float(value),
        ConstantData::Complex { value } => PublicAstConstant::Complex {
            real: value.re,
            imag: value.im,
        },
        ConstantData::Ellipsis => PublicAstConstant::Ellipsis,
        ConstantData::Code { .. } | ConstantData::Slice { .. } => {
            unreachable!("public AST constants cannot contain code objects or slices")
        }
    }
}

pub(super) fn public_ast_constant_to_constant_data(value: PublicAstConstant) -> ConstantData {
    match value {
        PublicAstConstant::None => ConstantData::None,
        PublicAstConstant::Boolean(value) => ConstantData::Boolean { value },
        PublicAstConstant::Str(value) => ConstantData::Str {
            value: value.to_string().into(),
        },
        PublicAstConstant::Bytes(value) => ConstantData::Bytes {
            value: value.into_vec(),
        },
        PublicAstConstant::Integer(value) => ConstantData::Integer {
            value: value
                .parse()
                .expect("RustPython public AST integer constants are decimal integers"),
        },
        PublicAstConstant::Tuple(elements) => ConstantData::Tuple {
            elements: elements
                .into_iter()
                .map(public_ast_constant_to_constant_data)
                .collect(),
        },
        PublicAstConstant::Frozenset(elements) => ConstantData::Frozenset {
            elements: elements
                .into_iter()
                .map(public_ast_constant_to_constant_data)
                .collect(),
        },
        PublicAstConstant::Float(value) => ConstantData::Float { value },
        PublicAstConstant::Complex { real, imag } => ConstantData::Complex {
            value: num_complex::Complex::new(real, imag),
        },
        PublicAstConstant::Ellipsis => ConstantData::Ellipsis,
    }
}

pub(super) fn constant_object_to_constant_data(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    value_object: PyObjectRef,
) -> PyResult<ConstantData> {
    let value = ConstantLiteral::ast_from_object(ctx, source_file, value_object)?;
    Ok(constant_literal_to_constant_data(&value))
}

fn public_ast_string_from_object(
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

fn public_ast_string_to_object(
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

fn first_invalid_constant_type(
    ctx: &AstFromObjectContext<'_>,
    value_object: PyObjectRef,
) -> PyResult<String> {
    let cls = value_object.class();
    let class_name = cls.name().to_owned();
    if cls.is(ctx.ctx.types.tuple_type) {
        ctx.with_recursion(" during compilation", || {
            let tuple = value_object.clone().downcast::<PyTuple>().map_err(|obj| {
                ctx.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            for item in tuple.iter() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(ctx, item.clone())? {
                    return Ok(invalid_type);
                }
            }
            Ok(class_name)
        })
    } else if cls.is(ctx.ctx.types.frozenset_type) {
        ctx.with_recursion(" during compilation", || {
            let set = value_object.clone().downcast::<PyFrozenSet>().unwrap();
            for item in set.elements() {
                if let Some(invalid_type) = first_invalid_constant_type_opt(ctx, item)? {
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
    ctx: &AstFromObjectContext<'_>,
    value_object: PyObjectRef,
) -> PyResult<Option<String>> {
    let cls = value_object.class();
    if cls.is(ctx.ctx.types.none_type)
        || cls.is(ctx.ctx.types.bool_type)
        || cls.is(ctx.ctx.types.str_type)
        || cls.is(ctx.ctx.types.bytes_type)
        || cls.is(ctx.ctx.types.int_type)
        || cls.is(ctx.ctx.types.float_type)
        || cls.is(ctx.ctx.types.complex_type)
        || cls.is(ctx.ctx.types.ellipsis_type)
    {
        return Ok(None);
    }
    if cls.is(ctx.ctx.types.tuple_type) || cls.is(ctx.ctx.types.frozenset_type) {
        return first_invalid_constant_type(ctx, value_object).map(Some);
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
            unreachable!("public AST constants cannot contain code objects or slices")
        }
    }
}

// constructor
pub(super) fn constant_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<Constant> {
    let value_object = get_node_field(ctx, &object, "value", "Constant")?;
    let (value, invalid_type) =
        match ConstantLiteral::ast_from_object(ctx, source_file, value_object.clone()) {
            Ok(value) => (value, None),
            Err(_) => (
                ConstantLiteral::None,
                Some(first_invalid_constant_type(ctx, value_object)?),
            ),
        };
    let kind = get_node_field_opt(ctx, &object, "kind")?
        .map(|object| {
            if !object.class().is(ctx.ctx.types.str_type) {
                return Err(ctx.new_type_error("AST string must be of type str"));
            }
            Ok(object.try_to_value::<String>(ctx)?.into_boxed_str())
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
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            range,
            value,
            kind,
            invalid_type: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let kind = kind
            .or_else(|| constant_literal_kind(&value))
            .map_or_else(|| ctx.ctx.none(), |kind| ctx.ctx.new_str(kind).into());
        let value = value.ast_to_object(to_ctx);
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value, ctx).unwrap();
        dict.set_item("kind", kind, ctx).unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Constant")?;
        constant_from_object_with_range(ctx, source_file, object, range)
    }
}

impl Node for ConstantLiteral {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        match self {
            Self::None => ctx.ctx.none(),
            Self::Bool(value) => ctx.ctx.new_bool(value).to_pyobject(ctx),
            Self::Str { value, .. } => ctx.ctx.new_str(value).to_pyobject(ctx),
            Self::Bytes(value) => ctx.ctx.new_bytes(value.into()).to_pyobject(ctx),
            Self::Int(value) => value.ast_to_object(to_ctx),
            Self::Tuple(value) => {
                let value = value.into_iter().map(|c| c.ast_to_object(to_ctx)).collect();
                ctx.ctx.new_tuple(value).to_pyobject(ctx)
            }
            Self::FrozenSet(value) => {
                PyFrozenSet::from_iter(ctx, value.into_iter().map(|c| c.ast_to_object(to_ctx)))
                    .unwrap()
                    .into_pyobject(ctx)
            }
            Self::Float(value) => ctx.ctx.new_float(value).into_pyobject(ctx),
            Self::Complex { real, imag } => ctx
                .ctx
                .new_complex(num_complex::Complex::new(real, imag))
                .into_pyobject(ctx),
            Self::Ellipsis => ctx.ctx.ellipsis.clone().into(),
        }
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        value_object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = value_object.class();
        let value = if cls.is(ctx.ctx.types.none_type) {
            Self::None
        } else if cls.is(ctx.ctx.types.bool_type) {
            Self::Bool(if value_object.is(&ctx.ctx.true_value) {
                true
            } else if value_object.is(&ctx.ctx.false_value) {
                false
            } else {
                value_object.try_to_value(ctx)?
            })
        } else if cls.is(ctx.ctx.types.str_type) {
            Self::Str {
                value: value_object.try_to_value::<String>(ctx)?.into(),
                prefix: StringLiteralPrefix::Empty,
            }
        } else if cls.is(ctx.ctx.types.bytes_type) {
            Self::Bytes(value_object.try_to_value::<Vec<u8>>(ctx)?.into())
        } else if cls.is(ctx.ctx.types.int_type) {
            Self::Int(Node::ast_from_object(ctx, source_file, value_object)?)
        } else if cls.is(ctx.ctx.types.tuple_type) {
            let tuple = value_object.downcast::<PyTuple>().map_err(|obj| {
                ctx.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            let tuple = tuple
                .into_iter()
                .map(|object| {
                    let object = object.clone();
                    ctx.with_recursion("during compilation", || {
                        Node::ast_from_object(ctx, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::Tuple(tuple)
        } else if cls.is(ctx.ctx.types.frozenset_type) {
            let set = value_object.downcast::<PyFrozenSet>().unwrap();
            let elements = set
                .elements()
                .into_iter()
                .map(|object| {
                    ctx.with_recursion("during compilation", || {
                        Node::ast_from_object(ctx, source_file, object)
                    })
                })
                .collect::<PyResult<_>>()?;
            Self::FrozenSet(elements)
        } else if cls.is(ctx.ctx.types.float_type) {
            let float = value_object.try_into_value(ctx)?;
            Self::Float(float)
        } else if cls.is(ctx.ctx.types.complex_type) {
            let complex = value_object.try_complex(ctx)?;
            let complex = match complex {
                None => {
                    return Err(ctx.new_type_error(format!(
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
        } else if cls.is(ctx.ctx.types.ellipsis_type) {
            Self::Ellipsis
        } else {
            return Err(ctx.new_type_error(format!(
                "got an invalid type in Constant: {}",
                value_object.class().name()
            )));
        };
        Ok(value)
    }
}

pub(super) fn number_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
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
    c.ast_to_object(to_ctx)
}

pub(super) fn string_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
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
    c.ast_to_object(to_ctx)
}

pub(super) fn bytes_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
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
    c.ast_to_object(to_ctx)
}

pub(super) fn boolean_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
    constant: ast::ExprBooleanLiteral,
) -> PyObjectRef {
    let ast::ExprBooleanLiteral {
        node_index: _,
        range,
        value,
        ..
    } = constant;
    let c = Constant::new_bool(value, range);
    c.ast_to_object(to_ctx)
}

pub(super) fn none_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
    constant: ast::ExprNoneLiteral,
) -> PyObjectRef {
    let ast::ExprNoneLiteral {
        node_index: _,
        range,
        ..
    } = constant;
    let c = Constant::new_none(range);
    c.ast_to_object(to_ctx)
}

pub(super) fn ellipsis_literal_to_object(
    to_ctx: &AstToObjectContext<'_>,
    constant: ast::ExprEllipsisLiteral,
) -> PyObjectRef {
    let ast::ExprEllipsisLiteral {
        node_index: _,
        range,
        ..
    } = constant;
    let c = Constant::new_ellipsis(range);
    c.ast_to_object(to_ctx)
}
