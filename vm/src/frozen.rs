use crate::bytecode::FrozenModule;
use std::collections::HashMap;

pub fn get_module_inits() -> HashMap<String, FrozenModule> {
    let mut modules = HashMap::new();
    modules.extend(py_compile_bytecode!(
        source = "initialized = True; print(\"Hello world!\")\n",
        module_name = "__hello__",
    ));
    modules.extend(py_compile_bytecode!(
        file = "Lib/_bootstrap.py",
        module_name = "_frozen_importlib",
    ));
    modules.extend(py_compile_bytecode!(
        file = "Lib/_bootstrap_external.py",
        module_name = "_frozen_importlib_external",
    ));
    modules.extend(py_compile_bytecode!(
        file = "../Lib/copyreg.py",
        module_name = "copyreg",
    ));
    modules.extend(py_compile_bytecode!(
        file = "Lib/__reducelib.py",
        module_name = "__reducelib",
    ));

    #[cfg(feature = "freeze-stdlib")]
    {
        modules.extend(py_compile_bytecode!(dir = "../Lib/"));
    }

    modules
}
