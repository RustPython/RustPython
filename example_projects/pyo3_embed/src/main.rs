use pyo3::prelude::*;
use pyo3::types::PyInt;

fn main() {
    Python::initialize();

    Python::attach(|py| {
        // let _x = PyInt::new(py, 123);
    });
}
