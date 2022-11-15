use rustpython_vm::{
    pyclass, pymodule, PyObject, PyPayload, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};

pub(crate) use rust_py_module::make_module;

pub fn main() {
    let interp = rustpython_vm::Interpreter::with_init(Default::default(), |vm| {
        vm.add_native_modules(rustpython_stdlib::get_module_inits());
        vm.add_native_module("rust_py_module".to_owned(), Box::new(make_module));
    });

    interp.enter(|vm| {
        vm.insert_sys_path(vm.new_pyobj("examples"))
            .expect("add path");

        let module = vm.import("call_between_rust_and_python", None, 0).unwrap();
        let init_fn = module.get_attr("python_callback", vm).unwrap();
        vm.invoke(&init_fn, ()).unwrap();

        let take_string_fn = module.get_attr("take_string", vm).unwrap();
        vm.invoke(&take_string_fn, (String::from("Rust string sent to python"),)).unwrap();
    })
}

#[pymodule]
mod rust_py_module {
    use super::*;

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
            num,
            s,
            python_person.name
        );
        Ok(RustStruct)
    }

    #[pyattr]
    #[pyclass(module = "rust_py_module", name = "RustStruct")]
    #[derive(Debug, PyPayload)]
    struct RustStruct;

    #[pyclass]
    impl RustStruct {
        #[pymethod]
        fn print_in_rust_from_python(&self) {
            println!("Calling a rust method from python");
        }
    }

    struct PythonPerson {
        name: String,
    }

    impl TryFromBorrowedObject for PythonPerson {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Self> {
            let name = obj
                .get_attr("name", vm)?
                .try_into_value::<String>(vm)?;
            Ok(PythonPerson { name })
        }
    }
}
