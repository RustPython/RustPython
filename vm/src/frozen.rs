use crate::bytecode::CodeObject;
use std::collections::HashMap;

pub fn get_module_inits() -> HashMap<String, CodeObject> {
    hashmap! {
        "__hello__".into() => py_compile_bytecode!(
            source = "initialized = True; print(\"Hello world!\")\n",
            module_name = "__hello__",
        ),
        "_frozen_importlib".into() => py_compile_bytecode!(
            file = "Lib/_bootstrap.py",
            module_name = "_frozen_importlib",
        ),
        "_frozen_importlib_external".into() => py_compile_bytecode!(
            file = "Lib/_bootstrap_external.py",
            module_name = "_frozen_importlib_external",
        ),
    }
}
