use std::rc::Rc;
use std::{env, mem};

use crate::frame::FrameRef;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{
    IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyResult, TypeProtocol,
};
use crate::version;
use crate::vm::{PySettings, VirtualMachine};

/*
 * The magic sys module.
 */

fn argv(vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.new_list(
        vm.settings
            .argv
            .iter()
            .map(|arg| vm.new_str(arg.to_owned()))
            .collect(),
    )
}

fn executable(ctx: &PyContext) -> PyObjectRef {
    if let Some(arg) = env::args().next() {
        ctx.new_str(arg)
    } else {
        ctx.none()
    }
}

fn getframe(offset: OptionalArg<usize>, vm: &VirtualMachine) -> PyResult<FrameRef> {
    let offset = offset.into_option().unwrap_or(0);
    if offset > vm.frames.borrow().len() - 1 {
        return Err(vm.new_value_error("call stack is not deep enough".to_string()));
    }
    let idx = vm.frames.borrow().len() - offset - 1;
    let frame = &vm.frames.borrow()[idx];
    Ok(frame.clone())
}

/// sys.flags
///
/// Flags provided through command line arguments or environment vars.
#[pystruct_sequence(name = "flags")]
#[derive(Default, Debug)]
struct SysFlags {
    /// -d
    debug: bool,
    /// -i
    inspect: bool,
    /// -i
    interactive: bool,
    /// -O or -OO
    optimize: u8,
    /// -B
    dont_write_bytecode: bool,
    /// -s
    no_user_site: bool,
    /// -S
    no_site: bool,
    /// -E
    ignore_environment: bool,
    /// -v
    verbose: u8,
    /// -b
    bytes_warning: bool,
    /// -q
    quiet: bool,
    /// -R
    hash_randomization: bool,
    /// -I
    isolated: bool,
    /// -X dev
    dev_mode: bool,
    /// -X utf8
    utf8_mode: bool,
}

impl SysFlags {
    fn from_settings(settings: &PySettings) -> Self {
        // Start with sensible defaults:
        let mut flags: SysFlags = Default::default();
        flags.debug = settings.debug;
        flags.inspect = settings.inspect;
        flags.optimize = settings.optimize;
        flags.no_user_site = settings.no_user_site;
        flags.no_site = settings.no_site;
        flags.ignore_environment = settings.ignore_environment;
        flags.verbose = settings.verbose;
        flags.quiet = settings.quiet;
        flags.dont_write_bytecode = settings.dont_write_bytecode;
        flags
    }
}

fn sys_getrefcount(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    let size = Rc::strong_count(&object);
    Ok(vm.ctx.new_int(size))
}

fn sys_getsizeof(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(object, None)]);
    // TODO: implement default optional argument.
    let size = mem::size_of_val(&object);
    Ok(vm.ctx.new_int(size))
}

fn sys_getfilesystemencoding(_vm: &VirtualMachine) -> String {
    // TODO: implmement non-utf-8 mode.
    "utf-8".to_string()
}

#[cfg(not(windows))]
fn sys_getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
    "surrogateescape".to_string()
}

#[cfg(windows)]
fn sys_getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
    "surrogatepass".to_string()
}

fn sys_getprofile(vm: &VirtualMachine) -> PyObjectRef {
    vm.profile_func.borrow().clone()
}

fn sys_setprofile(profilefunc: PyObjectRef, vm: &VirtualMachine) {
    vm.profile_func.replace(profilefunc);
    update_use_tracing(vm);
}

fn sys_gettrace(vm: &VirtualMachine) -> PyObjectRef {
    vm.trace_func.borrow().clone()
}

fn sys_settrace(tracefunc: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
    vm.trace_func.replace(tracefunc);
    update_use_tracing(vm);
    vm.ctx.none()
}

fn update_use_tracing(vm: &VirtualMachine) {
    let trace_is_none = vm.is_none(&vm.trace_func.borrow());
    let profile_is_none = vm.is_none(&vm.profile_func.borrow());
    let tracing = !(trace_is_none && profile_is_none);
    vm.use_tracing.replace(tracing);
}

// TODO implement string interning, this will be key for performance
fn sys_intern(value: PyStringRef, _vm: &VirtualMachine) -> PyStringRef {
    value
}

