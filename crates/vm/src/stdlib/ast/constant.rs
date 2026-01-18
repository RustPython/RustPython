use super::*;
use crate::builtins::{PyComplex, PyFrozenSet, PyTuple};
use ast::str_prefix::StringLiteralPrefix;
use rustpython_compiler_core::SourceFile;

#[derive(Debug)]
pub(super) struct Constant {
    pub(super) range: TextRange,
    pub(super) value: ConstantLiteral,
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
        }
    }

    pub(super) const fn new_int(value: ast::Int, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Int(value),
        }
    }

    pub(super) const fn new_float(value: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Float(value),
        }
    }

    pub(super) const fn new_complex(real: f64, imag: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Complex { real, imag },
        }
    }

    pub(super) const fn new_bytes(value: Box<[u8]>, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bytes(value),
        }
    }

    pub(super) const fn new_bool(value: bool, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bool(value),
        }
    }

    pub(super) const fn new_none(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::None,
        }
    }

    pub(super) const fn new_ellipsis(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Ellipsis,
        }
    }

    pub(crate) fn into_expr(self) -> ast::Expr {
        constant_to_ruff_expr(self)
    }
}

#[derive(Debug)]
pub(crate) enum ConstantLiteral {
    None,
    Bool(bool),
    Str {
        value: Box<str>,
        prefix: StringLiteralPrefix,
    },
    Bytes(Box<[u8]>),
    Int(ast::Int),
    Tuple(Vec<ConstantLiteral>),
    FrozenSet(Vec<ConstantLiteral>),
    Float(f64),
    Complex {
        real: f64,
        imag: f64,
    },
    Ellipsis,
}

// constructor
impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { range, value } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let kind = match &value {
            ConstantLiteral::Str {
                prefix: StringLiteralPrefix::Unicode,
                ..
            } => vm.ctx.new_str("u").into(),
            _ => vm.ctx.none(),
        };
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
        let value_object = get_node_field(vm, &object, "value", "Constant")?;
        let value = Node::ast_from_object(vm, source_file, value_object)?;

        Ok(Self {
            value,
            // kind: get_node_field_opt(_vm, &_object, "kind")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(vm, source_file, object, "Constant")?,
        })
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
                .cloned()
                .map(|object| Node::ast_from_object(vm, source_file, object))
                .collect::<PyResult<_>>()?;
            Self::Tuple(tuple)
        } else if cls.is(vm.ctx.types.frozenset_type) {
            let set = value_object.downcast::<PyFrozenSet>().unwrap();
            let elements = set
                .elements()
                .into_iter()
                .map(|object| Node::ast_from_object(vm, source_file, object))
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

fn constant_to_ruff_expr(value: Constant) -> ast::Expr {
    let Constant { value, range } = value;
    match value {
        ConstantLiteral::None => ast::Expr::NoneLiteral(ast::ExprNoneLiteral {
            node_index: Default::default(),
            range,
        }),
        ConstantLiteral::Bool(value) => ast::Expr::BooleanLiteral(ast::ExprBooleanLiteral {
            node_index: Default::default(),
            range,
            value,
        }),
        ConstantLiteral::Str { value, prefix } => {
            ast::Expr::StringLiteral(ast::ExprStringLiteral {
                node_index: Default::default(),
                range,
                value: ast::StringLiteralValue::single(ast::StringLiteral {
                    node_index: Default::default(),
                    range,
                    value,
                    flags: ast::StringLiteralFlags::empty().with_prefix(prefix),
                }),
            })
        }
        ConstantLiteral::Bytes(value) => {
            ast::Expr::BytesLiteral(ast::ExprBytesLiteral {
                node_index: Default::default(),
                range,
                value: ast::BytesLiteralValue::single(ast::BytesLiteral {
                    node_index: Default::default(),
                    range,
                    value,
                    flags: ast::BytesLiteralFlags::empty(), // TODO
                }),
            })
        }
        ConstantLiteral::Int(value) => ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: Default::default(),
            range,
            value: ast::Number::Int(value),
        }),
        ConstantLiteral::Tuple(value) => ast::Expr::Tuple(ast::ExprTuple {
            node_index: Default::default(),
            range,
            elts: value
                .into_iter()
                .map(|value| {
                    constant_to_ruff_expr(Constant {
                        range: TextRange::default(),
                        value,
                    })
                })
                .collect(),
            ctx: ast::ExprContext::Load,
            // TODO: Does this matter?
            parenthesized: true,
        }),
        ConstantLiteral::FrozenSet(value) => ast::Expr::Call(ast::ExprCall {
            node_index: Default::default(),
            range,
            // idk lol
            func: Box::new(ast::Expr::Name(ast::ExprName {
                node_index: Default::default(),
                range: TextRange::default(),
                id: ast::name::Name::new_static("frozenset"),
                ctx: ast::ExprContext::Load,
            })),
            arguments: ast::Arguments {
                node_index: Default::default(),
                range,
                args: value
                    .into_iter()
                    .map(|value| {
                        constant_to_ruff_expr(Constant {
                            range: TextRange::default(),
                            value,
                        })
                    })
                    .collect(),
                keywords: Box::default(),
            },
        }),
        ConstantLiteral::Float(value) => ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
            node_index: Default::default(),
            range,
            value: ast::Number::Float(value),
        }),
        ConstantLiteral::Complex { real, imag } => {
            ast::Expr::NumberLiteral(ast::ExprNumberLiteral {
                node_index: Default::default(),
                range,
                value: ast::Number::Complex { real, imag },
            })
        }
        ConstantLiteral::Ellipsis => ast::Expr::EllipsisLiteral(ast::ExprEllipsisLiteral {
            node_index: Default::default(),
            range,
        }),
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
    } = constant;
    let c = Constant::new_ellipsis(range);
    c.ast_to_object(vm, source_file)
}
