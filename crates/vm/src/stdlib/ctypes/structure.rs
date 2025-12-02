use super::base::{CDataObject, PyCData};
use super::field::PyCField;
use super::util::StgInfo;
use crate::builtins::{PyList, PyStr, PyTuple, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::protocol::{BufferDescriptor, BufferMethods, PyBuffer, PyNumberMethods};
use crate::stdlib::ctypes::_ctypes::get_size;
use crate::types::{AsBuffer, AsNumber, Constructor};
use crate::{AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};
use indexmap::IndexMap;
use num_traits::ToPrimitive;
use rustpython_common::lock::PyRwLock;
use std::fmt::Debug;

/// PyCStructType - metaclass for Structure
#[pyclass(name = "PyCStructType", base = PyType, module = "_ctypes")]
#[derive(Debug, Default)]
pub struct PyCStructType {}

impl Constructor for PyCStructType {
    type Args = FuncArgs;

    fn slot_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // 1. Create the new class using PyType::py_new
        let new_class = crate::builtins::type_::PyType::slot_new(metatype, args, vm)?;

        // 2. Process _fields_ if defined on the new class
        let new_type = new_class
            .clone()
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("expected type"))?;

        // Only process _fields_ if defined directly on this class (not inherited)
        if let Some(fields_attr) = new_type.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            Self::process_fields(&new_type, fields_attr, vm)?;
        }

        Ok(new_class)
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

#[pyclass(flags(BASETYPE), with(AsNumber, Constructor))]
impl PyCStructType {
    /// Called when a new Structure subclass is created
    #[pyclassmethod]
    fn __init_subclass__(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<()> {
        // Check if _fields_ is defined
        if let Some(fields_attr) = cls.get_direct_attr(vm.ctx.intern_str("_fields_")) {
            Self::process_fields(&cls, fields_attr, vm)?;
        }
        Ok(())
    }

    /// Process _fields_ and create CField descriptors
    fn process_fields(
        cls: &PyTypeRef,
        fields_attr: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Try to downcast to list or tuple
        let fields: Vec<PyObjectRef> = if let Some(list) = fields_attr.downcast_ref::<PyList>() {
            list.borrow_vec().to_vec()
        } else if let Some(tuple) = fields_attr.downcast_ref::<PyTuple>() {
            tuple.to_vec()
        } else {
            return Err(vm.new_type_error("_fields_ must be a list or tuple".to_string()));
        };

        let mut offset = 0usize;
        for (index, field) in fields.iter().enumerate() {
            let field_tuple = field
                .downcast_ref::<PyTuple>()
                .ok_or_else(|| vm.new_type_error("_fields_ must contain tuples".to_string()))?;

            if field_tuple.len() < 2 {
                return Err(vm.new_type_error(
                    "_fields_ tuple must have at least 2 elements (name, type)".to_string(),
                ));
            }

            let name = field_tuple
                .first()
                .unwrap()
                .downcast_ref::<PyStr>()
                .ok_or_else(|| vm.new_type_error("field name must be a string".to_string()))?
                .to_string();

            let field_type = field_tuple.get(1).unwrap().clone();

            // Get size of the field type
            let size = Self::get_field_size(&field_type, vm)?;

            // Create CField descriptor (accepts any ctypes type including arrays)
            let c_field = PyCField::new(name.clone(), field_type, offset, size, index);

            // Set the CField as a class attribute
            cls.set_attr(vm.ctx.intern_str(name), c_field.to_pyobject(vm));

            offset += size;
        }

        Ok(())
    }

    /// Get the size of a ctypes type
    fn get_field_size(field_type: &PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        // Try to get _type_ attribute for simple types
        if let Some(size) = field_type
            .get_attr("_type_", vm)
            .ok()
            .and_then(|type_attr| type_attr.str(vm).ok())
            .and_then(|type_str| {
                let s = type_str.to_string();
                (s.len() == 1).then(|| get_size(&s))
            })
        {
            return Ok(size);
        }

        // Try sizeof for other types
        if let Some(s) = field_type
            .get_attr("size_of_instances", vm)
            .ok()
            .and_then(|size_method| size_method.call((), vm).ok())
            .and_then(|size| size.try_int(vm).ok())
            .and_then(|n| n.as_bigint().to_usize())
        {
            return Ok(s);
        }

        // Default to pointer size for unknown types
        Ok(std::mem::size_of::<usize>())
    }

    /// Get the alignment of a ctypes type
    fn get_field_align(field_type: &PyObjectRef, vm: &VirtualMachine) -> usize {
        // Try to get _type_ attribute for simple types
        if let Some(align) = field_type
            .get_attr("_type_", vm)
            .ok()
            .and_then(|type_attr| type_attr.str(vm).ok())
            .and_then(|type_str| {
                let s = type_str.to_string();
                (s.len() == 1).then(|| get_size(&s)) // alignment == size for simple types
            })
        {
            return align;
        }
        // Default alignment
        1
    }

    #[pymethod]
    fn __mul__(cls: PyTypeRef, n: isize, vm: &VirtualMachine) -> PyResult {
        use super::array::create_array_type_with_stg_info;
        use crate::stdlib::ctypes::_ctypes::size_of;

        if n < 0 {
            return Err(vm.new_value_error(format!("Array length must be >= 0, not {n}")));
        }

        // Calculate element size from the Structure type
        let element_size = size_of(cls.clone().into(), vm)?;

        let total_size = element_size
            .checked_mul(n as usize)
            .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
        let stg_info = super::util::StgInfo::new_array(
            total_size,
            element_size,
            n as usize,
            cls.clone().into(),
            element_size,
        );
        create_array_type_with_stg_info(stg_info, vm)
    }
}

impl AsNumber for PyCStructType {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            multiply: Some(|a, b, vm| {
                let cls = a
                    .downcast_ref::<PyType>()
                    .ok_or_else(|| vm.new_type_error("expected type".to_owned()))?;
                let n = b
                    .try_index(vm)?
                    .as_bigint()
                    .to_isize()
                    .ok_or_else(|| vm.new_overflow_error("array size too large".to_owned()))?;
                PyCStructType::__mul__(cls.to_owned(), n, vm)
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

/// Structure field info stored in instance
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub type_ref: PyTypeRef,
}

/// PyCStructure - base class for Structure instances
#[pyclass(
    module = "_ctypes",
    name = "Structure",
    base = PyCData,
    metaclass = "PyCStructType"
)]
pub struct PyCStructure {
    /// Common CDataObject for memory buffer
    pub(super) cdata: PyRwLock<CDataObject>,
    /// Field information (name -> FieldInfo)
    #[allow(dead_code)]
    pub(super) fields: PyRwLock<IndexMap<String, FieldInfo>>,
}

impl Debug for PyCStructure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyCStructure")
            .field("size", &self.cdata.read().size())
            .finish()
    }
}

