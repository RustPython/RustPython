#[cfg(feature = "rustpython_parser")]
mod ast;
mod binascii;
mod dis;
mod hashlib;
mod imp;
mod itertools;
mod json;
#[cfg(feature = "rustpython_parser")]
mod keyword;
mod marshal;
mod math;
mod platform;
mod pystruct;
mod random;
mod re;
pub mod socket;
mod string;
mod thread;
mod time_module;
#[cfg(feature = "rustpython_parser")]
mod tokenize;
mod unicodedata;
mod warnings;
mod weakref;
use std::collections::HashMap;

use crate::vm::VirtualMachine;

#[cfg(not(target_arch = "wasm32"))]
pub mod io;
#[cfg(not(target_arch = "wasm32"))]
mod os;
#[cfg(all(unix, not(target_os = "android")))]
mod pwd;

use crate::pyobject::PyObjectRef;

pub type StdlibInitFunc = Box<dyn Fn(&VirtualMachine) -> PyObjectRef>;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    #[allow(unused_mut)]
    let mut modules = hashmap! {
        "binascii".to_string() => Box::new(binascii::make_module) as StdlibInitFunc,
        "dis".to_string() => Box::new(dis::make_module) as StdlibInitFunc,
        "hashlib".to_string() => Box::new(hashlib::make_module),
        "itertools".to_string() => Box::new(itertools::make_module),
        "json".to_string() => Box::new(json::make_module),
        "marshal".to_string() => Box::new(marshal::make_module),
        "math".to_string() => Box::new(math::make_module),
        "platform".to_string() => Box::new(platform::make_module),
        "re".to_string() => Box::new(re::make_module),
        "random".to_string() => Box::new(random::make_module),
        "string".to_string() => Box::new(string::make_module),
        "struct".to_string() => Box::new(pystruct::make_module),
        "_thread".to_string() => Box::new(thread::make_module),
        "time".to_string() => Box::new(time_module::make_module),
        "_weakref".to_string() => Box::new(weakref::make_module),
        "_imp".to_string() => Box::new(imp::make_module),
        "unicodedata".to_string() => Box::new(unicodedata::make_module),
        "_warnings".to_string() => Box::new(warnings::make_module),
    };

    // Insert parser related modules:
    #[cfg(feature = "rustpython_parser")]
    {
        modules.insert(
            "ast".to_string(),
            Box::new(ast::make_module) as StdlibInitFunc,
        );
        modules.insert("keyword".to_string(), Box::new(keyword::make_module));
        modules.insert("tokenize".to_string(), Box::new(tokenize::make_module));
    }

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("_io".to_string(), Box::new(io::make_module));
        modules.insert("_os".to_string(), Box::new(os::make_module));
        modules.insert("socket".to_string(), Box::new(socket::make_module));
    }

    // Unix-only
    #[cfg(all(unix, not(target_os = "android")))]
    {
        modules.insert("pwd".to_string(), Box::new(pwd::make_module));
    }

    modules
}
