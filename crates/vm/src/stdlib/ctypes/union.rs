use super::base::PyCData;
use super::field::PyCField;
use crate::builtins::{PyList, PyStr, PyTuple, PyType, PyTypeRef};
use crate::convert::ToPyObject;
use crate::function::FuncArgs;
use crate::stdlib::ctypes::_ctypes::get_size;
use crate::types::Constructor;
use crate::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
use num_traits::ToPrimitive;

/// PyCUnionType - metaclass for Union
#[pyclass(name = "UnionType", base = PyType, module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCUnionType {}

impl Constructor for PyCUnionType {
    type Args = FuncArgs;

    fn py_new(metatype: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // 1. Create the new class using PyType::py_new
        let new_class = crate::builtins::type_::PyType::py_new(metatype, args, vm)?;

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
}

impl PyCUnionType {
    /// Process _fields_ and create CField descriptors
    /// For Union, all fields start at offset 0
    fn process_fields(
        cls: &PyTypeRef,
        fields_attr: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let fields: Vec<PyObjectRef> = if let Some(list) = fields_attr.downcast_ref::<PyList>() {
            list.borrow_vec().to_vec()
        } else if let Some(tuple) = fields_attr.downcast_ref::<PyTuple>() {
            tuple.to_vec()
        } else {
            return Err(vm.new_type_error("_fields_ must be a list or tuple".to_string()));
        };

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
            let size = Self::get_field_size(&field_type, vm)?;

            // For Union, all fields start at offset 0
            // Create CField descriptor (accepts any ctypes type including arrays)
            let cfield = PyCField::new(name.clone(), field_type, 0, size, index);

            cls.set_attr(vm.ctx.intern_str(name), cfield.to_pyobject(vm));
        }

        Ok(())
    }

    fn get_field_size(field_type: &PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
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

        if let Some(s) = field_type
            .get_attr("size_of_instances", vm)
            .ok()
            .and_then(|size_method| size_method.call((), vm).ok())
            .and_then(|size| size.try_int(vm).ok())
            .and_then(|n| n.as_bigint().to_usize())
        {
            return Ok(s);
        }

        Ok(std::mem::size_of::<usize>())
    }
}

#[pyclass(flags(BASETYPE), with(Constructor))]
impl PyCUnionType {}

/// PyCUnion - base class for Union
#[pyclass(module = "_ctypes", name = "Union", base = PyCData, metaclass = "PyCUnionType")]
#[derive(Debug, PyPayload)]
pub struct PyCUnion {}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCUnion {
    #[pyclassmethod]
    fn from_address(cls: PyTypeRef, address: isize, vm: &VirtualMachine) -> PyResult {
        use crate::function::Either;
        use crate::stdlib::ctypes::_ctypes::size_of;

        // Get size from cls
        let size = size_of(Either::A(cls.clone()), vm)?;

        if address != 0 && size > 0 {
            // Create instance (Union doesn't have internal buffer in current impl)
            Ok(PyCUnion {}.into_ref_with_type(vm, cls)?.into())
        } else {
            Err(vm.new_value_error("NULL pointer access".to_owned()))
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
        use crate::function::Either;
        use crate::protocol::PyBuffer;
        use crate::stdlib::ctypes::_ctypes::size_of;

        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        let buffer = PyBuffer::try_from_object(vm, source.clone())?;

        if buffer.desc.readonly {
            return Err(vm.new_type_error("underlying buffer is not writable".to_owned()));
        }

        let size = size_of(Either::A(cls.clone()), vm)?;
        let buffer_len = buffer.desc.len;

        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        Ok(PyCUnion {}.into_ref_with_type(vm, cls)?.into())
    }

    #[pyclassmethod]
    fn from_buffer_copy(
        cls: PyTypeRef,
        source: crate::function::ArgBytesLike,
        offset: crate::function::OptionalArg<isize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        use crate::function::Either;
        use crate::stdlib::ctypes::_ctypes::size_of;

        let offset = offset.unwrap_or(0);
        if offset < 0 {
            return Err(vm.new_value_error("offset cannot be negative".to_owned()));
        }
        let offset = offset as usize;

        let size = size_of(Either::A(cls.clone()), vm)?;
        let source_bytes = source.borrow_buf();
        let buffer_len = source_bytes.len();

        if offset + size > buffer_len {
            return Err(vm.new_value_error(format!(
                "Buffer size too small ({} instead of at least {} bytes)",
                buffer_len,
                offset + size
            )));
        }

        Ok(PyCUnion {}.into_ref_with_type(vm, cls)?.into())
    }
}
