mod ast;
mod dis;
mod json;
mod keyword;
mod math;
mod platform;
mod pystruct;
mod random;
mod re;
pub mod socket;
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
    modules.insert(
        "ast".to_string(),
        Box::new(ast::mk_module) as StdlibInitFunc,
    );
    modules.insert("dis".to_string(), Box::new(dis::mk_module));
    modules.insert("json".to_string(), Box::new(json::mk_module));
    modules.insert("keyword".to_string(), Box::new(keyword::mk_module));
    modules.insert("math".to_string(), Box::new(math::mk_module));
    modules.insert("platform".to_string(), Box::new(platform::mk_module));
    modules.insert("re".to_string(), Box::new(re::mk_module));
    modules.insert("random".to_string(), Box::new(random::mk_module));
    modules.insert("string".to_string(), Box::new(string::mk_module));
    modules.insert("struct".to_string(), Box::new(pystruct::mk_module));
    modules.insert("time".to_string(), Box::new(time_module::mk_module));
    modules.insert("tokenize".to_string(), Box::new(tokenize::mk_module));
    modules.insert("types".to_string(), Box::new(types::mk_module));
    modules.insert("_weakref".to_string(), Box::new(weakref::mk_module));

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("io".to_string(), Box::new(io::mk_module));
        modules.insert("os".to_string(), Box::new(os::mk_module));
        modules.insert("socket".to_string(), Box::new(socket::mk_module));
    }

    modules
}
