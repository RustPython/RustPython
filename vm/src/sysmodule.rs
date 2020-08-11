use num_traits::ToPrimitive;
use std::{env, mem, path};

use crate::frame::FrameRef;
use crate::function::{Args, OptionalArg, PyFuncArgs};
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IntoPyObject, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyResult};
use crate::vm::{PySettings, VirtualMachine};
use crate::{builtins, exceptions, py_io, version};
use rustpython_common::hash::{PyHash, PyUHash};
use rustpython_common::rc::PyRc;

/*
 * The magic sys module.
 */

fn argv(vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.new_list(
        vm.state
            .settings
            .argv
            .iter()
            .map(|arg| vm.ctx.new_str(arg))
            .collect(),
    )
}

fn executable(ctx: &PyContext) -> PyObjectRef {
    if let Some(exec_path) = env::args().next() {
        let path = path::Path::new(&exec_path);
        if !path.exists() {
            return ctx.new_str("");
        }
        if path.is_absolute() {
            return ctx.new_str(exec_path);
        }
        if let Ok(dir) = env::current_dir() {
            if let Ok(dir) = dir.into_os_string().into_string() {
                return ctx.new_str(format!(
                    "{}/{}",
                    dir,
                    exec_path.strip_prefix("./").unwrap_or(&exec_path)
                ));
            }
        }
    }
    ctx.none()
}

