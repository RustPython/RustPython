mod json;
use std::collections::HashMap;

use super::pyobject::{PyContext, PyObjectRef};

pub type StdlibInitFunc = fn(&PyContext) -> PyObjectRef;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    let mut modules = HashMap::new();
    modules.insert("json".to_string(), json::mk_module as StdlibInitFunc);
    modules
}
