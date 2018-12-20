mod ast;
mod io;
mod json;
mod keyword;
mod math;
mod pystruct;
mod random;
mod re;
mod string;
mod time_module;
mod tokenize;
mod types;
mod weakref;
use std::collections::HashMap;

use super::pyobject::{PyContext, PyObjectRef};

pub type StdlibInitFunc = fn(&PyContext) -> PyObjectRef;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    let mut modules = HashMap::new();
    modules.insert("ast".to_string(), ast::mk_module as StdlibInitFunc);
    modules.insert("io".to_string(), io::mk_module as StdlibInitFunc);
    modules.insert("json".to_string(), json::mk_module as StdlibInitFunc);
    modules.insert("keyword".to_string(), keyword::mk_module as StdlibInitFunc);
    modules.insert("math".to_string(), math::mk_module as StdlibInitFunc);
    modules.insert("re".to_string(), re::mk_module as StdlibInitFunc);
    modules.insert("random".to_string(), random::mk_module as StdlibInitFunc);
    modules.insert("string".to_string(), string::mk_module as StdlibInitFunc);
    modules.insert("struct".to_string(), pystruct::mk_module as StdlibInitFunc);
    modules.insert("time".to_string(), time_module::mk_module as StdlibInitFunc);
    modules.insert(
        "tokenize".to_string(),
        tokenize::mk_module as StdlibInitFunc,
    );
    modules.insert("types".to_string(), types::mk_module as StdlibInitFunc);
    modules.insert("_weakref".to_string(), weakref::mk_module as StdlibInitFunc);
    modules
}
