#[pyclass(name = "Structure", module = "_ctypes")]
pub struct PyCStructure {
}

#[pyclass(flags(BASETYPE, IMMUTABLETYPE))]
impl PyCStructure {

}
