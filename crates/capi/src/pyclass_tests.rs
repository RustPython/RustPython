use pyo3::Python;
use pyo3::types::PyAnyMethods;

#[pyo3::pyclass]
struct Point {
    x: i64,
}

#[pyo3::pymethods]
impl Point {
    #[new]
    fn new(x: i64) -> Self {
        Self { x }
    }

    fn get(&self) -> i64 {
        self.x
    }
}

#[test]
fn pyclass_alloc_call_and_method() {
    Python::attach(|py| {
        let p = pyo3::Py::new(py, Point { x: 1 }).unwrap();
        assert_eq!(p.borrow(py).x, 1);

        let ty = py.get_type::<Point>();
        let p2 = ty.call1((2,)).unwrap();
        let p2 = p2.cast_into::<Point>().unwrap();
        assert_eq!(p2.borrow().x, 2);

        let got: i64 = p2
            .getattr("get")
            .unwrap()
            .call0()
            .unwrap()
            .extract()
            .unwrap();
        assert_eq!(got, 2);
    });
}
