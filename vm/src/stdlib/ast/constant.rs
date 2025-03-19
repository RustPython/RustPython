use super::*;
use crate::builtins::{PyComplex, PyTuple};

#[derive(Debug)]
pub(super) struct Constant {
    pub(super) range: TextRange,
    pub(super) value: ConstantLiteral,
}

impl Constant {
    pub(super) fn new_string(value: String, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Str(value),
        }
    }

    pub(super) fn new_str(value: &str, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Str(value.to_string()),
        }
    }

    pub(super) fn new_int(value: ruff::Int, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Int(value),
        }
    }

    pub(super) fn new_float(value: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Float(value),
        }
    }
    pub(super) fn new_complex(real: f64, imag: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Complex { real, imag },
        }
    }

    pub(super) fn new_bytes(value: impl Iterator<Item = u8>, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bytes(value.collect()),
        }
    }

    pub(super) fn new_bool(value: bool, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bool(value),
        }
    }

    pub(super) fn new_none(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::None,
        }
    }

    pub(super) fn new_ellipsis(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Ellipsis,
        }
    }

    pub(crate) fn into_expr(self) -> ruff::Expr {
        constant_to_ruff_expr(self)
    }
}

#[derive(Debug)]
pub(crate) enum ConstantLiteral {
    None,
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Int(ruff::Int),
    Tuple {
        value: Vec<Constant>,
        ctx: ruff::ExprContext,
    },
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    Ellipsis,
}

// constructor
impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { range, value } = self;
        let mut string_kind = None;
        let mut tuple_ctx = None;
        let value = match value {
            ConstantLiteral::None => vm.ctx.none(),
            ConstantLiteral::Bool(value) => vm.ctx.new_bool(value).to_pyobject(vm),
            ConstantLiteral::Str(value) => {
                string_kind = Some(!value.is_ascii());
                vm.ctx.new_str(value).to_pyobject(vm)
            }
            ConstantLiteral::Bytes(value) => vm.ctx.new_bytes(value).to_pyobject(vm),
            ConstantLiteral::Int(value) => value.ast_to_object(vm, source_code),
            ConstantLiteral::Tuple { value, ctx } => {
                tuple_ctx = Some(ctx.ast_to_object(vm, source_code));
                let value = value
                    .into_iter()
                    .map(|c| c.ast_to_object(vm, source_code))
                    .collect();
                vm.ctx.new_tuple(value).to_pyobject(vm)
            }
            ConstantLiteral::Float(value) => vm.ctx.new_float(value).into_pyobject(vm),
            ConstantLiteral::Complex { real, imag } => vm
                .ctx
                .new_complex(num_complex::Complex::new(real, imag))
                .into_pyobject(vm),
            ConstantLiteral::Ellipsis => vm.ctx.ellipsis(),
        };
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value, vm).unwrap();
        if let Some(is_unicode_str) = string_kind {
            // TODO: Figure out how this works
            let kind = if is_unicode_str {
                vm.ctx.new_str("u").to_pyobject(vm)
            } else {
                vm.ctx.empty_str.to_pyobject(vm)
            };
            dict.set_item("kind", kind, vm).unwrap();
        }
        if let Some(tuple_ctx) = tuple_ctx {
            dict.set_item("ctx", tuple_ctx, vm).unwrap();
        }
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let value_object = get_node_field(vm, &object, "value", "Constant")?;
        let cls = value_object.class();
        let value = if cls.is(vm.ctx.types.none_type) {
            ConstantLiteral::None
        } else if cls.is(vm.ctx.types.bool_type) {
            ConstantLiteral::Bool(if value_object.is(&vm.ctx.true_value) {
                true
            } else if value_object.is(&vm.ctx.false_value) {
                false
            } else {
                value_object.try_to_value(vm)?
            })
        } else if cls.is(vm.ctx.types.str_type) {
            ConstantLiteral::Str(value_object.try_to_value(vm)?)
        } else if cls.is(vm.ctx.types.bytes_type) {
            ConstantLiteral::Bytes(value_object.try_to_value(vm)?)
        } else if cls.is(vm.ctx.types.int_type) {
            ConstantLiteral::Int(Node::ast_from_object(vm, source_code, value_object)?)
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
                .cloned()
                .map(|object| Node::ast_from_object(vm, source_code, object))
                .collect::<PyResult<_>>()?;
            let ctx_object = get_node_field(vm, &object, "ctx", "Constant")?;
            let ctx_object = Node::ast_from_object(vm, source_code, ctx_object)?;
            ConstantLiteral::Tuple {
                value: tuple,
                ctx: ctx_object,
            }
        } else if cls.is(vm.ctx.types.float_type) {
            let float = value_object.try_into_value(vm)?;
            ConstantLiteral::Float(float)
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
            ConstantLiteral::Complex {
                real: complex.re,
                imag: complex.im,
            }
        } else if cls.is(vm.ctx.types.ellipsis_type) {
            ConstantLiteral::Ellipsis
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                value_object.repr(vm)?
            )));
        };

        Ok(Self {
            value,
            // kind: get_node_field_opt(_vm, &_object, "kind")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(vm, source_code, object, "Constant")?,
        })
    }
}

