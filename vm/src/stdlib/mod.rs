mod ast;
mod dis;
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

#[cfg(not(target_arch = "wasm32"))]
pub mod io;
#[cfg(not(target_arch = "wasm32"))]
mod os;

use crate::pyobject::{PyContext, PyObjectRef};

pub type StdlibInitFunc = Box<dyn Fn(&PyContext) -> PyObjectRef>;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    let mut modules = HashMap::new();
    modules.insert("ast".to_string(), Box::new(ast::mk_module) as StdlibInitFunc);
    modules.insert("dis".to_string(), Box::new(dis::mk_module) as StdlibInitFunc);
    modules.insert("json".to_string(), Box::new(json::mk_module) as StdlibInitFunc);
    modules.insert("keyword".to_string(), Box::new(keyword::mk_module) as StdlibInitFunc);
    modules.insert("math".to_string(), Box::new(math::mk_module) as StdlibInitFunc);
    modules.insert("re".to_string(), Box::new(re::mk_module) as StdlibInitFunc);
    modules.insert("random".to_string(), Box::new(random::mk_module) as StdlibInitFunc);
    modules.insert("string".to_string(), Box::new(string::mk_module) as StdlibInitFunc);
    modules.insert("struct".to_string(), Box::new(pystruct::mk_module) as StdlibInitFunc);
    modules.insert("time".to_string(), Box::new(time_module::mk_module) as StdlibInitFunc);
    modules.insert( "tokenize".to_string(), Box::new(tokenize::mk_module) as StdlibInitFunc);
    modules.insert("types".to_string(), Box::new(types::mk_module) as StdlibInitFunc);
    modules.insert("_weakref".to_string(), Box::new(weakref::mk_module) as StdlibInitFunc);

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("io".to_string(), Box::new(io::mk_module) as StdlibInitFunc);
        modules.insert("os".to_string(), Box::new(os::mk_module) as StdlibInitFunc);
    }

    modules
}