fn sys_exc_info(vm: &VirtualMachine) -> PyResult {
    Ok(vm.ctx.new_tuple(match vm.current_exception() {
        Some(exception) => vec![
            exception.class().into_object(),
            exception.clone(),
            vm.get_none(),
        ],
        None => vec![vm.get_none(), vm.get_none(), vm.get_none()],
    }))
}

// TODO: raise a SystemExit here
fn sys_exit(code: OptionalArg<i32>, _vm: &VirtualMachine) -> PyResult<()> {
    let code = code.unwrap_or(0);
    std::process::exit(code)
}

#[pystruct_sequence(name = "version_info")]
#[derive(Default, Debug)]
struct VersionInfo {
    major: usize,
    minor: usize,
    micro: usize,
    releaselevel: String,
    serial: usize,
}

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef, builtins: PyObjectRef) {
    let ctx = &vm.ctx;

    let flags_type = SysFlags::make_class(ctx);
    let flags = SysFlags::from_settings(&vm.settings)
        .into_struct_sequence(vm, flags_type)
        .unwrap();

    let version_info_type = VersionInfo::make_class(ctx);
    let version_info = VersionInfo {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap(),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap(),
        micro: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap(),
        releaselevel: "alpha".to_owned(),
        serial: 0,
    }
    .into_struct_sequence(vm, version_info_type)
    .unwrap();

    // TODO Add crate version to this namespace
    let implementation = py_namespace!(vm, {
        "name" => ctx.new_str("RustPython".to_string()),
        "cache_tag" => ctx.new_str("rustpython-01".to_string()),
    });

    let path = ctx.new_list(
        vm.settings
            .path_list
            .iter()
            .map(|path| ctx.new_str(path.clone()))
            .collect(),
    );

    let platform = if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "macos") {
        "darwin".to_string()
    } else if cfg!(target_os = "windows") {
        "win32".to_string()
    } else if cfg!(target_os = "android") {
        // Linux as well. see https://bugs.python.org/issue32637
        "linux".to_string()
    } else {
        "unknown".to_string()
    };

    // https://doc.rust-lang.org/reference/conditional-compilation.html#target_endian
    let bytorder = if cfg!(target_endian = "little") {
        "little".to_string()
    } else if cfg!(target_endian = "big") {
        "big".to_string()
    } else {
        "unknown".to_string()
    };

    let copyright = "Copyright (c) 2019 RustPython Team";

    let sys_doc = "This module provides access to some objects used or maintained by the
interpreter and to functions that interact strongly with the interpreter.

Dynamic objects:

argv -- command line arguments; argv[0] is the script pathname if known
path -- module search path; path[0] is the script directory, else ''
modules -- dictionary of loaded modules

displayhook -- called to show results in an interactive session
excepthook -- called to handle any uncaught exception other than SystemExit
  To customize printing in an interactive session or to install a custom
  top-level exception handler, assign other functions to replace these.

stdin -- standard input file object; used by input()
stdout -- standard output file object; used by print()
stderr -- standard error object; used for error messages
  By assigning other file objects (or objects that behave like files)
  to these, it is possible to redirect all of the interpreter's I/O.

last_type -- type of last uncaught exception
last_value -- value of last uncaught exception
last_traceback -- traceback of last uncaught exception
  These three are only available in an interactive session after a
  traceback has been printed.

Static objects:

builtin_module_names -- tuple of module names built into this interpreter
copyright -- copyright notice pertaining to this interpreter
exec_prefix -- prefix used to find the machine-specific Python library
executable -- absolute path of the executable binary of the Python interpreter
float_info -- a struct sequence with information about the float implementation.
float_repr_style -- string indicating the style of repr() output for floats
hash_info -- a struct sequence with information about the hash algorithm.
hexversion -- version information encoded as a single integer
implementation -- Python implementation information.
int_info -- a struct sequence with information about the int implementation.
maxsize -- the largest supported length of containers.
maxunicode -- the value of the largest Unicode code point
platform -- platform identifier
prefix -- prefix used to find the Python library
thread_info -- a struct sequence with information about the thread implementation.
version -- the version of this interpreter as a string
version_info -- version information as a named tuple
__stdin__ -- the original stdin; don't touch!
__stdout__ -- the original stdout; don't touch!
__stderr__ -- the original stderr; don't touch!
__displayhook__ -- the original displayhook; don't touch!
__excepthook__ -- the original excepthook; don't touch!

