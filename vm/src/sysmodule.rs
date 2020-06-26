use std::sync::Arc;
use std::{env, mem};

use crate::builtins;
use crate::frame::FrameRef;
use crate::function::{Args, OptionalArg, PyFuncArgs};
use crate::obj::objstr::PyStringRef;
use crate::pyhash::PyHashInfo;
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
        vm.state
            .settings
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
        return Err(vm.new_value_error("call stack is not deep enough".to_owned()));
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

fn sys_getrefcount(obj: PyObjectRef) -> usize {
    Arc::strong_count(&obj)
}

fn sys_getsizeof(obj: PyObjectRef) -> usize {
    // TODO: implement default optional argument.
    mem::size_of_val(&obj)
}

fn sys_getfilesystemencoding(_vm: &VirtualMachine) -> String {
    // TODO: implement non-utf-8 mode.
    "utf-8".to_owned()
}

fn sys_getdefaultencoding(_vm: &VirtualMachine) -> String {
    "utf-8".to_owned()
}

#[cfg(not(windows))]
fn sys_getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
    "surrogateescape".to_owned()
}

#[cfg(windows)]
fn sys_getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
    "surrogatepass".to_owned()
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
    vm.use_tracing.set(tracing);
}

fn sys_getrecursionlimit(vm: &VirtualMachine) -> usize {
    vm.recursion_limit.get()
}

fn sys_setrecursionlimit(recursion_limit: usize, vm: &VirtualMachine) -> PyResult {
    let recursion_depth = vm.frames.borrow().len();

    if recursion_limit > recursion_depth + 1 {
        vm.recursion_limit.set(recursion_limit);
        Ok(vm.ctx.none())
    } else {
        Err(vm.new_recursion_error(format!(
            "cannot set the recursion limit to {} at the recursion depth {}: the limit is too low",
            recursion_limit, recursion_depth
        )))
    }
}

// TODO implement string interning, this will be key for performance
fn sys_intern(value: PyStringRef) -> PyStringRef {
    value
}

fn sys_exc_info(vm: &VirtualMachine) -> PyObjectRef {
    let exc_info = match vm.current_exception() {
        Some(exception) => vec![
            exception.class().into_object(),
            exception.clone().into_object(),
            exception
                .traceback()
                .map_or(vm.get_none(), |tb| tb.into_object()),
        ],
        None => vec![vm.get_none(), vm.get_none(), vm.get_none()],
    };
    vm.ctx.new_tuple(exc_info)
}

fn sys_git_info(vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.new_tuple(vec![
        vm.ctx.new_str("RustPython".to_owned()),
        vm.ctx.new_str(version::get_git_identifier()),
        vm.ctx.new_str(version::get_git_revision()),
    ])
}

fn sys_exit(code: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    let code = code.unwrap_or_else(|| vm.new_int(0));
    Err(vm.new_exception(vm.ctx.exceptions.system_exit.clone(), vec![code]))
}

fn sys_audit(_args: PyFuncArgs) {
    // TODO: sys.audit implementation
}

fn sys_displayhook(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    // Save non-None values as "_"
    if vm.is_none(&obj) {
        return Ok(());
    }
    // set to none to avoid recursion while printing
    vm.set_attr(&vm.builtins, "_", vm.get_none())?;
    // TODO: catch encoding errors
    let repr = vm.to_repr(&obj)?.into_object();
    builtins::builtin_print(Args::new(vec![repr]), Default::default(), vm)?;
    vm.set_attr(&vm.builtins, "_", obj)?;
    Ok(())
}

const PLATFORM: &str = {
    cfg_if::cfg_if! {
        if #[cfg(any(target_os = "linux", target_os = "android"))] {
            // Android is linux as well. see https://bugs.python.org/issue32637
            "linux"
        } else if #[cfg(target_os = "macos")] {
            "darwin"
        } else if #[cfg(windows)] {
            "win32"
        } else {
            "unknown"
        }
    }
};

const ABIFLAGS: &str = "";

// not the same as CPython (e.g. rust's x86_x64-unknown-linux-gnu is just x86_64-linux-gnu)
// but hopefully that's just an implementation detail? TODO: copy CPython's multiarch exactly,
// https://github.com/python/cpython/blob/3.8/configure.ac#L725
const MULTIARCH: &str = env!("RUSTPYTHON_TARGET_TRIPLE");

