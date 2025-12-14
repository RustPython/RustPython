use crate::atomic_func;
use crate::builtins::{PyBytes, PyInt};
use crate::class::StaticType;
use crate::function::FuncArgs;
use crate::protocol::{
    BufferDescriptor, BufferMethods, PyBuffer, PyNumberMethods, PySequenceMethods,
};
use crate::stdlib::ctypes::base::CDataObject;
use crate::stdlib::ctypes::util::StgInfo;
use crate::types::{AsBuffer, AsNumber, AsSequence};
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

/// PyCArrayType - metatype for Array types
/// CPython stores array info (type, length) in StgInfo via type_data
#[pyclass(name = "PyCArrayType", base = PyType, module = "_ctypes")]
#[derive(Debug, Default)]
pub struct PyCArrayType {}

/// Create a new Array type with StgInfo stored in type_data (CPython style)
pub fn create_array_type_with_stg_info(stg_info: StgInfo, vm: &VirtualMachine) -> PyResult {
    // Get PyCArrayType as metaclass
    let metaclass = PyCArrayType::static_type().to_owned();

    // Create a unique name for the array type
    let type_name = format!("Array_{}", stg_info.length);

    // Create args for type(): (name, bases, dict)
    let name = vm.ctx.new_str(type_name);
    let bases = vm
        .ctx
        .new_tuple(vec![PyCArray::static_type().to_owned().into()]);
    let dict = vm.ctx.new_dict();

    let args = FuncArgs::new(
        vec![name.into(), bases.into(), dict.into()],
        crate::function::KwArgs::default(),
    );

    // Create the new type using PyType::slot_new with PyCArrayType as metaclass
    let new_type = crate::builtins::type_::PyType::slot_new(metaclass, args, vm)?;

    // Set StgInfo in type_data
    let type_ref: PyTypeRef = new_type
        .clone()
        .downcast()
        .map_err(|_| vm.new_type_error("Failed to create array type".to_owned()))?;

    if type_ref.init_type_data(stg_info.clone()).is_err() {
        // Type data already initialized - update it
        if let Some(mut existing) = type_ref.get_type_data_mut::<StgInfo>() {
            *existing = stg_info;
        }
    }

    Ok(new_type)
}

impl Constructor for PyCArrayType {
    type Args = PyObjectRef;

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

#[pyclass(flags(IMMUTABLETYPE), with(Constructor, AsNumber))]
impl PyCArrayType {
    #[pygetset(name = "_type_")]
    fn typ(zelf: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        zelf.downcast_ref::<PyType>()
            .and_then(|t| t.get_type_data::<StgInfo>())
            .and_then(|stg| stg.element_type.clone())
            .unwrap_or_else(|| vm.ctx.none())
    }

    #[pygetset(name = "_length_")]
    fn length(zelf: PyObjectRef) -> usize {
        zelf.downcast_ref::<PyType>()
            .and_then(|t| t.get_type_data::<StgInfo>())
            .map(|stg| stg.length)
            .unwrap_or(0)
    }

    #[pymethod]
    fn __mul__(zelf: PyObjectRef, n: isize, vm: &VirtualMachine) -> PyResult {
        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }

        // Get inner array info from TypeDataSlot
        let type_ref = zelf.downcast_ref::<PyType>().unwrap();
        let (_inner_length, inner_size) = type_ref
            .get_type_data::<StgInfo>()
            .map(|stg| (stg.length, stg.size))
            .unwrap_or((0, 0));

        // The element type of the new array is the current array type itself
        let current_array_type: PyObjectRef = zelf.clone();

        // Element size is the total size of the inner array
        let new_element_size = inner_size;
        let total_size = new_element_size * (n as usize);

        let stg_info = StgInfo::new_array(
            total_size,
            new_element_size,
            n as usize,
            current_array_type,
            new_element_size,
        );

        create_array_type_with_stg_info(stg_info, vm)
    }

