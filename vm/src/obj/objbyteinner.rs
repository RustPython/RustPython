#[derive(Debug, Default, Clone)]
pub struct PyByteInner {
    pub elements: Vec<u8>,
}
impl PyByteInner {
    pub fn new(data: Vec<u8>) -> Self {
        PyByteInner { elements: data }
    }
}
