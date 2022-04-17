use crate::{
    convert::{ToPyObject, TryFromObject},
    pyobject::{AsObject, PyObjectRef, PyResult},
    VirtualMachine,
};

#[derive(result_like::OptionLike)]
pub enum PyArithmeticValue<T> {
    Implemented(T),
    NotImplemented,
}

impl PyArithmeticValue<PyObjectRef> {
    pub fn from_object(vm: &VirtualMachine, obj: PyObjectRef) -> Self {
        if obj.is(&vm.ctx.not_implemented) {
            Self::NotImplemented
        } else {
            Self::Implemented(obj)
        }
    }
}

impl<T: TryFromObject> TryFromObject for PyArithmeticValue<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        PyArithmeticValue::from_object(vm, obj)
            .map(|x| T::try_from_object(vm, x))
            .transpose()
    }
}

impl<T> ToPyObject for PyArithmeticValue<T>
where
    T: ToPyObject,
{
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            PyArithmeticValue::Implemented(v) => v.to_pyobject(vm),
            PyArithmeticValue::NotImplemented => vm.ctx.not_implemented(),
        }
    }
}

pub type PyComparisonValue = PyArithmeticValue<bool>;
