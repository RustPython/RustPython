use crate::bytecode::CodeObject;
use std::collections::HashMap;

pub fn get_module_inits() -> HashMap<&'static str, &'static CodeObject> {
    hashmap! {
        "__hello__" => py_compile_bytecode!(
            lazy_static,
            source = "initialized = True; print(\"Hello world!\")\n",
        ),
        "_frozen_importlib" => py_compile_bytecode!(
            lazy_static,
            file = "../Lib/importlib/_bootstrap.py",
        ),
        "_frozen_importlib_external" => py_compile_bytecode!(
            lazy_static,
            file = "../Lib/importlib/_bootstrap_external.py",
        ),
    }
}