    #[pyclassmethod]
    fn in_dll(
        zelf: PyObjectRef,
        dll: PyObjectRef,
        name: crate::builtins::PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        use libloading::Symbol;

        // Get the library handle from dll object
        let handle = if let Ok(int_handle) = dll.try_int(vm) {
            // dll is an integer handle
            int_handle
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        } else {
            // dll is a CDLL/PyDLL/WinDLL object with _handle attribute
            dll.get_attr("_handle", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        };

        // Get the library from cache
        let library_cache = crate::stdlib::ctypes::library::libcache().read();
        let library = library_cache
            .get_lib(handle)
            .ok_or_else(|| vm.new_attribute_error("Library not found".to_owned()))?;

        // Get symbol address from library
        let symbol_name = format!("{}\0", name.as_str());
        let inner_lib = library.lib.lock();

        let symbol_address = if let Some(lib) = &*inner_lib {
            unsafe {
                // Try to get the symbol from the library
                let symbol: Symbol<'_, *mut u8> = lib.get(symbol_name.as_bytes()).map_err(|e| {
                    vm.new_attribute_error(format!("{}: symbol '{}' not found", e, name.as_str()))
                })?;
                *symbol as usize
            }
        } else {
            return Err(vm.new_attribute_error("Library is closed".to_owned()));
        };

        // Get size from the array type via TypeDataSlot
        let type_ref = zelf.downcast_ref::<PyType>().unwrap();
        let (element_type, length, element_size) = type_ref
            .get_type_data::<StgInfo>()
            .map(|stg| {
                (
                    stg.element_type.clone().unwrap_or_else(|| vm.ctx.none()),
                    stg.length,
                    stg.element_size,
                )
            })
            .unwrap_or_else(|| (vm.ctx.none(), 0, 0));
        let total_size = element_size * length;

        // Read data from symbol address
        let data = if symbol_address != 0 && total_size > 0 {
            unsafe {
                let ptr = symbol_address as *const u8;
                std::slice::from_raw_parts(ptr, total_size).to_vec()
            }
        } else {
            vec![0; total_size]
        };

        // Create instance
        let instance = PyCArray {
            typ: PyRwLock::new(element_type),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            cdata: PyRwLock::new(CDataObject::from_bytes(data, None)),
        }
        .into_pyobject(vm);

        // Store base reference to keep dll alive
        if let Ok(array_ref) = instance.clone().downcast::<PyCArray>() {
            array_ref.cdata.write().base = Some(dll);
        }

        Ok(instance)
    }
}

impl AsNumber for PyCArrayType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                // a is a type object whose metaclass is PyCArrayType (e.g., Array_5)
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCArrayType::__mul__(a.to_owned(), n, vm)
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
pub struct PyCArray {
    /// Element type - can be a simple type (c_int) or an array type (c_int * 5)
    pub(super) typ: PyRwLock<PyObjectRef>,
    pub(super) length: AtomicCell<usize>,
    pub(super) element_size: AtomicCell<usize>,
    pub(super) cdata: PyRwLock<CDataObject>,
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

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
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
                let bytes = PyCArray::int_to_bytes(int_val.as_bigint(), element_size);
                if offset + element_size <= buffer.len() {
                    buffer[offset..offset + element_size].copy_from_slice(&bytes);
                }
            }
        }

        PyCArray {
            typ: PyRwLock::new(element_type),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            cdata: PyRwLock::new(CDataObject::from_bytes(buffer, None)),
        }
        .into_ref_with_type(vm, cls)
        .map(Into::into)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
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