impl Constructor for PyCStructure {
    type Args = FuncArgs;

    fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // Get _fields_ from the class using get_attr to properly search MRO
        let fields_attr = cls.as_object().get_attr("_fields_", vm).ok();

        let mut fields_map = IndexMap::new();
        let mut total_size = 0usize;
        let mut max_align = 1usize;

        if let Some(fields_attr) = fields_attr {
            let fields: Vec<PyObjectRef> = if let Some(list) = fields_attr.downcast_ref::<PyList>()
            {
                list.borrow_vec().to_vec()
            } else if let Some(tuple) = fields_attr.downcast_ref::<PyTuple>() {
                tuple.to_vec()
            } else {
                vec![]
            };

            let mut offset = 0usize;
            for field in fields.iter() {
                let Some(field_tuple) = field.downcast_ref::<PyTuple>() else {
                    continue;
                };
                if field_tuple.len() < 2 {
                    continue;
                }
                let Some(name) = field_tuple.first().unwrap().downcast_ref::<PyStr>() else {
                    continue;
                };
                let name = name.to_string();
                let field_type = field_tuple.get(1).unwrap().clone();
                let size = PyCStructType::get_field_size(&field_type, vm)?;
                let field_align = PyCStructType::get_field_align(&field_type, vm);
                max_align = max_align.max(field_align);

                let type_ref = field_type
                    .downcast::<PyType>()
                    .unwrap_or_else(|_| vm.ctx.types.object_type.to_owned());

                fields_map.insert(
                    name.clone(),
                    FieldInfo {
                        name,
                        offset,
                        size,
                        type_ref,
                    },
                );

                offset += size;
            }
            total_size = offset;
        }

