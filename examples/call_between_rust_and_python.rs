use rustpython::InterpreterBuilderExt;
use rustpython::vm::{
    Py, PyObject, PyPayload, PyResult, TryFromBorrowedObject, VirtualMachine, pyclass, pymodule,
};

pub fn main() {
    let builder = rustpython::Interpreter::builder(Default::default());
    let def = rust_py_module::module_def(unsafe { builder.context() });
    // SAFETY: module callbacks use raw objects only within their native entry.
    let interp = unsafe { builder.init_stdlib().add_native_module(def) }.build();

    interp.enter(|vm| {
        vm.exec("import sys; sys.path.insert(0, 'examples')")
            .unwrap();

        let module = vm.import("call_between_rust_and_python").unwrap();
        let init_fn = module.get_attr("python_callback").unwrap();
        init_fn.call(&[]).unwrap();

        let take_string_fn = module.get_attr("take_string").unwrap();
        take_string_fn
            .call(&[vm.new_str("Rust string sent to python").unwrap()])
            .unwrap();
    })
}

#[pymodule]
mod rust_py_module {
    use super::*;
    use rustpython::vm::{PyObjectRef, convert::ToPyObject};

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
            numbers: NumVec(vec![1, 2, 3, 4]),
        }
    }

    #[derive(Debug, Clone)]
    struct NumVec(Vec<i32>);

    impl ToPyObject for NumVec {
        fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
            let list = self.0.into_iter().map(|e| vm.new_pyobj(e)).collect();
            vm.ctx.new_list(list).to_pyobject(vm)
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
        fn numbers(zelf: &Py<Self>) -> NumVec {
            zelf.numbers.clone()
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