fn constant_to_ruff_expr(value: Constant) -> ruff::Expr {
    let Constant { value, range } = value;
    match value {
        ConstantLiteral::None => ruff::Expr::NoneLiteral(ruff::ExprNoneLiteral { range }),
        ConstantLiteral::Bool(value) => {
            ruff::Expr::BooleanLiteral(ruff::ExprBooleanLiteral { range, value })
        }
        ConstantLiteral::Str(value) => {
            ruff::Expr::StringLiteral(ruff::ExprStringLiteral {
                range,
                value: ruff::StringLiteralValue::single(ruff::StringLiteral {
                    range,
                    value: value.into(),
                    flags: Default::default(), // TODO
                }),
            })
        }
        ConstantLiteral::Bytes(value) => {
            ruff::Expr::BytesLiteral(ruff::ExprBytesLiteral {
                range,
                value: ruff::BytesLiteralValue::single(ruff::BytesLiteral {
                    range,
                    value: value.into(),
                    flags: Default::default(), // TODO
                }),
            })
        }
        ConstantLiteral::Int(value) => ruff::Expr::NumberLiteral(ruff::ExprNumberLiteral {
            range,
            value: ruff::Number::Int(value),
        }),
        ConstantLiteral::Tuple { value, ctx } => ruff::Expr::Tuple(ruff::ExprTuple {
            range,
            elts: value.into_iter().map(constant_to_ruff_expr).collect(),
            ctx,
            // TODO: Does this matter?
            parenthesized: true,
        }),
        ConstantLiteral::Float(value) => ruff::Expr::NumberLiteral(ruff::ExprNumberLiteral {
            range,
            value: ruff::Number::Float(value),
        }),
        ConstantLiteral::Complex { real, imag } => {
            ruff::Expr::NumberLiteral(ruff::ExprNumberLiteral {
                range,
                value: ruff::Number::Complex { real, imag },
            })
        }
        ConstantLiteral::Ellipsis => {
            ruff::Expr::EllipsisLiteral(ruff::ExprEllipsisLiteral { range })
        }
    }
}

pub(super) fn number_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprNumberLiteral,
) -> PyObjectRef {
    let ruff::ExprNumberLiteral { range, value } = constant;
    let c = match value {
        ruff::Number::Int(n) => Constant::new_int(n, range),
        ruff::Number::Float(n) => Constant::new_float(n, range),
        ruff::Number::Complex { real, imag } => Constant::new_complex(real, imag, range),
    };
    c.ast_to_object(vm, source_code)
}

pub(super) fn string_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprStringLiteral,
) -> PyObjectRef {
    let ruff::ExprStringLiteral { range, value } = constant;
    let c = Constant::new_str(value.to_str(), range);
    c.ast_to_object(vm, source_code)
}

pub(super) fn bytes_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprBytesLiteral,
) -> PyObjectRef {
    let ruff::ExprBytesLiteral { range, value } = constant;
    let bytes = value.as_slice().iter().flat_map(|b| b.value.iter());
    let c = Constant::new_bytes(bytes.copied(), range);
    c.ast_to_object(vm, source_code)
}

pub(super) fn boolean_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprBooleanLiteral,
) -> PyObjectRef {
    let ruff::ExprBooleanLiteral { range, value } = constant;
    let c = Constant::new_bool(value, range);
    c.ast_to_object(vm, source_code)
}

pub(super) fn none_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprNoneLiteral,
) -> PyObjectRef {
    let ruff::ExprNoneLiteral { range } = constant;
    let c = Constant::new_none(range);
    c.ast_to_object(vm, source_code)
}

pub(super) fn ellipsis_literal_to_object(
    vm: &VirtualMachine,
    source_code: &SourceCodeOwned,
    constant: ruff::ExprEllipsisLiteral,
) -> PyObjectRef {
    let ruff::ExprEllipsisLiteral { range } = constant;
    let c = Constant::new_ellipsis(range);
    c.ast_to_object(vm, source_code)
}
