use core::{fmt, marker::PhantomData};

use crate::{
    PyObjectRef, PyResult, VirtualMachine,
    builtins::PyDictRef,
    object::{Traverse, TraverseFn},
};

use super::ToPyObject;

/// A Python dict constructed from Rust key/value pairs.
///
/// Keys use Python hashing and equality; later equal keys replace earlier values.
/// Construction can execute Python code and propagates its exceptions. Cloning
/// this handle shares the same dictionary, including subsequent mutations.
///
/// `K` and `V` describe the input conversion, not runtime invariants: Python can
/// insert other types. Access through [`Self::as_dict`] is therefore untyped.
pub struct PyDictOf<K, V> {
    dict: PyDictRef,
    types: PhantomData<fn() -> (K, V)>,
}

impl<K: ToPyObject, V: ToPyObject> PyDictOf<K, V> {
    pub fn try_new(items: impl IntoIterator<Item = (K, V)>, vm: &VirtualMachine) -> PyResult<Self> {
        let dict = vm.ctx.new_dict();
        for (key, value) in items {
            let key = key.to_pyobject(vm);
            let value = value.to_pyobject(vm);
            dict.set_item(&*key, value, vm)?;
        }
        Ok(Self {
            dict,
            types: PhantomData,
        })
    }
}

impl<K, V> PyDictOf<K, V> {
    #[must_use]
    pub fn as_dict(&self) -> &PyDictRef {
        &self.dict
    }

    #[must_use]
    pub fn into_dict(self) -> PyDictRef {
        self.dict
    }
}

impl<K, V> Clone for PyDictOf<K, V> {
    fn clone(&self) -> Self {
        Self {
            dict: self.dict.clone(),
            types: PhantomData,
        }
    }
}

impl<K, V> fmt::Debug for PyDictOf<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.dict.fmt(f)
    }
}

impl<K, V> ToPyObject for PyDictOf<K, V> {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.dict.into()
    }
}

impl<K, V> From<PyDictOf<K, V>> for PyObjectRef {
    fn from(value: PyDictOf<K, V>) -> Self {
        value.dict.into()
    }
}

// SAFETY: Only `dict` owns a Python reference; visit it once without cloning.
unsafe impl<K, V> Traverse for PyDictOf<K, V> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.dict.traverse(tracer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsObject, Interpreter};

    #[test]
    fn pairs_use_python_key_semantics_and_propagate_errors() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let first: PyObjectRef = vm.ctx.new_bool(true).into();
            let dict = PyDictOf::try_new([(first.clone(), 10), (vm.ctx.new_int(1).into(), 20)], vm)
                .unwrap();
            let keys = dict.as_dict().keys_vec();
            assert_eq!(keys.len(), 1);
            assert!(keys[0].is(&first));
            assert_eq!(
                dict.as_dict()
                    .get_item(&*first, vm)
                    .unwrap()
                    .try_into_value::<i32>(vm)
                    .unwrap(),
                20
            );
            assert!(dict.clone().to_pyobject(vm).is(dict.as_dict()));
            let mut visits = 0;
            dict.traverse(&mut |obj| {
                assert!(obj.is(dict.as_dict()));
                visits += 1;
            });
            assert_eq!(visits, 1);
            let error = PyDictOf::try_new([(vm.ctx.new_list(vec![]), 1)], vm).unwrap_err();
            assert!(error.fast_isinstance(vm.ctx.exceptions.type_error));
        });
    }
}
