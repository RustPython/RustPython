use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyInt, PyNone, PyString};


#[pyfunction]
fn python_function(py: Python<'_>) -> Borrowed<'_, '_, PyNone> {
    PyNone::get(py)
}

fn main() {
    Python::initialize();

    Python::attach(|py| {
        let number = PyInt::new(py, 123);
        assert!(number.is_instance_of::<PyInt>());
        assert_eq!(number.extract::<i32>()?, 123);

        let string = PyString::new(py, "Hello, World!");
        assert!(string.is_instance_of::<PyString>());
        assert_eq!(string.to_str()?, "Hello, World!");

        assert_eq!(string.get_type().name()?.to_str()?, "str");

        let number = number.unbind();
        std::thread::spawn(move || {
            Python::attach(|py| {
                let number = number.bind(py);
                assert!(number.is_instance_of::<PyInt>());
            });
        })
        .join()
        .unwrap();

        assert!(python_function(py).is_none());

        let bytes = PyBytes::new(py, b"Hello, World!");
        assert_eq!(bytes.as_bytes(), b"Hello, World!");

        PyResult::Ok(())
    })
    .unwrap();
}
