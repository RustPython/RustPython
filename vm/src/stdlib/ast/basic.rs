use super::*;

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
