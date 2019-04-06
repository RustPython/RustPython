mod ast;
mod dis;
pub(crate) mod json;
mod keyword;
mod math;
mod platform;
mod pystruct;
mod random;
mod re;
pub mod socket;
mod string;
mod thread;
mod time_module;
mod tokenize;
mod types;
mod weakref;
use std::collections::HashMap;

use crate::vm::VirtualMachine;

#[cfg(not(target_arch = "wasm32"))]
pub mod io;
#[cfg(not(target_arch = "wasm32"))]
mod os;

use crate::pyobject::PyObjectRef;

pub type StdlibInitFunc = Box<dyn Fn(&VirtualMachine) -> PyObjectRef>;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    let mut modules = HashMap::new();
    modules.insert(
        "ast".to_string(),
        Box::new(ast::make_module) as StdlibInitFunc,
    );
    modules.insert("dis".to_string(), Box::new(dis::make_module));
    modules.insert("json".to_string(), Box::new(json::make_module));
    modules.insert("keyword".to_string(), Box::new(keyword::make_module));
    modules.insert("math".to_string(), Box::new(math::make_module));
    modules.insert("platform".to_string(), Box::new(platform::make_module));
    modules.insert("re".to_string(), Box::new(re::make_module));
    modules.insert("random".to_string(), Box::new(random::make_module));
    modules.insert("string".to_string(), Box::new(string::make_module));
    modules.insert("struct".to_string(), Box::new(pystruct::make_module));
    modules.insert("_thread".to_string(), Box::new(thread::make_module));
    modules.insert("time".to_string(), Box::new(time_module::make_module));
    modules.insert("tokenize".to_string(), Box::new(tokenize::make_module));
    modules.insert("types".to_string(), Box::new(types::make_module));
    modules.insert("_weakref".to_string(), Box::new(weakref::make_module));

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("io".to_string(), Box::new(io::make_module));
        modules.insert("_os".to_string(), Box::new(os::make_module));
        modules.insert("socket".to_string(), Box::new(socket::make_module));
    }

    modules
}
