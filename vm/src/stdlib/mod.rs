mod json;
use std::collections::HashMap;

use super::pyobject::{PyContext, PyObjectRef};

pub fn get_modules(ctx: &PyContext) -> HashMap<String, PyObjectRef> {
    let mut modules = HashMap::new();
    modules.insert("json".to_string(), json::mk_module(ctx));
    modules
}
