pub mod array;
#[cfg(feature = "rustpython-parser")]
pub(crate) mod ast;
mod binascii;
mod codecs;
mod collections;
mod csv;
mod dis;
mod errno;
mod functools;
mod hashlib;
mod imp;
mod itertools;
mod json;
#[cfg(feature = "rustpython-parser")]
mod keyword;
mod marshal;
mod math;
mod platform;
mod pystruct;
mod random;
mod re;
#[cfg(not(target_arch = "wasm32"))]
pub mod socket;
mod string;
#[cfg(feature = "rustpython-compiler")]
mod symtable;
mod thread;
mod time_module;
#[cfg(feature = "rustpython-parser")]
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
#[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
mod pwd;
#[cfg(not(target_arch = "wasm32"))]
mod select;
#[cfg(not(target_arch = "wasm32"))]
pub mod signal;
#[cfg(not(target_arch = "wasm32"))]
mod subprocess;
#[cfg(not(target_arch = "wasm32"))]
mod zlib;

use crate::pyobject::PyObjectRef;

pub type StdlibInitFunc = Box<dyn Fn(&VirtualMachine) -> PyObjectRef>;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    #[allow(unused_mut)]
    let mut modules = hashmap! {
        "array".to_string() => Box::new(array::make_module) as StdlibInitFunc,
        "binascii".to_string() => Box::new(binascii::make_module),
        "dis".to_string() => Box::new(dis::make_module),
        "_codecs".to_string() => Box::new(codecs::make_module),
        "_collections".to_string() => Box::new(collections::make_module),
        "_csv".to_string() => Box::new(csv::make_module),
        "_functools".to_string() => Box::new(functools::make_module),
        "errno".to_string() => Box::new(errno::make_module),
        "hashlib".to_string() => Box::new(hashlib::make_module),
        "itertools".to_string() => Box::new(itertools::make_module),
        "json".to_string() => Box::new(json::make_module),
        "marshal".to_string() => Box::new(marshal::make_module),
        "math".to_string() => Box::new(math::make_module),
        "platform".to_string() => Box::new(platform::make_module),
        "regex_crate".to_string() => Box::new(re::make_module),
        "random".to_string() => Box::new(random::make_module),
        "_string".to_string() => Box::new(string::make_module),
        "struct".to_string() => Box::new(pystruct::make_module),
        "_thread".to_string() => Box::new(thread::make_module),
        "time".to_string() => Box::new(time_module::make_module),
        "_weakref".to_string() => Box::new(weakref::make_module),
        "_imp".to_string() => Box::new(imp::make_module),
        "unicodedata".to_string() => Box::new(unicodedata::make_module),
        "_warnings".to_string() => Box::new(warnings::make_module),
    };

    // Insert parser related modules:
    #[cfg(feature = "rustpython-parser")]
    {
        modules.insert(
            "_ast".to_string(),
            Box::new(ast::make_module) as StdlibInitFunc,
        );
        modules.insert("keyword".to_string(), Box::new(keyword::make_module));
        modules.insert("tokenize".to_string(), Box::new(tokenize::make_module));
    }

    // Insert compiler related modules:
    #[cfg(feature = "rustpython-compiler")]
    {
        modules.insert("symtable".to_string(), Box::new(symtable::make_module));
    }

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("_io".to_string(), Box::new(io::make_module));
        modules.insert("_os".to_string(), Box::new(os::make_module));
        modules.insert("socket".to_string(), Box::new(socket::make_module));
        modules.insert("signal".to_string(), Box::new(signal::make_module));
        modules.insert("select".to_string(), Box::new(select::make_module));
        modules.insert("_subprocess".to_string(), Box::new(subprocess::make_module));
        modules.insert("zlib".to_string(), Box::new(zlib::make_module));
    }

    // Unix-only
    #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
    {
        modules.insert("pwd".to_string(), Box::new(pwd::make_module));
    }

    modules
}
