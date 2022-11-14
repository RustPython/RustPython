use rustpython_vm::{
    builtins::PyStr,
    function::{FuncArgs, KwArgs, PosArgs},
    pyclass, pymodule, PyObject, PyObjectRef, PyPayload, PyResult, TryFromBorrowedObject,
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

        let pystr = PyObjectRef::from(PyStr::new_ref(
            unsafe {
                PyStr::new_ascii_unchecked(String::from("Rust string sent to python").into_bytes())
            },
            vm.as_ref(),
        ));
        let take_string_args = FuncArgs::new(PosArgs::new(vec![pystr]), KwArgs::default());
        let take_string_fn = module.get_attr("take_string", vm).unwrap();

        vm.invoke(&take_string_fn, take_string_args).unwrap();
    })
}

#[pymodule]
mod rust_py_module {
    use super::*;

    #[pyfunction]
    fn rust_function(
        num: i32,
        s: String,
        python_person: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<RustStruct> {
        println!(
            "Calling standalone rust function from python passing args:
num: {},
string: {},
python_person.name: {}",
            num,
            s,
            python_person.try_into_value::<PythonPerson>(vm).unwrap().name
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
                .get_attr("name", vm)
                .unwrap()
                .try_into_value::<String>(vm)
                .unwrap();
            Ok(PythonPerson { name })
        }
    }
}
