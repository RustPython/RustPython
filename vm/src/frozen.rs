use crate::bytecode::CodeObject;
use std::collections::HashMap;

lazy_static! {
    static ref HELLO: CodeObject = py_compile_bytecode!(
        source = "initialized = True
print(\"Hello world!\")
",
    );
    static ref IMPORTLIB_BOOTSTRAP: CodeObject =
        py_compile_bytecode!(file = "../Lib/importlib/_bootstrap.py");
    static ref IMPORTLIB_BOOTSTRAP_EXTERNAL: CodeObject =
        py_compile_bytecode!(file = "../Lib/importlib/_bootstrap_external.py");
}

pub fn get_module_inits() -> HashMap<&'static str, &'static CodeObject> {
    hashmap! {
        "__hello__" => &*HELLO,
        "_frozen_importlib_external" => &*IMPORTLIB_BOOTSTRAP_EXTERNAL,
        "_frozen_importlib" => &*IMPORTLIB_BOOTSTRAP,
    }
}
