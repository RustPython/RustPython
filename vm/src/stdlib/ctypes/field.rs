use crate::builtins::PyType;

#[pyclass(name = "PyCFieldType", base = "PyType", module = "_ctypes")]
#[derive(PyPayload)]
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
pub struct PyCField {
    byte_offset: usize,
    byte_size: usize,
    index: usize,
    proto: PyTypeRef,
    anonymous: bool,
    bitfield_size: bool,
    bit_offset: u8,
    name: String,
}

impl Representable for PyCFuncPtr {
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        let field = zelf.inner.read();
        let tp_name = field.proto.name().to_string();
        if field.bitfield_size != 0 {
            Ok(format!(
                "<{} {} type={}, ofs={}, bit_size={}, bit_offset={}",
                field.name, tp_name, field.byte_offset, field.bitfield_size, field.bit_offset
            ))
        } else {
            Ok(format!(
                "<{} {} type={}, ofs={}, size={}",
                field.name, tp_name, field.byte_offset, field.byte_size
            ))
        }
    }
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE), with(Representable))]
impl PyCField {
    #[pygetset]
    fn size(&self) -> usize {
        self.byte_size
    }

    #[pygetset]
    fn bit_size(&self) -> u8 {
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
}
