use pyo3::prelude::*;
use pyo3::types::{PyInt, PyString};

fn main() {
    Python::initialize();

    Python::attach(|py| {
        let number = PyInt::new(py, 123);
        assert!(number.is_instance_of::<PyInt>());

        let string = PyString::new(py, "Hello, World!");
        assert!(string.is_instance_of::<PyString>());
    });
}
