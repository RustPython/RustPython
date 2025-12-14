use crate::builtins::PyType;
use crate::function::PySetterValue;
use crate::types::{GetDescriptor, Representable, Unconstructible};
use crate::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};
use num_traits::ToPrimitive;

use super::structure::PyCStructure;
use super::union::PyCUnion;

#[pyclass(name = "PyCFieldType", base = PyType, module = "_ctypes")]
#[derive(Debug)]
pub struct PyCFieldType {
    #[allow(dead_code)]
    pub(super) inner: PyCField,
}

#[pyclass]
impl PyCFieldType {}

#[pyclass(name = "CField", module = "_ctypes")]
#[derive(Debug, PyPayload)]
pub struct PyCField {
    pub(super) byte_offset: usize,
    pub(super) byte_size: usize,
    #[allow(unused)]
    pub(super) index: usize,
    /// The ctypes type for this field (can be any ctypes type including arrays)
    pub(super) proto: PyObjectRef,
    pub(super) anonymous: bool,
    pub(super) bitfield_size: bool,
    pub(super) bit_offset: u8,
    pub(super) name: String,
}

impl PyCField {
    pub fn new(
        name: String,
        proto: PyObjectRef,
        byte_offset: usize,
        byte_size: usize,
        index: usize,
    ) -> Self {
        Self {
            name,
            proto,
            byte_offset,
            byte_size,
            index,
            anonymous: false,
            bitfield_size: false,
            bit_offset: 0,
        }
    }
}

impl Representable for PyCField {
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        // Get type name from the proto object
        let tp_name = if let Some(name_attr) = vm
            .ctx
            .interned_str("__name__")
            .and_then(|s| zelf.proto.get_attr(s, vm).ok())
        {
            name_attr.str(vm)?.to_string()
        } else {
            zelf.proto.class().name().to_string()
        };

        if zelf.bitfield_size {
            Ok(format!(
                "<{} type={}, ofs={byte_offset}, bit_size={bitfield_size}, bit_offset={bit_offset}",
                zelf.name,
                tp_name,
                byte_offset = zelf.byte_offset,
                bitfield_size = zelf.bitfield_size,
                bit_offset = zelf.bit_offset
            ))
        } else {
            Ok(format!(
                "<{} type={tp_name}, ofs={}, size={}",
                zelf.name, zelf.byte_offset, zelf.byte_size
            ))
        }
    }
}

impl Unconstructible for PyCField {}

impl GetDescriptor for PyCField {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let zelf = zelf
            .downcast::<PyCField>()
            .map_err(|_| vm.new_type_error("expected CField".to_owned()))?;

        // If obj is None, return the descriptor itself (class attribute access)
        let obj = match obj {
            Some(obj) if !vm.is_none(&obj) => obj,
            _ => return Ok(zelf.into()),
        };

        // Instance attribute access - read value from the structure/union's buffer
        if let Some(structure) = obj.downcast_ref::<PyCStructure>() {
            let cdata = structure.cdata.read();
            let offset = zelf.byte_offset;
            let size = zelf.byte_size;

            if offset + size <= cdata.buffer.len() {
                let bytes = &cdata.buffer[offset..offset + size];
                return PyCField::bytes_to_value(bytes, size, vm);
            }
        } else if let Some(union) = obj.downcast_ref::<PyCUnion>() {
            let cdata = union.cdata.read();
            let offset = zelf.byte_offset;
            let size = zelf.byte_size;

            if offset + size <= cdata.buffer.len() {
                let bytes = &cdata.buffer[offset..offset + size];
                return PyCField::bytes_to_value(bytes, size, vm);
            }
        }

        // Fallback: return 0 for uninitialized or unsupported types
        Ok(vm.ctx.new_int(0).into())
    }
}

