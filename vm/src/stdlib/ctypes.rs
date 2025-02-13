pub(crate) use _ctypes::make_module;

#[pymodule]
mod _ctypes {
    use crate::{common::lock::PyRwLock, PyObjectRef};
    use crossbeam_utils::atomic::AtomicCell;

    pub struct RawBuffer {
        #[allow(dead_code)]
        pub inner: Box<[u8]>,
        #[allow(dead_code)]
        pub size: usize,
    }

    #[pyattr]
    #[pyclass(name = "_CData")]
    pub struct PyCData {
        _objects: AtomicCell<Vec<PyObjectRef>>,
        _buffer: PyRwLock<RawBuffer>,
    }

    #[pyclass]
    impl PyCData {}

    #[pyfunction]
    fn get_errno() -> i32 {
        errno::errno().0
    }

    #[pyfunction]
    fn set_errno(value: i32) {
        errno::set_errno(errno::Errno(value));
    }
}