#[pyclass(
    flags(BASETYPE, IMMUTABLETYPE),
    with(Constructor, AsSequence, AsBuffer)
)]
impl PyCArray {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.cdata.read().objects.clone()
    }

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
        let buffer = zelf.cdata.read().buffer.clone();
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

        let mut cdata = zelf.cdata.write();
        if offset + element_size <= cdata.buffer.len() {
            cdata.buffer[offset..offset + element_size].copy_from_slice(&bytes);
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
    fn typ(&self) -> PyObjectRef {
        self.typ.read().clone()
    }

    #[pygetset(name = "_length_")]
    fn length_getter(&self) -> usize {
        self.length.load()
    }

    #[pygetset]
    fn value(&self, vm: &VirtualMachine) -> PyObjectRef {
        // Return bytes representation of the buffer
        let buffer = self.cdata.read().buffer.clone();
        vm.ctx.new_bytes(buffer.clone()).into()
    }

    #[pygetset(setter)]
    fn set_value(&self, value: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            let mut cdata = self.cdata.write();
            let src = bytes.as_bytes();
            let len = std::cmp::min(src.len(), cdata.buffer.len());
            cdata.buffer[..len].copy_from_slice(&src[..len]);
        }
        Ok(())
    }

    #[pygetset]
    fn raw(&self, vm: &VirtualMachine) -> PyObjectRef {
        let cdata = self.cdata.read();
        vm.ctx.new_bytes(cdata.buffer.clone()).into()
    }

    #[pygetset(setter)]
    fn set_raw(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(bytes) = value.downcast_ref::<PyBytes>() {
            let mut cdata = self.cdata.write();
            let src = bytes.as_bytes();
            let len = std::cmp::min(src.len(), cdata.buffer.len());
            cdata.buffer[..len].copy_from_slice(&src[..len]);
            Ok(())
        } else {
            Err(vm.new_type_error("expected bytes".to_owned()))
        }
    }

    #[pyclassmethod]
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        use crate::stdlib::ctypes::_ctypes::size_of;

        // Get size from cls
        let size = size_of(cls.clone().into(), vm)?;

        // Create instance with data from address
        if address == 0 || size == 0 {
            return Err(vm.new_value_error("NULL pointer access".to_owned()));
        }
        unsafe {
            let ptr = address as *const u8;
            let bytes = std::slice::from_raw_parts(ptr, size);
            // Get element type and length from cls
            let element_type = cls.as_object().get_attr("_type_", vm)?;
            let element_type: PyTypeRef = element_type
                .downcast()
                .map_err(|_| vm.new_type_error("_type_ must be a type".to_owned()))?;
            let length = cls
                .as_object()
                .get_attr("_length_", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .unwrap_or(0);
            let element_size = if length > 0 { size / length } else { 0 };

            Ok(PyCArray {
                typ: PyRwLock::new(element_type.into()),
                length: AtomicCell::new(length),
                element_size: AtomicCell::new(element_size),
                cdata: PyRwLock::new(CDataObject::from_bytes(bytes.to_vec(), None)),
            }
            .into_pyobject(vm))
        }
    }

    #[pyclassmethod]
    fn from_buffer(
        cls: PyTypeRef,
        source: PyObjectRef,
        offset: crate::function::OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::TryFromObject;
        use crate::protocol::PyBuffer;
        use crate::stdlib::ctypes::_ctypes::size_of;

        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        // Get buffer from source
        let buffer = PyBuffer::try_from_object(vm, source.clone())?;

        // Check if buffer is writable
        if buffer.desc.readonly {
            return Err(vm.new_type_error("underlying buffer is not writable".to_owned()));
        }

        // Get size from cls
        let size = size_of(cls.clone().into(), vm)?;

        // Check if buffer is large enough
        let buffer_len = buffer.desc.len;
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Read bytes from buffer at offset
        let bytes = buffer.obj_bytes();
        let data = &bytes[offset..offset + size];

        // Get element type and length from cls
        let element_type = cls.as_object().get_attr("_type_", vm)?;
        let element_type: PyTypeRef = element_type
            .downcast()
            .map_err(|_| vm.new_type_error("_type_ must be a type".to_owned()))?;
        let length = cls
            .as_object()
            .get_attr("_length_", vm)?
            .try_int(vm)?
            .as_bigint()
            .to_usize()
            .unwrap_or(0);
        let element_size = if length > 0 { size / length } else { 0 };

        Ok(PyCArray {
            typ: PyRwLock::new(element_type.into()),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            cdata: PyRwLock::new(CDataObject::from_bytes(
                data.to_vec(),
                Some(buffer.obj.clone()),
            )),
        }
        .into_pyobject(vm))
    }

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        source: crate::function::ArgBytesLike,
        offset: crate::function::OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::stdlib::ctypes::_ctypes::size_of;

        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        // Get size from cls
        let size = size_of(cls.clone().into(), vm)?;

        // Borrow bytes from source
        let source_bytes = source.borrow_buf();
        let buffer_len = source_bytes.len();

        // Check if buffer is large enough
        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        // Copy bytes from buffer at offset
        let data = &source_bytes[offset..offset + size];

        // Get element type and length from cls
        let element_type = cls.as_object().get_attr("_type_", vm)?;
        let element_type: PyTypeRef = element_type
            .downcast()
            .map_err(|_| vm.new_type_error("_type_ must be a type".to_owned()))?;
        let length = cls
            .as_object()
            .get_attr("_length_", vm)?
            .try_int(vm)?
            .as_bigint()
            .to_usize()
            .unwrap_or(0);
        let element_size = if length > 0 { size / length } else { 0 };

        Ok(PyCArray {
            typ: PyRwLock::new(element_type.into()),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            cdata: PyRwLock::new(CDataObject::from_bytes(data.to_vec(), None)),
        }
        .into_pyobject(vm))
    }

    #[pyclassmethod]
    fn in_dll(
        cls: PyTypeRef,
        dll: PyObjectRef,
        name: crate::builtins::PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::stdlib::ctypes::_ctypes::size_of;
        use libloading::Symbol;

        // Get the library handle from dll object
        let handle = if let Ok(int_handle) = dll.try_int(vm) {
            // dll is an integer handle
            int_handle
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        } else {
            // dll is a CDLL/PyDLL/WinDLL object with _handle attribute
            dll.get_attr("_handle", vm)?
                .try_int(vm)?
                .as_bigint()
                .to_usize()
                .ok_or_else(|| vm.new_value_error("Invalid library handle".to_owned()))?
        };

        // Get the library from cache
        let library_cache = crate::stdlib::ctypes::library::libcache().read();
        let library = library_cache
            .get_lib(handle)
            .ok_or_else(|| vm.new_attribute_error("Library not found".to_owned()))?;

        // Get symbol address from library
        let symbol_name = format!("{}\0", name.as_str());
        let inner_lib = library.lib.lock();

        let symbol_address = if let Some(lib) = &*inner_lib {
            unsafe {
                // Try to get the symbol from the library
                let symbol: Symbol<'_, *mut u8> = lib.get(symbol_name.as_bytes()).map_err(|e| {
                    vm.new_attribute_error(format!("{}: symbol '{}' not found", e, name.as_str()))
                })?;
                *symbol as usize
            }
        } else {
            return Err(vm.new_attribute_error("Library is closed".to_owned()));
        };

        // Get size from cls
        let size = size_of(cls.clone().into(), vm)?;

        // Read data from symbol address
        let data = if symbol_address != 0 && size > 0 {
            unsafe {
                let ptr = symbol_address as *const u8;
                std::slice::from_raw_parts(ptr, size).to_vec()
            }
        } else {
            vec![0; size]
        };

        // Get element type and length from cls
        let element_type = cls.as_object().get_attr("_type_", vm)?;
        let element_type: PyTypeRef = element_type
            .downcast()
            .map_err(|_| vm.new_type_error("_type_ must be a type".to_owned()))?;
        let length = cls
            .as_object()
            .get_attr("_length_", vm)?
            .try_int(vm)?
            .as_bigint()
            .to_usize()
            .unwrap_or(0);
        let element_size = if length > 0 { size / length } else { 0 };

        // Create instance
        let instance = PyCArray {
            typ: PyRwLock::new(element_type.into()),
            length: AtomicCell::new(length),
            element_size: AtomicCell::new(element_size),
            cdata: PyRwLock::new(CDataObject::from_bytes(data, None)),
        }
        .into_pyobject(vm);

        // Store base reference to keep dll alive
        if let Ok(array_ref) = instance.clone().downcast::<PyCArray>() {
            array_ref.cdata.write().base = Some(dll);
        }

        Ok(instance)
    }
}

impl PyCArray {
    #[allow(unused)]
    pub fn to_arg(&self, _vm: &VirtualMachine) -> PyResult<libffi::middle::Arg> {
        let cdata = self.cdata.read();
        Ok(libffi::middle::Arg::new(&cdata.buffer))
    }
}

static ARRAY_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        rustpython_common::lock::PyMappedRwLockReadGuard::map(
            rustpython_common::lock::PyRwLockReadGuard::map(
                buffer.obj_as::<PyCArray>().cdata.read(),
                |x: &CDataObject| x,
            ),
            |x: &CDataObject| x.buffer.as_slice(),
        )
        .into()
    },
    obj_bytes_mut: |buffer| {
        rustpython_common::lock::PyMappedRwLockWriteGuard::map(
            rustpython_common::lock::PyRwLockWriteGuard::map(
                buffer.obj_as::<PyCArray>().cdata.write(),
                |x: &mut CDataObject| x,
            ),
            |x: &mut CDataObject| x.buffer.as_mut_slice(),
        )
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

impl AsBuffer for PyCArray {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.cdata.read().buffer.len();
        let buf = PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(buffer_len, false), // readonly=false for ctypes
            &ARRAY_BUFFER_METHODS,
        );
        Ok(buf)
    }
}