fn _base_executable(ctx: &PyContext) -> PyObjectRef {
    if let Ok(var) = env::var("__PYVENV_LAUNCHER__") {
        ctx.new_str(var)
    } else {
        executable(ctx)
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
#[pystruct_sequence(name = "flags", module = "sys")]
#[derive(Default, Debug)]
struct SysFlags {
    /// -d
    debug: u8,
    /// -i
    inspect: u8,
    /// -i
    interactive: u8,
    /// -O or -OO
    optimize: u8,
    /// -B
    dont_write_bytecode: u8,
    /// -s
    no_user_site: u8,
    /// -S
    no_site: u8,
    /// -E
    ignore_environment: u8,
    /// -v
    verbose: u8,
    /// -b
    bytes_warning: u8,
    /// -q
    quiet: u8,
    /// -R
    hash_randomization: u8,
    /// -I
    isolated: u8,
    /// -X dev
    dev_mode: bool,
    /// -X utf8
    utf8_mode: u8,
}

impl SysFlags {
    fn from_settings(settings: &PySettings) -> Self {
        // Start with sensible defaults:
        let mut flags: SysFlags = Default::default();
        flags.debug = settings.debug as u8;
        flags.inspect = settings.inspect as u8;
        flags.optimize = settings.optimize;
        flags.no_user_site = settings.no_user_site as u8;
        flags.no_site = settings.no_site as u8;
        flags.ignore_environment = settings.ignore_environment as u8;
        flags.verbose = settings.verbose;
        flags.quiet = settings.quiet as u8;
        flags.dont_write_bytecode = settings.dont_write_bytecode as u8;
        flags
    }
}

fn sys_getrefcount(obj: PyObjectRef) -> usize {
    PyRc::strong_count(&obj)
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

fn sys_setrecursionlimit(recursion_limit: i32, vm: &VirtualMachine) -> PyResult {
    let recursion_limit = recursion_limit
        .to_usize()
        .filter(|&u| u >= 1)
        .ok_or_else(|| {
            vm.new_value_error("recursion limit must be greater than or equal to one".to_owned())
        })?;
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
    let (ty, val, tb) = match vm.current_exception() {
        Some(exception) => exceptions::split(exception, vm),
        None => (vm.get_none(), vm.get_none(), vm.get_none()),
    };
    vm.ctx.new_tuple(vec![ty, val, tb])
}

fn sys_git_info(vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx.new_tuple(vec![
        vm.ctx.new_str("RustPython"),
        vm.ctx.new_str(version::get_git_identifier()),
        vm.ctx.new_str(version::get_git_revision()),
    ])
}

fn sys_exit(code: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    let code = code.unwrap_or_else(|| vm.get_none());
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

#[pystruct_sequence(module = "sys", name = "getwindowsversion")]
#[derive(Default, Debug)]
#[cfg(windows)]
struct WindowsVersion {
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: String,
    service_pack_major: u16,
    service_pack_minor: u16,
    suite_mask: u16,
    product_type: u8,
    platform_version: (u32, u32, u32),
}

#[cfg(windows)]
fn sys_getwindowsversion(vm: &VirtualMachine) -> PyResult<crate::obj::objtuple::PyTupleRef> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::{
        sysinfoapi::GetVersionExW,
        winnt::{LPOSVERSIONINFOEXW, LPOSVERSIONINFOW, OSVERSIONINFOEXW},
    };

    let mut version = OSVERSIONINFOEXW::default();
    version.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOEXW>() as u32;
    let result = unsafe {
        let osvi = &mut version as LPOSVERSIONINFOEXW as LPOSVERSIONINFOW;
        // SAFE: GetVersionExW accepts a pointer of OSVERSIONINFOW, but winapi crate's type currently doesn't allow to do so.
        // https://docs.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getversionexw#parameters
        GetVersionExW(osvi)
    };

    if result == 0 {
        Err(vm.new_os_error("failed to get windows version".to_owned()))
    } else {
        let service_pack = {
            let (last, _) = version
                .szCSDVersion
                .iter()
                .take_while(|&x| x != &0)
                .enumerate()
                .last()
                .unwrap_or((0, &0));
            let sp = OsString::from_wide(&version.szCSDVersion[..last]);
            if let Ok(string) = sp.into_string() {
                string
            } else {
                return Err(vm.new_os_error("service pack is not ASCII".to_owned()));
            }
        };
        WindowsVersion {
            major: version.dwMajorVersion,
            minor: version.dwMinorVersion,
            build: version.dwBuildNumber,
            platform: version.dwPlatformId,
            service_pack,
            service_pack_major: version.wServicePackMajor,
            service_pack_minor: version.wServicePackMinor,
            suite_mask: version.wSuiteMask,
            product_type: version.wProductType,
            platform_version: (
                version.dwMajorVersion,
                version.dwMinorVersion,
                version.dwBuildNumber,
            ), // TODO Provide accurate version, like CPython impl
        }
        .into_struct_sequence(vm, vm.try_class("sys", "_getwindowsversion_type")?)
    }
}

pub fn get_stdout(vm: &VirtualMachine) -> PyResult {
    vm.get_attribute(vm.sys_module.clone(), "stdout")
        .map_err(|_| vm.new_runtime_error("lost sys.stdout".to_owned()))
}
pub fn get_stderr(vm: &VirtualMachine) -> PyResult {
    vm.get_attribute(vm.sys_module.clone(), "stderr")
        .map_err(|_| vm.new_runtime_error("lost sys.stderr".to_owned()))
}

fn sys_excepthook(
    exc_type: PyObjectRef,
    exc_val: PyObjectRef,
    exc_tb: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let exc = exceptions::normalize(exc_type, exc_val, exc_tb, vm)?;
    let stderr = get_stderr(vm)?;
    exceptions::write_exception(&mut py_io::PyWriter(stderr, vm), vm, &exc)
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

#[pystruct_sequence(module = "sys", name = "hash_info")]
struct PyHashInfo {
    width: usize,
    modulus: PyUHash,
    inf: PyHash,
    nan: PyHash,
    imag: PyHash,
    algorithm: &'static str,
    hash_bits: usize,
    seed_bits: usize,
    cutoff: usize,
}
impl PyHashInfo {
    const INFO: Self = {
        use rustpython_common::hash::*;
        PyHashInfo {
            width: BITS,
            modulus: MODULUS,
            inf: INF,
            nan: NAN,
            imag: IMAG,
            algorithm: ALGO,
            hash_bits: HASH_BITS,
            seed_bits: SEED_BITS,
            cutoff: 0, // no small string optimizations
        }
    };
}

#[pystruct_sequence(module = "sys", name = "float_info")]
struct PyFloatInfo {
    max: f64,
    max_exp: i32,
    max_10_exp: i32,
    min: f64,
    min_exp: i32,
    min_10_exp: i32,
    dig: u32,
    mant_dig: u32,
    epsilon: f64,
    radix: u32,
    rounds: i32,
}
impl PyFloatInfo {
    const INFO: Self = PyFloatInfo {
        max: f64::MAX,
        max_exp: f64::MAX_EXP,
        max_10_exp: f64::MAX_10_EXP,
        min: f64::MIN,
        min_exp: f64::MIN_EXP,
        min_10_exp: f64::MIN_10_EXP,
        dig: f64::DIGITS,
        mant_dig: f64::MANTISSA_DIGITS,
        epsilon: f64::EPSILON,
        radix: f64::RADIX,
        rounds: 1, // FE_TONEAREST
    };
}

#[pystruct_sequence(module = "sys", name = "int_info")]
struct PyIntInfo {
    bits_per_digit: usize,
    sizeof_digit: usize,
}
impl PyIntInfo {
    const INFO: Self = PyIntInfo {
        bits_per_digit: 30, //?
        sizeof_digit: std::mem::size_of::<u32>(),
    };
}

pub fn make_module(vm: &VirtualMachine, module: PyObjectRef, builtins: PyObjectRef) {
    let ctx = &vm.ctx;

    let flags_type = SysFlags::make_class(ctx);
    extend_class!(ctx, flags_type, {
      (slot new) => |_cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine| -> PyResult {
        Err(vm.new_type_error("cannot create 'sys.flags' instances".to_owned()))
      },
    });
    let flags = SysFlags::from_settings(&vm.state.settings)
        .into_struct_sequence(vm, flags_type)
        .unwrap();

    let version_info_type = version::VersionInfo::make_class(ctx);
    extend_class!(ctx, version_info_type, {
      (slot new) => |_cls: PyClassRef, _args: PyFuncArgs, vm: &VirtualMachine| -> PyResult {
        Err(vm.new_type_error("cannot create 'sys.version_info' instances".to_owned()))
      },
    });
    let version_info = version::VersionInfo::VERSION
        .into_struct_sequence(vm, version_info_type)
        .unwrap();

    let hash_info_type = PyHashInfo::make_class(ctx);
    let hash_info = PyHashInfo::INFO
        .into_struct_sequence(vm, hash_info_type)
        .unwrap();

    let float_info_type = PyFloatInfo::make_class(ctx);
    let float_info = PyFloatInfo::INFO
        .into_struct_sequence(vm, float_info_type)
        .unwrap();

    let int_info_type = PyIntInfo::make_class(ctx);
    let int_info = PyIntInfo::INFO
        .into_struct_sequence(vm, int_info_type)
        .unwrap();

    // TODO Add crate version to this namespace
    let implementation = py_namespace!(vm, {
        "name" => ctx.new_str("rustpython"),
        "cache_tag" => ctx.new_str("rustpython-01"),
        "_multiarch" => ctx.new_str(MULTIARCH.to_owned()),
        "version" => version_info.clone(),
        "hexversion" => ctx.new_int(version::VERSION_HEX),
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
_base_executable -- __PYVENV_LAUNCHER__ enviroment variable if defined, else sys.executable.

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
    let builtin_module_names =
        ctx.new_tuple(module_names.iter().map(|v| v.into_pyobject(vm)).collect());
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
      "copyright" => ctx.new_str(copyright),
      "_base_executable" => _base_executable(ctx),
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
      "maxunicode" => ctx.new_int(std::char::MAX as u32),
      "maxsize" => ctx.new_int(std::isize::MAX),
      "path" => path,
      "ps1" => ctx.new_str(">>>>> "),
      "ps2" => ctx.new_str("..... "),
      "__doc__" => ctx.new_str(sys_doc),
      "_getframe" => ctx.new_function(getframe),
      "modules" => modules.clone(),
      "warnoptions" => ctx.new_list(vec![]),
      "platform" => ctx.new_str(PLATFORM.to_owned()),
      "_framework" => ctx.new_str(framework),
      "meta_path" => ctx.new_list(vec![]),
      "path_hooks" => ctx.new_list(vec![]),
      "path_importer_cache" => ctx.new_dict(),
      "pycache_prefix" => vm.get_none(),
      "dont_write_bytecode" => vm.ctx.new_bool(vm.state.settings.dont_write_bytecode),
      "setprofile" => ctx.new_function(sys_setprofile),
      "setrecursionlimit" => ctx.new_function(sys_setrecursionlimit),
      "settrace" => ctx.new_function(sys_settrace),
      "version" => vm.ctx.new_str(version::get_version()),
      "version_info" => version_info,
      "_git" => sys_git_info(vm),
      "exc_info" => ctx.new_function(sys_exc_info),
      "prefix" => ctx.new_str(prefix),
      "base_prefix" => ctx.new_str(base_prefix),
      "exec_prefix" => ctx.new_str(exec_prefix),
      "base_exec_prefix" => ctx.new_str(base_exec_prefix),
      "exit" => ctx.new_function(sys_exit),
      "abiflags" => ctx.new_str(ABIFLAGS.to_owned()),
      "audit" => ctx.new_function(sys_audit),
      "displayhook" => ctx.new_function(sys_displayhook),
      "__displayhook__" => ctx.new_function(sys_displayhook),
      "excepthook" => ctx.new_function(sys_excepthook),
      "__excepthook__" => ctx.new_function(sys_excepthook),
      "hexversion" => ctx.new_int(version::VERSION_HEX),
      "api_version" => ctx.new_int(0x0), // what C api?
      "float_info" => float_info,
      "int_info" => int_info,
      "float_repr_style" => ctx.new_str("short"),
    });

    #[cfg(windows)]
    {
        let getwindowsversion = WindowsVersion::make_class(ctx);
        extend_module!(vm, module, {
            "getwindowsversion" => ctx.new_function(sys_getwindowsversion),
            "_getwindowsversion_type" => getwindowsversion, // XXX: This is not a python spec but required by current RustPython implementation
        })
    }

    modules.set_item("sys", module.clone(), vm).unwrap();
    modules.set_item("builtins", builtins.clone(), vm).unwrap();
}
