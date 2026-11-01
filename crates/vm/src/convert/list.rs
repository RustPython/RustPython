use core::{fmt, marker::PhantomData};

use crate::{
    PyObjectRef, VirtualMachine,
    builtins::PyListRef,
    object::{Traverse, TraverseFn},
};

use super::ToPyObject;

/// A Python list constructed from Rust values of type `T`.
///
/// Unlike `Vec<u8>`, which converts to Python `bytes`, `PyListOf<u8>` produces
/// a list of integers. Construction converts each element once; cloning this
/// handle shares the same Python list, including subsequent mutations.
///
/// `T` describes the input conversion, not a runtime invariant: Python can add
/// objects of any type. Access through [`Self::as_list`] is therefore untyped.
pub struct PyListOf<T> {
    list: PyListRef,
    element: PhantomData<fn() -> T>,
}

impl<T: ToPyObject> PyListOf<T> {
    pub fn new(items: impl IntoIterator<Item = T>, vm: &VirtualMachine) -> Self {
        Self {
            list: vm
                .ctx
                .new_list(items.into_iter().map(|item| item.to_pyobject(vm)).collect()),
            element: PhantomData,
        }
    }
}

impl<T> PyListOf<T> {
    #[must_use]
    pub fn as_list(&self) -> &PyListRef {
        &self.list
    }

    #[must_use]
    pub fn into_list(self) -> PyListRef {
        self.list
    }
}

impl<T> Clone for PyListOf<T> {
    fn clone(&self) -> Self {
        Self {
            list: self.list.clone(),
            element: PhantomData,
        }
    }
}

impl<T> fmt::Debug for PyListOf<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.list.fmt(f)
    }
}

impl<T> ToPyObject for PyListOf<T> {
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.list.into()
    }
}

impl<T> From<PyListOf<T>> for PyObjectRef {
    fn from(value: PyListOf<T>) -> Self {
        value.list.into()
    }
}

// SAFETY: Only `list` owns a Python reference; visit it once without cloning.
unsafe impl<T> Traverse for PyListOf<T> {
    fn traverse(&self, tracer: &mut TraverseFn<'_>) {
        self.list.traverse(tracer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsObject, Interpreter, builtins::PyBytes};

    #[test]
    fn explicit_list_conversion() {
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let list = PyListOf::new([0_u8, 255].into_iter().filter(|_| true), vm);
            assert_eq!(
                list.as_list()
                    .as_object()
                    .try_to_value::<Vec<u8>>(vm)
                    .unwrap(),
                [0, 255]
            );
            assert!(
                vec![0_u8, 255]
                    .to_pyobject(vm)
                    .downcast_ref::<PyBytes>()
                    .is_some()
            );
            assert!(
                PyListOf::<u8>::new([], vm)
                    .as_list()
                    .borrow_vec()
                    .is_empty()
            );
        });
    }

    #[test]
    fn shared_identity_and_traversal_do_not_require_element_traits() {
        struct Item;
        impl ToPyObject for Item {
            fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
                vm.ctx.new_int(1).into()
            }
        }
        Interpreter::without_stdlib(Default::default()).enter(|vm| {
            let list = PyListOf::new([Item], vm);
            let copy = list.clone();
            list.as_list().borrow_vec_mut().push(vm.ctx.none());
            assert_eq!(copy.as_list().borrow_vec().len(), 2);
            let mut visits = 0;
            copy.traverse(&mut |obj| {
                assert!(obj.is(list.as_list()));
                visits += 1;
            });
            assert_eq!(visits, 1);
            assert!(copy.to_pyobject(vm).is(&list.into_list()));
        });
    }
}
