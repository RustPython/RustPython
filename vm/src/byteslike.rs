use crate::builtins::memory::{try_buffer_from_object, BufferRef};
use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
use crate::pyobject::{BorrowValue, PyObjectRef, PyResult, TryFromObject};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyBytesLike(BufferRef);

#[derive(Debug)]
pub struct PyRwBytesLike(BufferRef);

impl PyBytesLike {
    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&*self.borrow_value())
    }

    pub fn len(&self) -> usize {
        self.borrow_value().len()
    }

    pub fn is_empty(&self) -> bool {
        self.borrow_value().is_empty()
    }

    pub fn to_cow(&self) -> std::borrow::Cow<[u8]> {
        self.borrow_value().to_vec().into()
    }
}

impl PyRwBytesLike {
    pub fn with_ref<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut *self.borrow_value())
    }

    pub fn len(&self) -> usize {
        self.borrow_value().len()
    }

    pub fn is_empty(&self) -> bool {
        self.borrow_value().is_empty()
    }
}

impl PyBytesLike {
    pub fn into_buffer(self) -> BufferRef {
        self.0
    }
}

impl TryFromObject for PyBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let buffer = try_buffer_from_object(vm, &obj)?;
        if buffer.get_options().contiguous {
            Ok(Self(buffer))
        } else {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        }
    }
}

impl<'a> BorrowValue<'a> for PyBytesLike {
    type Borrowed = BorrowedValue<'a, [u8]>;
    fn borrow_value(&'a self) -> Self::Borrowed {
        self.0.as_contiguous().unwrap()
    }
}

pub fn try_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&[u8]) -> R,
) -> PyResult<R> {
    let buffer = try_buffer_from_object(vm, obj)?;
    buffer.as_contiguous().map(|x| f(&*x)).ok_or_else(|| {
        vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned())
    })
}

pub fn try_rw_bytes_like<R>(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    f: impl FnOnce(&mut [u8]) -> R,
) -> PyResult<R> {
    let buffer = try_buffer_from_object(vm, obj)?;
    buffer
        .as_contiguous_mut()
        .map(|mut x| f(&mut *x))
        .ok_or_else(|| vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
}

impl PyRwBytesLike {
    pub fn new(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<Self> {
        let buffer = try_buffer_from_object(vm, &obj)?;
        let options = buffer.get_options();
        if !options.contiguous {
            Err(vm.new_type_error("non-contiguous buffer is not a bytes-like object".to_owned()))
        } else if options.readonly {
            Err(vm.new_type_error("buffer is not a read-write bytes-like object".to_owned()))
        } else {
            Ok(Self(buffer))
        }
    }

    pub fn into_buffer(self) -> BufferRef {
        self.0
    }
}

impl TryFromObject for PyRwBytesLike {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Self::new(vm, &obj)
    }
}

impl<'a> BorrowValue<'a> for PyRwBytesLike {
    type Borrowed = BorrowedValueMut<'a, [u8]>;
    fn borrow_value(&'a self) -> Self::Borrowed {
        self.0.as_contiguous_mut().unwrap()
    }
}