        // Initialize buffer with zeros
        let mut stg_info = StgInfo::new(total_size, max_align);
        stg_info.length = fields_map.len();
        let instance = PyCStructure {
            cdata: PyRwLock::new(CDataObject::from_stg_info(&stg_info)),
            fields: PyRwLock::new(fields_map.clone()),
        };

        // Handle keyword arguments for field initialization
        let py_instance = instance.into_ref_with_type(vm, cls.clone())?;
        let py_obj: PyObjectRef = py_instance.clone().into();

        // Set field values from kwargs using standard attribute setting
        for (key, value) in args.kwargs.iter() {
            if fields_map.contains_key(key.as_str()) {
                py_obj.set_attr(vm.ctx.intern_str(key.as_str()), value.clone(), vm)?;
            }
        }

        // Set field values from positional args
        let field_names: Vec<String> = fields_map.keys().cloned().collect();
        for (i, value) in args.args.iter().enumerate() {
            if i < field_names.len() {
                py_obj.set_attr(
                    vm.ctx.intern_str(field_names[i].as_str()),
                    value.clone(),
                    vm,
                )?;
            }
        }

        Ok(py_instance.into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

// Note: GetAttr and SetAttr are not implemented here.
// Field access is handled by CField descriptors registered on the class.

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor))]
impl PyCStructure {
    #[pygetset]
    fn _objects(&self) -> Option<PyObjectRef> {
        self.cdata.read().objects.clone()
    }

    #[pygetset]
    fn _fields_(&self, vm: &VirtualMachine) -> PyObjectRef {
        // Return the _fields_ from the class, not instance
        vm.ctx.none()
    }

    #[pyclassmethod]
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        use crate::stdlib::ctypes::_ctypes::size_of;

        // Get size from cls
        let size = size_of(cls.clone().into(), vm)?;

        // Read data from address
        if address == 0 || size == 0 {
            return Err(vm.new_value_error("NULL pointer access".to_owned()));
        }
        let data = unsafe {
            let ptr = address as *const u8;
            std::slice::from_raw_parts(ptr, size).to_vec()
        };

        // Create instance
        Ok(PyCStructure {
            cdata: PyRwLock::new(CDataObject::from_bytes(data, None)),
            fields: PyRwLock::new(IndexMap::new()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
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
        let data = bytes[offset..offset + size].to_vec();

        // Create instance
        Ok(PyCStructure {
            cdata: PyRwLock::new(CDataObject::from_bytes(data, Some(source))),
            fields: PyRwLock::new(IndexMap::new()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
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
        let data = source_bytes[offset..offset + size].to_vec();

        // Create instance
        Ok(PyCStructure {
            cdata: PyRwLock::new(CDataObject::from_bytes(data, None)),
            fields: PyRwLock::new(IndexMap::new()),
        }
        .into_ref_with_type(vm, cls)?
        .into())
    }
}

static STRUCTURE_BUFFER_METHODS: BufferMethods = BufferMethods {
    obj_bytes: |buffer| {
        rustpython_common::lock::PyMappedRwLockReadGuard::map(
            rustpython_common::lock::PyRwLockReadGuard::map(
                buffer.obj_as::<PyCStructure>().cdata.read(),
                |x: &CDataObject| x,
            ),
            |x: &CDataObject| x.buffer.as_slice(),
        )
        .into()
    },
    obj_bytes_mut: |buffer| {
        rustpython_common::lock::PyMappedRwLockWriteGuard::map(
            rustpython_common::lock::PyRwLockWriteGuard::map(
                buffer.obj_as::<PyCStructure>().cdata.write(),
                |x: &mut CDataObject| x,
            ),
            |x: &mut CDataObject| x.buffer.as_mut_slice(),
        )
        .into()
    },
    release: |_| {},
    retain: |_| {},
};

impl AsBuffer for PyCStructure {
    fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
        let buffer_len = zelf.cdata.read().buffer.len();
        let buf = PyBuffer::new(
            zelf.to_owned().into(),
            BufferDescriptor::simple(buffer_len, false), // readonly=false for ctypes
            &STRUCTURE_BUFFER_METHODS,
        );
        Ok(buf)
    }
}
