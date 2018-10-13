mod ast;
mod io;
mod json;
mod keyword;
mod math;
use std::collections::HashMap;

use super::pyobject::{PyContext, PyObjectRef};

pub type StdlibInitFunc = fn(&PyContext) -> PyObjectRef;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    let mut modules = HashMap::new();
    modules.insert("io".to_string(), io::mk_module as StdlibInitFunc);
    modules.insert("json".to_string(), json::mk_module as StdlibInitFunc);
    modules.insert("ast".to_string(), ast::mk_module as StdlibInitFunc);
    modules.insert("keyword".to_string(), keyword::mk_module as StdlibInitFunc);
    modules.insert("math".to_string(), math::mk_module as StdlibInitFunc);
    modules
}
