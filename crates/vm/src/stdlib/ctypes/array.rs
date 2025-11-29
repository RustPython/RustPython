use crate::atomic_func;
use crate::builtins::{PyBytes, PyInt};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::protocol::{PyNumberMethods, PySequenceMethods};
use crate::types::{AsNumber, AsSequence, Callable};
use crate::{AsObject, Py, PyObjectRef, PyPayload};
use crate::{
    PyResult, VirtualMachine,
    builtins::{PyType, PyTypeRef},
    types::Constructor,
};
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use rustpython_vm::stdlib::ctypes::_ctypes::get_size;
use rustpython_vm::stdlib::ctypes::base::PyCData;

#[pyclass(name = "PyCArrayType", base = PyType, module = "_ctypes")]
#[derive(PyPayload)]
pub struct PyCArrayType {
    pub(super) inner: PyCArray,
}

impl std::fmt::Debug for PyCArrayType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCArrayType")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Callable for PyCArrayType {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // Create an instance of the array
        let element_type = zelf.inner.typ.read().clone();
        let length = zelf.inner.length.load();
        let element_size = zelf.inner.element_size.load();
        let total_size = element_size * length;
        let mut buffer = vec![0u8; total_size];

        // Initialize from positional arguments
        for (i, value) in args.args.iter().enumerate() {
            if i >= length {
                break;
            }
            let offset = i * element_size;
            if let Ok(int_val) = value.try_int(vm) {
                let bytes = PyCArray::int_to_bytes(int_val.as_bigint(), element_size);
                if offset + element_size <= buffer.len() {
                    buffer[offset..offset + element_size].copy_from_slice(&bytes);
                }
            }
        }

        Ok(PyCArray {
            typ: PyRwLock::new(element_type),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            buffer: PyRwLock::new(buffer),
        }
        .into_pyobject(vm))
    }
}

impl Constructor for PyCArrayType {
    type Args = PyObjectRef;

    fn py_new(_cls: PyTypeRef, _args: Self::Args, _vm: &VirtualMachine) -> PyResult {
        unreachable!()
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Callable, Constructor, AsNumber))]
impl PyCArrayType {
    #[pygetset(name = "_type_")]
    fn typ(&self) -> PyTypeRef {
        self.inner.typ.read().clone()
    }

    #[pygetset(name = "_length_")]
    fn length(&self) -> usize {
        self.inner.length.load()
    }

    #[pymethod]
    fn __mul__(zelf: &Py<Self>, n: isize, vm: &VirtualMachine) -> PyResult {
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }
        // Create a nested array type: (inner_type * inner_length) * n
        // The new array has n elements, each element is the current array type
        // e.g., (c_int * 5) * 3 = Array of 3 elements, each is (c_int * 5)
        let inner_length = zelf.inner.length.load();
        let inner_element_size = zelf.inner.element_size.load();

        // The element type of the new array is the current array type itself
        let obj_ref: PyObjectRef = zelf.to_owned().into();
        let current_array_type = obj_ref
            .downcast::<PyType>()
            .expect("PyCArrayType should be a PyType");

        // Element size is the total size of the inner array
        let new_element_size = inner_length * inner_element_size;

        Ok(PyCArrayType {
            inner: PyCArray {
                typ: PyRwLock::new(current_array_type),
                length: AtomicCell::new(n as usize),
                element_size: AtomicCell::new(new_element_size),
                buffer: PyRwLock::new(vec![]),
            },
        }
        .to_pyobject(vm))
    }
}

impl AsNumber for PyCArrayType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let zelf = a
                    .downcast_ref::<PyCArrayType>()
                    .ok_or_else(|| vm.new_type_error("expected PyCArrayType".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCArrayType::__mul__(zelf, n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(
    name = "Array",
    base = PyCData,
    metaclass = "PyCArrayType",
    module = "_ctypes"
)]
#[derive(PyPayload)]
pub struct PyCArray {
    pub(super) typ: PyRwLock<PyTypeRef>,
    pub(super) length: AtomicCell<usize>,
    pub(super) element_size: AtomicCell<usize>,
    pub(super) buffer: PyRwLock<Vec<u8>>,
}

impl std::fmt::Debug for PyCArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCArray")
            .field("typ", &self.typ)
            .field("length", &self.length)
            .finish()
    }
}