Functions:

displayhook() -- print an object to the screen, and save it in builtins._
excepthook() -- print an exception and its traceback to sys.stderr
exc_info() -- return thread-safe information about the current exception
exit() -- exit the interpreter by raising SystemExit
getdlopenflags() -- returns flags to be used for dlopen() calls
getprofile() -- get the global profiling function
getrefcount() -- return the reference count for an object (plus one :-)
getrecursionlimit() -- return the max recursion depth for the interpreter
getsizeof() -- return the size of an object in bytes
gettrace() -- get the global debug tracing function
setcheckinterval() -- control how often the interpreter checks for events
setdlopenflags() -- set the flags to be used for dlopen() calls
setprofile() -- set the global profiling function
setrecursionlimit() -- set the max recursion depth for the interpreter
settrace() -- set the global debug tracing function
";
    let mut module_names: Vec<String> = vm.stdlib_inits.borrow().keys().cloned().collect();
    module_names.push("sys".to_string());
    module_names.push("builtins".to_string());
    module_names.sort();
    let builtin_module_names = ctx.new_tuple(
        module_names
            .iter()
            .map(|v| v.into_pyobject(vm).unwrap())
            .collect(),
    );
    let modules = ctx.new_dict();

    let prefix = option_env!("RUSTPYTHON_PREFIX").unwrap_or("/usr/local");
    let base_prefix = option_env!("RUSTPYTHON_BASEPREFIX").unwrap_or(prefix);
    let exec_prefix = option_env!("RUSTPYTHON_EXECPREFIX").unwrap_or(prefix);

    extend_module!(vm, module, {
      "__name__" => ctx.new_str(String::from("sys")),
      "argv" => argv(vm),
      "builtin_module_names" => builtin_module_names,
      "byteorder" => ctx.new_str(bytorder),
      "copyright" => ctx.new_str(copyright.to_string()),
      "executable" => executable(ctx),
      "flags" => flags,
      "getrefcount" => ctx.new_rustfunc(sys_getrefcount),
      "getsizeof" => ctx.new_rustfunc(sys_getsizeof),
      "implementation" => implementation,
      "getfilesystemencoding" => ctx.new_rustfunc(sys_getfilesystemencoding),
      "getfilesystemencodeerrors" => ctx.new_rustfunc(sys_getfilesystemencodeerrors),
      "getprofile" => ctx.new_rustfunc(sys_getprofile),
      "gettrace" => ctx.new_rustfunc(sys_gettrace),
      "intern" => ctx.new_rustfunc(sys_intern),
      "maxunicode" => ctx.new_int(0x0010_FFFF),
      "maxsize" => ctx.new_int(std::isize::MAX),
      "path" => path,
      "ps1" => ctx.new_str(">>>>> ".to_string()),
      "ps2" => ctx.new_str("..... ".to_string()),
      "__doc__" => ctx.new_str(sys_doc.to_string()),
      "_getframe" => ctx.new_rustfunc(getframe),
      "modules" => modules.clone(),
      "warnoptions" => ctx.new_list(vec![]),
      "platform" => ctx.new_str(platform),
      "meta_path" => ctx.new_list(vec![]),
      "path_hooks" => ctx.new_list(vec![]),
      "path_importer_cache" => ctx.new_dict(),
      "pycache_prefix" => vm.get_none(),
      "dont_write_bytecode" => vm.new_bool(vm.settings.dont_write_bytecode),
      "setprofile" => ctx.new_rustfunc(sys_setprofile),
      "settrace" => ctx.new_rustfunc(sys_settrace),
      "version" => vm.new_str(version::get_version()),
      "version_info" => version_info,
      "exc_info" => ctx.new_rustfunc(sys_exc_info),
      "prefix" => ctx.new_str(prefix.to_string()),
      "base_prefix" => ctx.new_str(base_prefix.to_string()),
      "exec_prefix" => ctx.new_str(exec_prefix.to_string()),
      "exit" => ctx.new_rustfunc(sys_exit),
    });

    modules.set_item("sys", module.clone(), vm).unwrap();
    modules.set_item("builtins", builtins.clone(), vm).unwrap();
}