pub fn sysconfigdata_name() -> String {
    format!("_sysconfigdata_{}_{}_{}", ABIFLAGS, PLATFORM, MULTIARCH)
}

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef, builtins: PyObjectRef) {
    let ctx = &vm.ctx;

    let flags_type = SysFlags::make_class(ctx);
    let flags = SysFlags::from_settings(&vm.state.settings)
        .into_struct_sequence(vm, flags_type)
        .unwrap();

    let version_info_type = version::VersionInfo::make_class(ctx);
    let version_info = version::get_version_info()
        .into_struct_sequence(vm, version_info_type)
        .unwrap();

    let hash_info_type = PyHashInfo::make_class(ctx);
    let hash_info = PyHashInfo::INFO
        .into_struct_sequence(vm, hash_info_type)
        .unwrap();

    // TODO Add crate version to this namespace
    let implementation = py_namespace!(vm, {
        "name" => ctx.new_str("rustpython".to_owned()),
        "cache_tag" => ctx.new_str("rustpython-01".to_owned()),
        "_multiarch" => ctx.new_str(MULTIARCH.to_owned()),
    });

    let path = ctx.new_list(
        vm.state
            .settings
            .path_list
            .iter()
            .map(|path| ctx.new_str(path.clone()))
            .collect(),
    );

    let framework = "".to_owned();

    // https://doc.rust-lang.org/reference/conditional-compilation.html#target_endian
    let bytorder = if cfg!(target_endian = "little") {
        "little".to_owned()
    } else if cfg!(target_endian = "big") {
        "big".to_owned()
    } else {
        "unknown".to_owned()
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
    let mut module_names: Vec<String> = vm.state.stdlib_inits.keys().cloned().collect();
    module_names.push("sys".to_owned());
    module_names.push("builtins".to_owned());
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
    let base_exec_prefix = option_env!("RUSTPYTHON_BASEEXECPREFIX").unwrap_or(exec_prefix);

    extend_module!(vm, module, {
      "__name__" => ctx.new_str(String::from("sys")),
      "argv" => argv(vm),
      "builtin_module_names" => builtin_module_names,
      "byteorder" => ctx.new_str(bytorder),
      "copyright" => ctx.new_str(copyright.to_owned()),
      "executable" => executable(ctx),
      "flags" => flags,
      "getrefcount" => ctx.new_function(sys_getrefcount),
      "getrecursionlimit" => ctx.new_function(sys_getrecursionlimit),
      "getsizeof" => ctx.new_function(sys_getsizeof),
      "implementation" => implementation,
      "getfilesystemencoding" => ctx.new_function(sys_getfilesystemencoding),
      "getfilesystemencodeerrors" => ctx.new_function(sys_getfilesystemencodeerrors),
      "getdefaultencoding" => ctx.new_function(sys_getdefaultencoding),
      "getprofile" => ctx.new_function(sys_getprofile),
      "gettrace" => ctx.new_function(sys_gettrace),
      "hash_info" => hash_info,
      "intern" => ctx.new_function(sys_intern),
      "maxunicode" => ctx.new_int(0x0010_FFFF),
      "maxsize" => ctx.new_int(std::isize::MAX),
      "path" => path,
      "ps1" => ctx.new_str(">>>>> ".to_owned()),
      "ps2" => ctx.new_str("..... ".to_owned()),
      "__doc__" => ctx.new_str(sys_doc.to_owned()),
      "_getframe" => ctx.new_function(getframe),
      "modules" => modules.clone(),
      "warnoptions" => ctx.new_list(vec![]),
      "platform" => ctx.new_str(PLATFORM.to_owned()),
      "_framework" => ctx.new_str(framework),
      "meta_path" => ctx.new_list(vec![]),
      "path_hooks" => ctx.new_list(vec![]),
      "path_importer_cache" => ctx.new_dict(),
      "pycache_prefix" => vm.get_none(),
      "dont_write_bytecode" => vm.new_bool(vm.state.settings.dont_write_bytecode),
      "setprofile" => ctx.new_function(sys_setprofile),
      "setrecursionlimit" => ctx.new_function(sys_setrecursionlimit),
      "settrace" => ctx.new_function(sys_settrace),
      "version" => vm.new_str(version::get_version()),
      "version_info" => version_info,
      "_git" => sys_git_info(vm),
      "exc_info" => ctx.new_function(sys_exc_info),
      "prefix" => ctx.new_str(prefix.to_owned()),
      "base_prefix" => ctx.new_str(base_prefix.to_owned()),
      "exec_prefix" => ctx.new_str(exec_prefix.to_owned()),
      "base_exec_prefix" => ctx.new_str(base_exec_prefix.to_owned()),
      "exit" => ctx.new_function(sys_exit),
      "abiflags" => ctx.new_str(ABIFLAGS.to_owned()),
      "audit" => ctx.new_function(sys_audit),
      "displayhook" => ctx.new_function(sys_displayhook),
      "__displayhook__" => ctx.new_function(sys_displayhook),
    });

    modules.set_item("sys", module.clone(), vm).unwrap();
    modules.set_item("builtins", builtins.clone(), vm).unwrap();
}