impl Constructor for PyCArray {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // Get _type_ and _length_ from the class
        let type_attr = cls.as_object().get_attr("_type_", vm).ok();
        let length_attr = cls.as_object().get_attr("_length_", vm).ok();

        let element_type = type_attr.unwrap_or_else(|| vm.ctx.types.object_type.to_owned().into());
        let length = if let Some(len_obj) = length_attr {
            len_obj.try_int(vm)?.as_bigint().to_usize().unwrap_or(0)
        } else {
            0
        };

        // Get element size from _type_
        let element_size = if let Ok(type_code) = element_type.get_attr("_type_", vm) {
            if let Ok(s) = type_code.str(vm) {
                let s = s.to_string();
                if s.len() == 1 {
                    get_size(&s)
                } else {
                    std::mem::size_of::<usize>()
                }
            } else {
                std::mem::size_of::<usize>()
            }
        } else {
            std::mem::size_of::<usize>()
        };

        let total_size = element_size * length;
        let mut buffer = vec![0u8; total_size];

        // Initialize from positional arguments
        for (i, value) in args.args.iter().enumerate() {
            if i >= length {
                break;
            }
            let offset = i * element_size;
            if let Ok(int_val) = value.try_int(vm) {
                let bytes = Self::int_to_bytes(int_val.as_bigint(), element_size);
                if offset + element_size <= buffer.len() {
                    buffer[offset..offset + element_size].copy_from_slice(&bytes);
                }
            }
        }

        let element_type_ref = element_type
            .downcast::<PyType>()
            .unwrap_or_else(|_| vm.ctx.types.object_type.to_owned());

        PyCArray {
            typ: PyRwLock::new(element_type_ref),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            buffer: PyRwLock::new(buffer),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }
}

impl AsSequence for PyCArray {
    fn as_sequence() -> &'static PySequenceMethods {
        use std::sync::LazyLock;
        static AS_SEQUENCE: LazyLock<PySequenceMethods> = LazyLock::new(|| PySequenceMethods {
            length: atomic_func!(|seq, _vm| Ok(PyCArray::sequence_downcast(seq).length.load())),
            item: atomic_func!(|seq, i, vm| {
                PyCArray::getitem_by_index(PyCArray::sequence_downcast(seq), i, vm)
            }),
            ass_item: atomic_func!(|seq, i, value, vm| {
                let zelf = PyCArray::sequence_downcast(seq);
                match value {
                    Some(v) => PyCArray::setitem_by_index(zelf, i, v, vm),
                    None => Err(vm.new_type_error("cannot delete array elements".to_owned())),
                }
            }),
            ..PySequenceMethods::NOT_IMPLEMENTED
        });
        &AS_SEQUENCE
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor, AsSequence))]
impl PyCArray {
    fn int_to_bytes(i: &malachite_bigint::BigInt, size: usize) -> Vec<u8> {
        match size {
            1 => vec![i.to_i8().unwrap_or(0) as u8],
            2 => i.to_i16().unwrap_or(0).to_ne_bytes().to_vec(),
            4 => i.to_i32().unwrap_or(0).to_ne_bytes().to_vec(),
            8 => i.to_i64().unwrap_or(0).to_ne_bytes().to_vec(),
            _ => vec![0u8; size],
        }
    }

    fn bytes_to_int(bytes: &[u8], size: usize, vm: &VirtualMachine) -> PyObjectRef {
        match size {
            1 => vm.ctx.new_int(bytes[0] as i8).into(),
            2 => {
                let val = i16::from_ne_bytes([bytes[0], bytes[1]]);
                vm.ctx.new_int(val).into()
            }
            4 => {
                let val = i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                vm.ctx.new_int(val).into()
            }
            8 => {
                let val = i64::from_ne_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                vm.ctx.new_int(val).into()
            }
            _ => vm.ctx.new_int(0).into(),
        }
    }

