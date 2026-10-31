use core::{fmt, marker::PhantomData};

use crate::{
    PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PySet,
    object::{Traverse, TraverseFn},
};

use super::ToPyObject;

/// A Python set constructed from Rust values.
///
/// Elements use Python hashing and equality, including duplicate elimination.
/// Construction can execute Python code and propagates its exceptions. Cloning
/// this handle shares the same set, including subsequent mutations.
///
/// `T` describes the input conversion, not a runtime invariant: Python can add
/// other types. Access through [`Self::as_set`] is therefore untyped.
pub struct PySetOf<T> {
    set: PyRef<PySet>,
    types: PhantomData<fn() -> T>,
}

impl<T: ToPyObject> PySetOf<T> {
    pub fn try_new(items: impl IntoIterator<Item = T>, vm: &VirtualMachine) -> PyResult<Self> {
        let set = PySet::default().into_ref(&vm.ctx);
        for item in items {
            set.add(item.to_pyobject(vm), vm)?;
        }
        Ok(Self {
            set,
            types: PhantomData,
        })
    }
}

impl<T> PySetOf<T> {
    #[must_use]
    pub fn as_set(&self) -> &PyRef<PySet> {
        &self.set
    }

    #[must_use]
    pub fn into_set(self) -> PyRef<PySet> {
        self.set
    }
}

impl<T> Clone for PySetOf<T> {
    fn clone(&self) -> Self {
        Self {
            set: self.set.clone(),
            types: PhantomData,
        }
    }
}

impl<T> fmt::Debug for PySetOf<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.set.fmt(f)
    }
}

impl<T> ToPyObject for PySetOf<T> {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.set.into()
    }
}

impl<T> From<PySetOf<T>> for PyObjectRef {
    fn from(value: PySetOf<T>) -> Self {
        value.set.into()
    }
}

// SAFETY: Only `set` owns a Python reference; visit it once without cloning.
unsafe impl<T> Traverse for PySetOf<T> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.set.traverse(tracer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsObject, Interpreter};

    #[test]
    fn elements_use_python_equality_and_propagate_errors() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let first: PyObjectRef = vm.ctx.new_bool(true).into();
            let set = PySetOf::try_new([first.clone(), vm.ctx.new_int(1).into()], vm).unwrap();
            let elements = set.as_set().elements();
            assert_eq!(elements.len(), 1);
            assert!(elements[0].is(&first));
            assert!(set.clone().to_pyobject(vm).is(set.as_set()));
            let mut visits = 0;
            set.traverse(&mut |obj| {
                assert!(obj.is(set.as_set()));
                visits += 1;
            });
            assert_eq!(visits, 1);
            let error = PySetOf::try_new([vm.ctx.new_list(vec![])], vm).unwrap_err();
            assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
        });
    }
}
