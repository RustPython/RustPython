use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};

use pyo3::Python;
use pyo3::types::{PyAnyMethods, PyDict, PyDictMethods};

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

#[test]
fn pyclass_call_method0() {
    Python::attach(|py| {
        let ty = py.get_type::<Point>();
        let p2 = ty.call1((2,)).unwrap();
        let got: i64 = p2.call_method0("get").unwrap().extract().unwrap();
        assert_eq!(got, 2);
    });
}

#[pyo3::pyclass(subclass)]
struct Base {
    x: i64,
}

#[pyo3::pymethods]
impl Base {
    #[new]
    fn new(x: i64) -> Self {
        Self { x }
    }

    fn get(&self) -> i64 {
        self.x
    }
}

#[test]
fn pyclass_python_subclass() {
    Python::attach(|py| {
        let locals = PyDict::new(py);
        locals.set_item("Base", py.get_type::<Base>()).unwrap();
        py.run(
            c"class Sub(Base): pass
obj = Sub(3)
",
            None,
            Some(&locals),
        )
        .unwrap();
        let obj = locals.get_item("obj").unwrap().unwrap();
        let got: i64 = obj.call_method0("get").unwrap().extract().unwrap();
        assert_eq!(got, 3);
        let base = obj.cast::<Base>().unwrap();
        assert_eq!(base.borrow().x, 3);

        let locals = PyDict::new(py);
        locals.set_item("Point", py.get_type::<Point>()).unwrap();
        let err = py
            .run(c"class Sub(Point): pass", None, Some(&locals))
            .unwrap_err();
        assert!(err.is_instance_of::<pyo3::exceptions::PyTypeError>(py));
    });
}

#[pyo3::pyclass]
struct Dropper {
    flag: Arc<AtomicBool>,
}

impl Drop for Dropper {
    fn drop(&mut self) {
        self.flag.store(true, Ordering::SeqCst);
    }
}

static DROPPER_FLAG: std::sync::LazyLock<std::sync::Mutex<Arc<AtomicBool>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Arc::new(AtomicBool::new(false))));

#[pyo3::pymethods]
impl Dropper {
    #[new]
    fn new() -> Self {
        let flag = match DROPPER_FLAG.lock() {
            Ok(flag) => flag.clone(),
            Err(err) => err.into_inner().clone(),
        };
        Self { flag }
    }
}

#[test]
fn pyclass_drop_runs() {
    Python::attach(|py| {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let obj = pyo3::Py::new(py, Dropper { flag: flag.clone() }).unwrap();
            drop(obj);
        }
        py.run(c"import gc; gc.collect()", None, None).unwrap();
        assert!(flag.load(Ordering::SeqCst));

        let py_flag = Arc::new(AtomicBool::new(false));
        *DROPPER_FLAG.lock().unwrap() = py_flag.clone();
        let locals = PyDict::new(py);
        locals
            .set_item("Dropper", py.get_type::<Dropper>())
            .unwrap();
        py.run(c"obj = Dropper()", None, Some(&locals)).unwrap();
        locals.del_item("obj").unwrap();
        py.run(c"import gc; gc.collect()", None, Some(&locals))
            .unwrap();
        assert!(py_flag.load(Ordering::SeqCst));
    });
}
