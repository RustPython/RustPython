extern crate alloc;

use rustpython::InterpreterBuilderExt;
use rustpython::vm::{
    Py, PyObject, PyPayload, PyResult, TryFromBorrowedObject, VirtualMachine, pyclass, pymodule,
};

pub fn main() {
    let builder = rustpython::Interpreter::builder(Default::default());
    let def = rust_py_module::module_def(&builder.ctx);
    let interp = builder.init_stdlib().add_native_module(def).build();

    interp.enter(|vm| {
        vm.insert_sys_path(vm.new_pyobj("examples"))
            .expect("add path");

        let module = vm.import("call_between_rust_and_python", 0).unwrap();
        let init_fn = module.get_attr("python_callback", vm).unwrap();
        init_fn.call((), vm).unwrap();

        let take_string_fn = module.get_attr("take_string", vm).unwrap();
        take_string_fn
            .call((String::from("Rust string sent to python"),), vm)
            .unwrap();
    })
}

#[pymodule]
mod rust_py_module {
    use super::*;
    use alloc::collections::BTreeMap;
    use rustpython::vm::{
        PyObjectRef,
        convert::{PyListOf, ToPyObject},
    };

    #[pyfunction]
    fn rust_function(
        num: i32,
        s: String,
        python_person: PythonPerson,
        _vm: &VirtualMachine,
    ) -> RustStruct {
        println!(
            "Calling standalone rust function from python passing args:
num: {},
string: {},
python_person.name: {}",
            num, s, python_person.name
        );
        RustStruct {
            numbers: vec![1, 2, 3, 4],
            measurements: Measurements(BTreeMap::from([
                ("temperature", 21.5),
                ("pressure", 1013.0),
            ])),
        }
    }

    // A domain type can choose its own Python representation. Here an ordered
    // map becomes a list of pairs rather than a dictionary.
    #[derive(Debug, Clone)]
    struct Measurements(BTreeMap<&'static str, f64>);

    impl ToPyObject for Measurements {
        fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
            let pairs = self
                .0
                .into_iter()
                .map(|pair| pair.to_pyobject(vm))
                .collect();
            vm.ctx.new_list(pairs).into()
        }
    }

    #[pyattr]
    #[pyclass(module = "rust_py_module", name = "RustStruct")]
    #[derive(Debug, PyPayload)]
    struct RustStruct {
        numbers: Vec<i32>,
        measurements: Measurements,
    }

    #[pyclass]
    impl RustStruct {
        #[pygetset]
        fn numbers(zelf: &Py<Self>, vm: &VirtualMachine) -> PyListOf<i32> {
            PyListOf::new(zelf.numbers.iter().copied(), vm)
        }

        #[pygetset]
        fn measurements(zelf: &Py<Self>) -> Measurements {
            zelf.measurements.clone()
        }

        #[pymethod]
        fn print_in_rust_from_python(_zelf: &Py<Self>) {
            println!("Calling a rust method from python");
        }
    }

    struct PythonPerson {
        name: String,
    }

    impl<'a> TryFromBorrowedObject<'a> for PythonPerson {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let name = obj.get_attr("name", vm)?.try_into_value::<String>(vm)?;
            Ok(Self { name })
        }
    }
}
