use rustpython::vm::{
    pyclass, pymodule, PyObject, PyPayload, PyResult, TryFromBorrowedObject, VirtualMachine,
};

pub fn main() {
    let interp = rustpython::InterpreterConfig::new()
        .init_stdlib()
        .init_hook(Box::new(|vm| {
            vm.add_native_module(
                "rust_py_module".to_owned(),
                Box::new(rust_py_module::make_module),
            );
        }))
        .interpreter();

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
    use rustpython::vm::{builtins::PyList, convert::ToPyObject, PyObjectRef};

    #[pyfunction]
    fn rust_function(
        num: i32,
        s: String,
        python_person: PythonPerson,
        _vm: &VirtualMachine,
    ) -> PyResult<RustStruct> {
        println!(
            "Calling standalone rust function from python passing args:
num: {},
string: {},
python_person.name: {}",
            num, s, python_person.name
        );
        Ok(RustStruct {
            numbers: NumVec(vec![1, 2, 3, 4]),
        })
    }

    #[derive(Debug, Clone)]
    struct NumVec(Vec<i32>);

    impl ToPyObject for NumVec {
        fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
            let list = self.0.into_iter().map(|e| vm.new_pyobj(e)).collect();
            PyList::new_ref(list, vm.as_ref()).to_pyobject(vm)
        }
    }

    #[pyattr]
    #[pyclass(module = "rust_py_module", name = "RustStruct")]
    #[derive(Debug, PyPayload)]
    struct RustStruct {
        numbers: NumVec,
    }

    #[pyclass]
    impl RustStruct {
        #[pygetset]
        fn numbers(&self) -> NumVec {
            self.numbers.clone()
        }

        #[pymethod]
        fn print_in_rust_from_python(&self) {
            println!("Calling a rust method from python");
        }
    }

    struct PythonPerson {
        name: String,
    }

    impl<'a> TryFromBorrowedObject<'a> for PythonPerson {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let name = obj.get_attr("name", vm)?.try_into_value::<String>(vm)?;
            Ok(PythonPerson { name })
        }
    }
}
