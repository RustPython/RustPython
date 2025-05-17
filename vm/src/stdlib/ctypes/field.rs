use crate::builtins::PyType;
use crate::builtins::PyTypeRef;
use crate::stdlib::ctypes::PyCData;
use crate::types::Constructor;
use crate::types::Representable;
use crate::{Py, PyResult, VirtualMachine};

#[pyclass(name = "PyCFieldType", base = "PyType", module = "_ctypes")]
#[derive(PyPayload, Debug)]
pub struct PyCFieldType {
    pub(super) inner: PyCField,
}

#[pyclass]
impl PyCFieldType {}

#[pyclass(
    name = "CField",
    base = "PyCData",
    metaclass = "PyCFieldType",
    module = "_ctypes"
)]
#[derive(Debug, PyPayload)]
pub struct PyCField {
    byte_offset: usize,
    byte_size: usize,
    #[allow(unused)]
    index: usize,
    proto: PyTypeRef,
    anonymous: bool,
    bitfield_size: bool,
    bit_offset: u8,
    name: String,
}

impl Representable for PyCField {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let tp_name = zelf.proto.name().to_string();
        if zelf.bitfield_size != false {
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

#[derive(Debug, FromArgs)]
pub struct PyCFieldConstructorArgs {
    // PyObject *name, PyObject *proto,
    //               Py_ssize_t byte_size, Py_ssize_t byte_offset,
    //               Py_ssize_t index, int _internal_use,
    //               PyObject *bit_size_obj, PyObject *bit_offset_obj
}

impl Constructor for PyCField {
    type Args = PyCFieldConstructorArgs;

    fn py_new(_cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot instantiate a PyCField".to_string()))
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Constructor, Representable))]
impl PyCField {
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
    fn type_(&self) -> PyTypeRef {
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

#[inline(always)]
pub const fn low_bit(offset: usize) -> usize {
    offset & 0xFFFF
}

#[inline(always)]
pub const fn high_bit(offset: usize) -> usize {
    offset >> 16
}
