use crate::pyobject::PyObjectRef;
use crate::vm::VirtualMachine;
use std::collections::HashMap;

pub mod array;
#[cfg(feature = "rustpython-parser")]
pub(crate) mod ast;
mod binascii;
mod collections;
mod csv;
mod dis;
mod errno;
mod functools;
mod hashlib;
mod imp;
pub mod io;
mod itertools;
mod json;
#[cfg(feature = "rustpython-parser")]
mod keyword;
mod marshal;
mod math;
mod operator;
mod platform;
mod pystruct;
mod random;
mod re;
#[cfg(not(target_arch = "wasm32"))]
pub mod socket;
mod string;
#[cfg(feature = "rustpython-compiler")]
mod symtable;
#[cfg(not(target_arch = "wasm32"))]
mod thread;
mod time_module;
#[cfg(feature = "rustpython-parser")]
mod tokenize;
mod unicodedata;
mod warnings;
mod weakref;

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[macro_use]
mod os;

#[cfg(not(target_arch = "wasm32"))]
mod faulthandler;
#[cfg(windows)]
mod msvcrt;
#[cfg(not(target_arch = "wasm32"))]
mod multiprocessing;
#[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
mod pwd;
#[cfg(not(target_arch = "wasm32"))]
mod select;
#[cfg(not(target_arch = "wasm32"))]
pub mod signal;
#[cfg(all(not(target_arch = "wasm32"), feature = "ssl"))]
mod ssl;
#[cfg(not(target_arch = "wasm32"))]
mod subprocess;
#[cfg(windows)]
mod winapi;
#[cfg(windows)]
mod winreg;
#[cfg(not(any(target_arch = "wasm32", target_os = "redox")))]
mod zlib;

pub type StdlibInitFunc = Box<dyn Fn(&VirtualMachine) -> PyObjectRef + Send + Sync>;

pub fn get_module_inits() -> HashMap<String, StdlibInitFunc> {
    #[allow(unused_mut)]
    let mut modules = hashmap! {
        "array".to_owned() => Box::new(array::make_module) as StdlibInitFunc,
        "binascii".to_owned() => Box::new(binascii::make_module),
        "_collections".to_owned() => Box::new(collections::make_module),
        "_csv".to_owned() => Box::new(csv::make_module),
        "dis".to_owned() => Box::new(dis::make_module),
        "errno".to_owned() => Box::new(errno::make_module),
        "_functools".to_owned() => Box::new(functools::make_module),
        "hashlib".to_owned() => Box::new(hashlib::make_module),
        "itertools".to_owned() => Box::new(itertools::make_module),
        "_io".to_owned() => Box::new(io::make_module),
        "_json".to_owned() => Box::new(json::make_module),
        "marshal".to_owned() => Box::new(marshal::make_module),
        "math".to_owned() => Box::new(math::make_module),
        "_operator".to_owned() => Box::new(operator::make_module),
        "_platform".to_owned() => Box::new(platform::make_module),
        "regex_crate".to_owned() => Box::new(re::make_module),
        "_random".to_owned() => Box::new(random::make_module),
        "_string".to_owned() => Box::new(string::make_module),
        "_struct".to_owned() => Box::new(pystruct::make_module),
        "time".to_owned() => Box::new(time_module::make_module),
        "_weakref".to_owned() => Box::new(weakref::make_module),
        "_imp".to_owned() => Box::new(imp::make_module),
        "unicodedata".to_owned() => Box::new(unicodedata::make_module),
        "_warnings".to_owned() => Box::new(warnings::make_module),
    };

    // Insert parser related modules:
    #[cfg(feature = "rustpython-parser")]
    {
        modules.insert(
            "_ast".to_owned(),
            Box::new(ast::make_module) as StdlibInitFunc,
        );
        modules.insert("keyword".to_owned(), Box::new(keyword::make_module));
        modules.insert("tokenize".to_owned(), Box::new(tokenize::make_module));
    }

    // Insert compiler related modules:
    #[cfg(feature = "rustpython-compiler")]
    {
        modules.insert("symtable".to_owned(), Box::new(symtable::make_module));
    }

    #[cfg(any(unix, windows, target_os = "wasi"))]
    modules.insert(os::MODULE_NAME.to_owned(), Box::new(os::make_module));

    // disable some modules on WASM
    #[cfg(not(target_arch = "wasm32"))]
    {
        modules.insert("_socket".to_owned(), Box::new(socket::make_module));
        modules.insert(
            "_multiprocessing".to_owned(),
            Box::new(multiprocessing::make_module),
        );
        modules.insert("signal".to_owned(), Box::new(signal::make_module));
        modules.insert("select".to_owned(), Box::new(select::make_module));
        #[cfg(feature = "ssl")]
        modules.insert("_ssl".to_owned(), Box::new(ssl::make_module));
        modules.insert("_subprocess".to_owned(), Box::new(subprocess::make_module));
        modules.insert("_thread".to_owned(), Box::new(thread::make_module));
        #[cfg(not(target_os = "redox"))]
        modules.insert("zlib".to_owned(), Box::new(zlib::make_module));
        modules.insert(
            "faulthandler".to_owned(),
            Box::new(faulthandler::make_module),
        );
    }

    // Unix-only
    #[cfg(all(unix, not(any(target_os = "android", target_os = "redox"))))]
    {
        modules.insert("pwd".to_owned(), Box::new(pwd::make_module));
    }

    // Windows-only
    #[cfg(windows)]
    {
        modules.insert("msvcrt".to_owned(), Box::new(msvcrt::make_module));
        modules.insert("_winapi".to_owned(), Box::new(winapi::make_module));
        modules.insert("winreg".to_owned(), Box::new(winreg::make_module));
    }

    modules
}