impl PyCField {
    /// Convert bytes to a Python value based on size
    fn bytes_to_value(bytes: &[u8], size: usize, vm: &VirtualMachine) -> PyResult {
        match size {
            1 => Ok(vm.ctx.new_int(bytes[0] as i8).into()),
            2 => {
                let val = i16::from_ne_bytes([bytes[0], bytes[1]]);
                Ok(vm.ctx.new_int(val).into())
            }
            4 => {
                let val = i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                Ok(vm.ctx.new_int(val).into())
            }
            8 => {
                let val = i64::from_ne_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                Ok(vm.ctx.new_int(val).into())
            }
            _ => Ok(vm.ctx.new_int(0).into()),
        }
    }

    /// Convert a Python value to bytes
    fn value_to_bytes(value: &PyObjectRef, size: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        if let Ok(int_val) = value.try_int(vm) {
            let i = int_val.as_bigint();
            match size {
                1 => {
                    let val = i.to_i8().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                2 => {
                    let val = i.to_i16().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                4 => {
                    let val = i.to_i32().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                8 => {
                    let val = i.to_i64().unwrap_or(0);
                    Ok(val.to_ne_bytes().to_vec())
                }
                _ => Ok(vec![0u8; size]),
            }
        } else {
            Ok(vec![0u8; size])
        }
    }
}

#[pyclass(
    flags(DISALLOW_INSTANTIATION, IMMUTABLETYPE),
    with(Unconstructible, Representable, GetDescriptor)
)]
impl PyCField {
    #[pyslot]
    fn descr_set(
        zelf: &crate::PyObject,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = zelf
            .downcast_ref::<PyCField>()
            .ok_or_else(|| vm.new_type_error("expected CField".to_owned()))?;

        // Get the structure/union instance - use downcast_ref() to access the struct data
        if let Some(structure) = obj.downcast_ref::<PyCStructure>() {
            match value {
                PySetterValue::Assign(value) => {
                    let offset = zelf.byte_offset;
                    let size = zelf.byte_size;
                    let bytes = PyCField::value_to_bytes(&value, size, vm)?;

                    let mut cdata = structure.cdata.write();
                    if offset + size <= cdata.buffer.len() {
                        cdata.buffer[offset..offset + size].copy_from_slice(&bytes);
                    }
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_type_error("cannot delete structure field".to_owned()))
                }
            }
        } else if let Some(union) = obj.downcast_ref::<PyCUnion>() {
            match value {
                PySetterValue::Assign(value) => {
                    let offset = zelf.byte_offset;
                    let size = zelf.byte_size;
                    let bytes = PyCField::value_to_bytes(&value, size, vm)?;

                    let mut cdata = union.cdata.write();
                    if offset + size <= cdata.buffer.len() {
                        cdata.buffer[offset..offset + size].copy_from_slice(&bytes);
                    }
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_type_error("cannot delete union field".to_owned()))
                }
            }
        } else {
            Err(vm.new_type_error(format!(
                "descriptor works only on Structure or Union instances, got {}",
                obj.class().name()
            )))
        }
    }

    #[pymethod]
    fn __set__(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Self::descr_set(&zelf, obj, PySetterValue::Assign(value), vm)
    }

    #[pymethod]
    fn __delete__(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        Self::descr_set(&zelf, obj, PySetterValue::Delete, vm)
    }

    #[pygetset]
    fn size(&self) -> usize {
        self.byte_size
    }

    #[pygetset]
    fn bit_size(&self) -> bool {
        self.bitfield_size
    }

    #[pygetset]
    fn is_bitfield(&self) -> bool {
        self.bitfield_size
    }

    #[pygetset]
    fn is_anonymous(&self) -> bool {
        self.anonymous
    }

    #[pygetset]
    fn name(&self) -> String {
        self.name.clone()
    }

    #[pygetset(name = "type")]
    fn type_(&self) -> PyObjectRef {
        self.proto.clone()
    }

    #[pygetset]
    fn offset(&self) -> usize {
        self.byte_offset
    }

    #[pygetset]
    fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    #[pygetset]
    fn byte_size(&self) -> usize {
        self.byte_size
    }

    #[pygetset]
    fn bit_offset(&self) -> u8 {
        self.bit_offset
    }
}