    fn getitem_by_index(zelf: &PyCArray, i: isize, vm: &VirtualMachine) -> PyResult {
        let length = zelf.length.load() as isize;
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("array index out of range".to_owned()));
        }
        let index = index as usize;
        let element_size = zelf.element_size.load();
        let offset = index * element_size;
        let buffer = zelf.buffer.read();
        if offset + element_size <= buffer.len() {
            let bytes = &buffer[offset..offset + element_size];
            Ok(Self::bytes_to_int(bytes, element_size, vm))
        } else {
            Ok(vm.ctx.new_int(0).into())
        }
    }

    fn setitem_by_index(
        zelf: &PyCArray,
        i: isize,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let length = zelf.length.load() as isize;
        let index = if i < 0 { length + i } else { i };
        if index < 0 || index >= length {
            return Err(vm.new_index_error("array index out of range".to_owned()));
        }
        let index = index as usize;
        let element_size = zelf.element_size.load();
        let offset = index * element_size;

        let int_val = value.try_int(vm)?;
        let bytes = Self::int_to_bytes(int_val.as_bigint(), element_size);

        let mut buffer = zelf.buffer.write();
        if offset + element_size <= buffer.len() {
            buffer[offset..offset + element_size].copy_from_slice(&bytes);
        }
        Ok(())
    }

    #[pymethod]
    fn __getitem__(&self, index: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if let Some(i) = index.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer".to_owned())
            })?;
            Self::getitem_by_index(self, i, vm)
        } else {
            Err(vm.new_type_error("array indices must be integers".to_owned()))
        }
    }

    #[pymethod]
    fn __setitem__(
        &self,
        index: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if let Some(i) = index.downcast_ref::<PyInt>() {
            let i = i.as_bigint().to_isize().ok_or_else(|| {
                vm.new_index_error("cannot fit index into an index-sized integer".to_owned())
            })?;
            Self::setitem_by_index(self, i, value, vm)
        } else {
            Err(vm.new_type_error("array indices must be integers".to_owned()))
        }
    }

    #[pymethod]
    fn __len__(&self) -> usize {
        self.length.load()
    }

    #[pygetset(name = "_type_")]
    fn typ(&self) -> PyTypeRef {
        self.typ.read().clone()
    }

    #[pygetset(name = "_length_")]
    fn length_getter(&self) -> usize {
        self.length.load()
    }

    #[pygetset]
    fn value(&self, vm: &VirtualMachine) -> PyObjectRef {
        // Return bytes representation of the buffer
        let buffer = self.buffer.read();
        vm.ctx.new_bytes(buffer.clone()).into()
    }

    #[pygetset(setter)]
    fn set_value(&self, value: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            let mut buffer = self.buffer.write();
            let src = bytes.as_bytes();
            let len = std::cmp::min(src.len(), buffer.len());
            buffer[..len].copy_from_slice(&src[..len]);
        }
        Ok(())
    }

    #[pygetset]
    fn raw(&self, vm: &VirtualMachine) -> PyObjectRef {
        let buffer = self.buffer.read();
        vm.ctx.new_bytes(buffer.clone()).into()
    }

    #[pygetset(setter)]
    fn set_raw(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            let mut buffer = self.buffer.write();
            let src = bytes.as_bytes();
            let len = std::cmp::min(src.len(), buffer.len());
            buffer[..len].copy_from_slice(&src[..len]);
            Ok(())
        } else {
            Err(vm.new_type_error("expected bytes".to_owned()))
        }
    }
}

impl PyCArray {
    #[allow(unused)]
    pub fn to_arg(&self, _vm: &VirtualMachine) -> PyResult<libffi::middle::Arg> {
        // TODO: This needs a different approach to ensure buffer lifetime
        // The buffer must outlive the Arg returned here
        let buffer = self.buffer.read();
        let ptr = buffer.as_ptr();
        Ok(libffi::middle::Arg::new(&ptr))
    }
}
