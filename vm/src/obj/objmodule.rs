use crate::obj::objdict::PyDictRef;
use crate::obj::objproperty::PropertyBuilder;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{ItemProtocol, PyContext, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

#[derive(Debug)]
pub struct PyModule {
    pub name: String,
}
pub type PyModuleRef = PyRef<PyModule>;

impl PyValue for PyModule {
    const HAVE_DICT: bool = true;

    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.module_type()
    }
}

impl PyModuleRef {
    fn new(cls: PyClassRef, name: PyStringRef, vm: &VirtualMachine) -> PyResult<PyModuleRef> {
        PyModule {
            name: name.as_str().to_string(),
        }
        .into_ref_with_type(vm, cls)
    }

    fn dir(self: PyModuleRef, vm: &VirtualMachine) -> PyResult {
        if let Some(dict) = &self.into_object().dict {
            let keys = dict.into_iter().map(|(k, _v)| k.clone()).collect();
            Ok(vm.ctx.new_list(keys))
        } else {
            panic!("Modules should definitely have a dict.");
        }
    }

    fn dict(self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let name_obj = vm.new_str(self.name.clone());
        if let Some(ref dict) = &self.into_object().dict {
            let mod_dict = dict.clone();
            mod_dict.set_item("__name__", name_obj, vm)?;
            Ok(mod_dict)
        } else {
            panic!("Modules should definitely have a dict.");
        }
    }

    fn name(self, _vm: &VirtualMachine) -> String {
        self.name.clone()
    }
}

pub fn init(context: &PyContext) {
    extend_class!(&context, &context.module_type, {
        "__dir__" => context.new_rustfunc(PyModuleRef::dir),
        "__name__" => PropertyBuilder::new(context)
                .add_getter(PyModuleRef::name)
                .create(),
        "__dict__" =>
        PropertyBuilder::new(context)
                .add_getter(PyModuleRef::dict)
                .create(),
        "__new__" => context.new_rustfunc(PyModuleRef::new),
    });
}
