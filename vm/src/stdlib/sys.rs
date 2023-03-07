use crate::{convert::ToPyObject, PyObject, PyResult, VirtualMachine};

pub(crate) use sys::{UnraisableHookArgs, MAXSIZE, MULTIARCH};

#[pymodule]
mod sys {
    use crate::{
        builtins::{PyDictRef, PyNamespace, PyStr, PyStrRef, PyTupleRef, PyTypeRef},
        common::{
            ascii,
            hash::{PyHash, PyUHash},
        },
        frame::FrameRef,
        function::{FuncArgs, OptionalArg, PosArgs},
        stdlib::builtins,
        stdlib::warnings::warn,
        types::PyStructSequence,
        version,
        vm::{Settings, VirtualMachine},
        AsObject, PyObject, PyObjectRef, PyRef, PyRefExact, PyResult,
    };
    use num_traits::ToPrimitive;
    use std::{
        env::{self, VarError},
        path,
        sync::atomic::Ordering,
    };

    // not the same as CPython (e.g. rust's x86_x64-unknown-linux-gnu is just x86_64-linux-gnu)
    // but hopefully that's just an implementation detail? TODO: copy CPython's multiarch exactly,
    // https://github.com/python/cpython/blob/3.8/configure.ac#L725
    pub(crate) const MULTIARCH: &str = env!("RUSTPYTHON_TARGET_TRIPLE");

    #[pyattr(name = "_rustpython_debugbuild")]
    const RUSTPYTHON_DEBUGBUILD: bool = cfg!(debug_assertions);

    #[pyattr(name = "abiflags")]
    pub(crate) const ABIFLAGS: &str = "";
    #[pyattr(name = "api_version")]
    const API_VERSION: u32 = 0x0; // what C api?
    #[pyattr(name = "copyright")]
    const COPYRIGHT: &str = "Copyright (c) 2019 RustPython Team";
    #[pyattr(name = "float_repr_style")]
    const FLOAT_REPR_STYLE: &str = "short";
    #[pyattr(name = "_framework")]
    const FRAMEWORK: &str = "";
    #[pyattr(name = "hexversion")]
    const HEXVERSION: usize = version::VERSION_HEX;
    #[pyattr(name = "maxsize")]
    pub(crate) const MAXSIZE: isize = isize::MAX;
    #[pyattr(name = "maxunicode")]
    const MAXUNICODE: u32 = std::char::MAX as u32;
    #[pyattr(name = "platform")]
    pub(crate) const PLATFORM: &str = {
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "android"))] {
                // Android is linux as well. see https://bugs.python.org/issue32637
                "linux"
            } else if #[cfg(target_os = "macos")] {
                "darwin"
            } else if #[cfg(windows)] {
                "win32"
            } else if #[cfg(target_os = "wasi")] {
                "wasi"
            } else {
                "unknown"
            }
        }
    };
    #[pyattr(name = "ps1")]
    const PS1: &str = ">>>>> ";
    #[pyattr(name = "ps2")]
    const PS2: &str = "..... ";
    #[cfg(windows)]
    #[pyattr(name = "_vpath")]
    const VPATH: Option<&'static str> = None; // TODO: actual VPATH value

    #[pyattr]
    fn default_prefix(_vm: &VirtualMachine) -> &'static str {
        // TODO: the windows one doesn't really make sense
        if cfg!(windows) {
            "C:"
        } else {
            "/usr/local"
        }
    }
    #[pyattr]
    fn prefix(vm: &VirtualMachine) -> &'static str {
        option_env!("RUSTPYTHON_PREFIX").unwrap_or_else(|| default_prefix(vm))
    }
    #[pyattr]
    fn base_prefix(vm: &VirtualMachine) -> &'static str {
        option_env!("RUSTPYTHON_BASEPREFIX").unwrap_or_else(|| prefix(vm))
    }
    #[pyattr]
    fn exec_prefix(vm: &VirtualMachine) -> &'static str {
        option_env!("RUSTPYTHON_BASEPREFIX").unwrap_or_else(|| prefix(vm))
    }
    #[pyattr]
    fn base_exec_prefix(vm: &VirtualMachine) -> &'static str {
        option_env!("RUSTPYTHON_BASEPREFIX").unwrap_or_else(|| exec_prefix(vm))
    }
    #[pyattr]
    fn platlibdir(_vm: &VirtualMachine) -> &'static str {
        option_env!("RUSTPYTHON_PLATLIBDIR").unwrap_or("lib")
    }

    // alphabetical order with segments of pyattr and others

    #[pyattr]
    fn argv(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vm.state
            .settings
            .argv
            .iter()
            .map(|arg| vm.ctx.new_str(arg.clone()).into())
            .collect()
    }

    #[pyattr]
    fn builtin_module_names(vm: &VirtualMachine) -> PyTupleRef {
        let mut module_names: Vec<_> = vm.state.module_inits.keys().cloned().collect();
        module_names.push("sys".into());
        module_names.push("builtins".into());
        module_names.sort();
        vm.ctx.new_tuple(
            module_names
                .into_iter()
                .map(|n| vm.ctx.new_str(n).into())
                .collect(),
        )
    }

    #[pyattr]
    fn byteorder(vm: &VirtualMachine) -> PyStrRef {
        // https://doc.rust-lang.org/reference/conditional-compilation.html#target_endian
        vm.ctx
            .intern_str(if cfg!(target_endian = "little") {
                "little"
            } else if cfg!(target_endian = "big") {
                "big"
            } else {
                "unknown"
            })
            .to_owned()
    }

    #[pyattr]
    fn _base_executable(vm: &VirtualMachine) -> PyObjectRef {
        let ctx = &vm.ctx;
        if let Ok(var) = env::var("__PYVENV_LAUNCHER__") {
            ctx.new_str(var).into()
        } else {
            executable(vm)
        }
    }

    #[pyattr]
    fn dont_write_bytecode(vm: &VirtualMachine) -> bool {
        vm.state.settings.dont_write_bytecode
    }

    #[pyattr]
    fn executable(vm: &VirtualMachine) -> PyObjectRef {
        let ctx = &vm.ctx;
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(exec_path) = env::args_os().next() {
                if let Ok(path) = which::which(exec_path) {
                    return ctx
                        .new_str(
                            path.into_os_string()
                                .into_string()
                                .unwrap_or_else(|p| p.to_string_lossy().into_owned()),
                        )
                        .into();
                }
            }
        }
        if let Some(exec_path) = env::args().next() {
            let path = path::Path::new(&exec_path);
            if !path.exists() {
                return ctx.new_str(ascii!("")).into();
            }
            if path.is_absolute() {
                return ctx.new_str(exec_path).into();
            }
            if let Ok(dir) = env::current_dir() {
                if let Ok(dir) = dir.into_os_string().into_string() {
                    return ctx
                        .new_str(format!(
                            "{}/{}",
                            dir,
                            exec_path.strip_prefix("./").unwrap_or(&exec_path)
                        ))
                        .into();
                }
            }
        }
        ctx.none()
    }

    #[pyattr]
    fn _git(vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((
            ascii!("RustPython"),
            version::get_git_identifier(),
            version::get_git_revision(),
        ))
    }

    #[pyattr]
    fn implementation(vm: &VirtualMachine) -> PyRef<PyNamespace> {
        // TODO: Add crate version to this namespace
        let ctx = &vm.ctx;
        py_namespace!(vm, {
            "name" => ctx.new_str(ascii!("rustpython")),
            "cache_tag" => ctx.new_str(ascii!("rustpython-01")),
            "_multiarch" => ctx.new_str(MULTIARCH.to_owned()),
            "version" => version_info(vm),
            "hexversion" => ctx.new_int(version::VERSION_HEX),
        })
    }

    #[pyattr]
    fn meta_path(_vm: &VirtualMachine) -> Vec<PyObjectRef> {
        Vec::new()
    }

    #[pyattr]
    fn orig_argv(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        env::args().map(|arg| vm.ctx.new_str(arg).into()).collect()
    }

    #[pyattr]
    fn path(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vm.state
            .settings
            .path_list
            .iter()
            .map(|path| vm.ctx.new_str(path.clone()).into())
            .collect()
    }

    #[pyattr]
    fn path_hooks(_vm: &VirtualMachine) -> Vec<PyObjectRef> {
        Vec::new()
    }

    #[pyattr]
    fn path_importer_cache(vm: &VirtualMachine) -> PyDictRef {
        vm.ctx.new_dict()
    }

    #[pyattr]
    fn pycache_prefix(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }

    #[pyattr]
    fn version(_vm: &VirtualMachine) -> String {
        version::get_version()
    }

    #[cfg(windows)]
    #[pyattr]
    fn winver(_vm: &VirtualMachine) -> String {
        // Note: This is Python DLL version in CPython, but we arbitrary fill it for compatibility
        version::get_winver_number()
    }

    #[pyattr]
    fn _xoptions(vm: &VirtualMachine) -> PyDictRef {
        let ctx = &vm.ctx;
        let xopts = ctx.new_dict();
        for (key, value) in &vm.state.settings.xopts {
            let value = value.as_ref().map_or_else(
                || ctx.new_bool(true).into(),
                |s| ctx.new_str(s.clone()).into(),
            );
            xopts.set_item(&**key, value, vm).unwrap();
        }
        xopts
    }

    #[pyattr]
    fn warnoptions(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vm.state
            .settings
            .warnopts
            .iter()
            .map(|s| vm.ctx.new_str(s.clone()).into())
            .collect()
    }

    #[pyfunction]
    fn audit(_args: FuncArgs) {
        // TODO: sys.audit implementation
    }

    #[pyfunction]
    fn exit(code: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let code = code.unwrap_or_none(vm);
        Err(vm.new_exception(vm.ctx.exceptions.system_exit.to_owned(), vec![code]))
    }

    #[pyfunction(name = "__displayhook__")]
    #[pyfunction]
    fn displayhook(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Save non-None values as "_"
        if vm.is_none(&obj) {
            return Ok(());
        }
        // set to none to avoid recursion while printing
        vm.builtins.set_attr("_", vm.ctx.none(), vm)?;
        // TODO: catch encoding errors
        let repr = obj.repr(vm)?.into();
        builtins::print(PosArgs::new(vec![repr]), Default::default(), vm)?;
        vm.builtins.set_attr("_", obj, vm)?;
        Ok(())
    }

    #[pyfunction(name = "__excepthook__")]
    #[pyfunction]
    fn excepthook(
        exc_type: PyObjectRef,
        exc_val: PyObjectRef,
        exc_tb: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let exc = vm.normalize_exception(exc_type, exc_val, exc_tb)?;
        let stderr = super::get_stderr(vm)?;
        vm.write_exception(&mut crate::py_io::PyWriter(stderr, vm), &exc)
    }

    #[pyfunction(name = "__breakpointhook__")]
    #[pyfunction]
    pub fn breakpointhook(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let envar = std::env::var("PYTHONBREAKPOINT")
            .and_then(|envar| {
                if envar.is_empty() {
                    Err(VarError::NotPresent)
                } else {
                    Ok(envar)
                }
            })
            .unwrap_or_else(|_| "pdb.set_trace".to_owned());

        if envar.eq("0") {
            return Ok(vm.ctx.none());
        };

        let print_unimportable_module_warn = || {
            warn(
                vm.ctx.exceptions.runtime_warning,
                format!(
                    "Ignoring unimportable $PYTHONBREAKPOINT: \"{}\"",
                    envar.to_owned(),
                ),
                0,
                vm,
            )
            .unwrap();
            Ok(vm.ctx.none())
        };

        let last = match envar.split('.').last() {
            Some(last) => last,
            None => {
                return print_unimportable_module_warn();
            }
        };

        let (modulepath, attrname) = if last.eq(&envar) {
            ("builtins".to_owned(), envar.to_owned())
        } else {
            (
                envar[..(envar.len() - last.len() - 1)].to_owned(),
                last.to_owned(),
            )
        };

        let module = match vm.import(modulepath, None, 0) {
            Ok(module) => module,
            Err(_) => {
                return print_unimportable_module_warn();
            }
        };

        match vm.get_attribute_opt(module, attrname) {
            Ok(Some(hook)) => hook.as_ref().call(args, vm),
            _ => print_unimportable_module_warn(),
        }
    }

    #[pyfunction]
    fn exc_info(vm: &VirtualMachine) -> (PyObjectRef, PyObjectRef, PyObjectRef) {
        match vm.topmost_exception() {
            Some(exception) => vm.split_exception(exception),
            None => (vm.ctx.none(), vm.ctx.none(), vm.ctx.none()),
        }
    }

    #[pyattr]
    fn flags(vm: &VirtualMachine) -> PyTupleRef {
        Flags::from_settings(&vm.state.settings).into_struct_sequence(vm)
    }

    #[pyattr]
    fn float_info(vm: &VirtualMachine) -> PyTupleRef {
        PyFloatInfo::INFO.into_struct_sequence(vm)
    }

    #[pyfunction]
    fn getdefaultencoding() -> &'static str {
        crate::codecs::DEFAULT_ENCODING
    }

    #[pyfunction]
    fn getrefcount(obj: PyObjectRef) -> usize {
        obj.strong_count()
    }

    #[pyfunction]
    fn getrecursionlimit(vm: &VirtualMachine) -> usize {
        vm.recursion_limit.get()
    }

    #[derive(FromArgs)]
    struct GetsizeofArgs {
        obj: PyObjectRef,
        #[pyarg(any, optional)]
        default: Option<PyObjectRef>,
    }

    #[pyfunction]
    fn getsizeof(args: GetsizeofArgs, vm: &VirtualMachine) -> PyResult {
        let sizeof = || -> PyResult<usize> {
            let res = vm.call_special_method(args.obj, identifier!(vm, __sizeof__), ())?;
            let res = res.try_index(vm)?.try_to_primitive::<usize>(vm)?;
            Ok(res + std::mem::size_of::<PyObject>())
        };
        sizeof()
            .map(|x| vm.ctx.new_int(x).into())
            .or_else(|err| args.default.ok_or(err))
    }

    #[pyfunction]
    fn getfilesystemencoding(_vm: &VirtualMachine) -> String {
        // TODO: implement non-utf-8 mode.
        "utf-8".to_owned()
    }

    #[cfg(not(windows))]
    #[pyfunction]
    fn getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
        "surrogateescape".to_owned()
    }

    #[cfg(windows)]
    #[pyfunction]
    fn getfilesystemencodeerrors(_vm: &VirtualMachine) -> String {
        "surrogatepass".to_owned()
    }

    #[pyfunction]
    fn getprofile(vm: &VirtualMachine) -> PyObjectRef {
        vm.profile_func.borrow().clone()
    }

    #[pyfunction]
    fn _getframe(offset: OptionalArg<usize>, vm: &VirtualMachine) -> PyResult<FrameRef> {
        let offset = offset.into_option().unwrap_or(0);
        if offset > vm.frames.borrow().len() - 1 {
            return Err(vm.new_value_error("call stack is not deep enough".to_owned()));
        }
        let idx = vm.frames.borrow().len() - offset - 1;
        let frame = &vm.frames.borrow()[idx];
        Ok(frame.clone())
    }

    #[pyfunction]
    fn gettrace(vm: &VirtualMachine) -> PyObjectRef {
        vm.trace_func.borrow().clone()
    }

    #[cfg(windows)]
    #[pyfunction]
    fn getwindowsversion(vm: &VirtualMachine) -> PyResult<crate::builtins::tuple::PyTupleRef> {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        use winapi::um::{
            sysinfoapi::GetVersionExW,
            winnt::{LPOSVERSIONINFOEXW, LPOSVERSIONINFOW, OSVERSIONINFOEXW},
        };

        let mut version = OSVERSIONINFOEXW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOEXW>() as u32,
            ..OSVERSIONINFOEXW::default()
        };
        let result = unsafe {
            let osvi = &mut version as LPOSVERSIONINFOEXW as LPOSVERSIONINFOW;
            // SAFETY: GetVersionExW accepts a pointer of OSVERSIONINFOW, but winapi crate's type currently doesn't allow to do so.
            // https://docs.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getversionexw#parameters
            GetVersionExW(osvi)
        };

        if result == 0 {
            return Err(vm.new_os_error("failed to get windows version".to_owned()));
        }

        let service_pack = {
            let (last, _) = version
                .szCSDVersion
                .iter()
                .take_while(|&x| x != &0)
                .enumerate()
                .last()
                .unwrap_or((0, &0));
            let sp = OsString::from_wide(&version.szCSDVersion[..last]);
            sp.into_string()
                .map_err(|_| vm.new_os_error("service pack is not ASCII".to_owned()))?
        };
        Ok(WindowsVersion {
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
        .into_struct_sequence(vm))
    }

    fn _unraisablehook(unraisable: UnraisableHookArgs, vm: &VirtualMachine) -> PyResult<()> {
        use super::PyStderr;

        let stderr = PyStderr(vm);
        if !vm.is_none(&unraisable.object) {
            if !vm.is_none(&unraisable.err_msg) {
                write!(stderr, "{}: ", unraisable.err_msg.str(vm)?);
            } else {
                write!(stderr, "Exception ignored in: ");
            }
            // exception in del will be ignored but printed
            let repr = &unraisable.object.repr(vm);
            let str = match repr {
                Ok(v) => v.to_string(),
                Err(_) => format!(
                    "<object {} repr() failed>",
                    unraisable.object.class().name()
                ),
            };
            writeln!(stderr, "{str}");
        } else if !vm.is_none(&unraisable.err_msg) {
            writeln!(stderr, "{}:", unraisable.err_msg.str(vm)?);
        }

        // TODO: print received unraisable.exc_traceback
        let tb_module = vm.import("traceback", None, 0)?;
        let print_stack = tb_module.get_attr("print_stack", vm)?;
        print_stack.call((), vm)?;

        if vm.is_none(unraisable.exc_type.as_object()) {
            // TODO: early return, but with what error?
        }
        assert!(unraisable
            .exc_type
            .fast_issubclass(vm.ctx.exceptions.base_exception_type));

        // TODO: print module name and qualname

        if !vm.is_none(&unraisable.exc_value) {
            write!(stderr, "{}: ", unraisable.exc_type);
            if let Ok(str) = unraisable.exc_value.str(vm) {
                write!(stderr, "{}", str.as_str());
            } else {
                write!(stderr, "<exception str() failed>");
            }
        }
        writeln!(stderr);
        // TODO: call file.flush()

        Ok(())
    }

    #[pyattr]
    #[pyfunction(name = "__unraisablehook__")]
    fn unraisablehook(unraisable: UnraisableHookArgs, vm: &VirtualMachine) {
        if let Err(e) = _unraisablehook(unraisable, vm) {
            let stderr = super::PyStderr(vm);
            writeln!(stderr, "{}", e.as_object().repr(vm).unwrap().as_str());
        }
    }

    #[pyattr]
    fn hash_info(vm: &VirtualMachine) -> PyTupleRef {
        PyHashInfo::INFO.into_struct_sequence(vm)
    }

    #[pyfunction]
    fn intern(s: PyRefExact<PyStr>, vm: &VirtualMachine) -> PyRef<PyStr> {
        vm.ctx.intern_str(s).to_owned()
    }

    #[pyattr]
    fn int_info(vm: &VirtualMachine) -> PyTupleRef {
        PyIntInfo::INFO.into_struct_sequence(vm)
    }

    #[pyfunction]
    fn is_finalizing(vm: &VirtualMachine) -> bool {
        vm.state.finalizing.load(Ordering::Acquire)
    }

    #[pyfunction]
    fn setprofile(profilefunc: PyObjectRef, vm: &VirtualMachine) {
        vm.profile_func.replace(profilefunc);
        update_use_tracing(vm);
    }

    #[pyfunction]
    fn setrecursionlimit(recursion_limit: i32, vm: &VirtualMachine) -> PyResult<()> {
        let recursion_limit = recursion_limit
            .to_usize()
            .filter(|&u| u >= 1)
            .ok_or_else(|| {
                vm.new_value_error(
                    "recursion limit must be greater than or equal to one".to_owned(),
                )
            })?;
        let recursion_depth = vm.current_recursion_depth();

        if recursion_limit > recursion_depth {
            vm.recursion_limit.set(recursion_limit);
            Ok(())
        } else {
            Err(vm.new_recursion_error(format!(
                "cannot set the recursion limit to {recursion_limit} at the recursion depth {recursion_depth}: the limit is too low"
            )))
        }
    }

    #[pyfunction]
    fn settrace(tracefunc: PyObjectRef, vm: &VirtualMachine) {
        vm.trace_func.replace(tracefunc);
        update_use_tracing(vm);
    }

    #[cfg(feature = "threading")]
    #[pyattr]
    fn thread_info(vm: &VirtualMachine) -> PyTupleRef {
        PyThreadInfo::INFO.into_struct_sequence(vm)
    }

    #[pyattr]
    fn version_info(vm: &VirtualMachine) -> PyTupleRef {
        VersionInfo::VERSION.into_struct_sequence(vm)
    }

    fn update_use_tracing(vm: &VirtualMachine) {
        let trace_is_none = vm.is_none(&vm.trace_func.borrow());
        let profile_is_none = vm.is_none(&vm.profile_func.borrow());
        let tracing = !(trace_is_none && profile_is_none);
        vm.use_tracing.set(tracing);
    }

    /// sys.flags
    ///
    /// Flags provided through command line arguments or environment vars.
    #[pyclass(no_attr, name = "flags", module = "sys")]
    #[derive(Debug, PyStructSequence)]
    pub(super) struct Flags {
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
        bytes_warning: u64,
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
        /// -X int_max_str_digits=number
        int_max_str_digits: i8,
        /// -P, `PYTHONSAFEPATH`
        safe_path: bool,
        /// -X warn_default_encoding, PYTHONWARNDEFAULTENCODING
        warn_default_encoding: u8,
    }

    #[pyclass(with(PyStructSequence))]
    impl Flags {
        fn from_settings(settings: &Settings) -> Self {
            Self {
                debug: settings.debug as u8,
                inspect: settings.inspect as u8,
                interactive: settings.interactive as u8,
                optimize: settings.optimize,
                dont_write_bytecode: settings.dont_write_bytecode as u8,
                no_user_site: settings.no_user_site as u8,
                no_site: settings.no_site as u8,
                ignore_environment: settings.ignore_environment as u8,
                verbose: settings.verbose,
                bytes_warning: settings.bytes_warning,
                quiet: settings.quiet as u8,
                hash_randomization: settings.hash_seed.is_none() as u8,
                isolated: settings.isolated as u8,
                dev_mode: settings.dev_mode,
                utf8_mode: 1,
                int_max_str_digits: -1,
                safe_path: false,
                warn_default_encoding: settings.warn_default_encoding as u8,
            }
        }

        #[pyslot]
        fn slot_new(_cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("cannot create 'sys.flags' instances".to_owned()))
        }
    }

    #[cfg(feature = "threading")]
    #[pyclass(no_attr, name = "thread_info")]
    #[derive(PyStructSequence)]
    pub(super) struct PyThreadInfo {
        name: Option<&'static str>,
        lock: Option<&'static str>,
        version: Option<&'static str>,
    }

    #[cfg(feature = "threading")]
    #[pyclass(with(PyStructSequence))]
    impl PyThreadInfo {
        const INFO: Self = PyThreadInfo {
            name: crate::stdlib::thread::_thread::PYTHREAD_NAME,
            /// As I know, there's only way to use lock as "Mutex" in Rust
            /// with satisfying python document spec.
            lock: Some("mutex+cond"),
            version: None,
        };
    }

    #[pyclass(no_attr, name = "float_info")]
    #[derive(PyStructSequence)]
    pub(super) struct PyFloatInfo {
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

    #[pyclass(with(PyStructSequence))]
    impl PyFloatInfo {
        const INFO: Self = PyFloatInfo {
            max: f64::MAX,
            max_exp: f64::MAX_EXP,
            max_10_exp: f64::MAX_10_EXP,
            min: f64::MIN_POSITIVE,
            min_exp: f64::MIN_EXP,
            min_10_exp: f64::MIN_10_EXP,
            dig: f64::DIGITS,
            mant_dig: f64::MANTISSA_DIGITS,
            epsilon: f64::EPSILON,
            radix: f64::RADIX,
            rounds: 1, // FE_TONEAREST
        };
    }

    #[pyclass(no_attr, name = "hash_info")]
    #[derive(PyStructSequence)]
    pub(super) struct PyHashInfo {
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

    #[pyclass(with(PyStructSequence))]
    impl PyHashInfo {
        const INFO: Self = {
            use rustpython_common::hash::*;
            PyHashInfo {
                width: std::mem::size_of::<PyHash>() * 8,
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

    #[pyclass(no_attr, name = "int_info")]
    #[derive(PyStructSequence)]
    pub(super) struct PyIntInfo {
        bits_per_digit: usize,
        sizeof_digit: usize,
        default_max_str_digits: usize,
        str_digits_check_threshold: usize,
    }

    #[pyclass(with(PyStructSequence))]
    impl PyIntInfo {
        const INFO: Self = PyIntInfo {
            bits_per_digit: 30, //?
            sizeof_digit: std::mem::size_of::<u32>(),
            default_max_str_digits: 4300,
            str_digits_check_threshold: 640,
        };
    }

    #[pyclass(no_attr, name = "version_info")]
    #[derive(Default, Debug, PyStructSequence)]
    pub struct VersionInfo {
        major: usize,
        minor: usize,
        micro: usize,
        releaselevel: &'static str,
        serial: usize,
    }

    #[pyclass(with(PyStructSequence))]
    impl VersionInfo {
        pub const VERSION: VersionInfo = VersionInfo {
            major: version::MAJOR,
            minor: version::MINOR,
            micro: version::MICRO,
            releaselevel: version::RELEASELEVEL,
            serial: version::SERIAL,
        };
        #[pyslot]
        fn slot_new(
            _cls: crate::builtins::type_::PyTypeRef,
            _args: crate::function::FuncArgs,
            vm: &crate::VirtualMachine,
        ) -> crate::PyResult {
            Err(vm.new_type_error("cannot create 'sys.version_info' instances".to_owned()))
        }
    }

    #[cfg(windows)]
    #[pyclass(no_attr, name = "getwindowsversion")]
    #[derive(Default, Debug, PyStructSequence)]
    pub(super) struct WindowsVersion {
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
    #[pyclass(with(PyStructSequence))]
    impl WindowsVersion {}

    #[pyclass(no_attr, name = "UnraisableHookArgs")]
    #[derive(Debug, PyStructSequence, TryIntoPyStructSequence)]
    pub struct UnraisableHookArgs {
        pub exc_type: PyTypeRef,
        pub exc_value: PyObjectRef,
        pub exc_traceback: PyObjectRef,
        pub err_msg: PyObjectRef,
        pub object: PyObjectRef,
    }

    #[pyclass(with(PyStructSequence))]
    impl UnraisableHookArgs {}
}

pub(crate) fn init_module(vm: &VirtualMachine, module: &PyObject, builtins: &PyObject) {
    sys::extend_module(vm, module);

    let modules = vm.ctx.new_dict();
    modules.set_item("sys", module.to_owned(), vm).unwrap();
    modules
        .set_item("builtins", builtins.to_owned(), vm)
        .unwrap();
    extend_module!(vm, module, {
        "__doc__" => sys::DOC.to_owned().to_pyobject(vm),
        "modules" => modules,
    });
}

/// Similar to PySys_WriteStderr in CPython.
///
/// # Usage
///
/// ```rust,ignore
/// writeln!(sys::PyStderr(vm), "foo bar baz :)");
/// ```
///
/// Unlike writing to a `std::io::Write` with the `write[ln]!()` macro, there's no error condition here;
/// this is intended to be a replacement for the `eprint[ln]!()` macro, so `write!()`-ing to PyStderr just
/// returns `()`.
pub struct PyStderr<'vm>(pub &'vm VirtualMachine);

impl PyStderr<'_> {
    pub fn write_fmt(&self, args: std::fmt::Arguments<'_>) {
        use crate::py_io::Write;

        let vm = self.0;
        if let Ok(stderr) = get_stderr(vm) {
            let mut stderr = crate::py_io::PyWriter(stderr, vm);
            if let Ok(()) = stderr.write_fmt(args) {
                return;
            }
        }
        eprint!("{args}")
    }
}

pub fn get_stdin(vm: &VirtualMachine) -> PyResult {
    vm.sys_module
        .clone()
        .get_attr("stdin", vm)
        .map_err(|_| vm.new_runtime_error("lost sys.stdin".to_owned()))
}
pub fn get_stdout(vm: &VirtualMachine) -> PyResult {
    vm.sys_module
        .clone()
        .get_attr("stdout", vm)
        .map_err(|_| vm.new_runtime_error("lost sys.stdout".to_owned()))
}
pub fn get_stderr(vm: &VirtualMachine) -> PyResult {
    vm.sys_module
        .clone()
        .get_attr("stderr", vm)
        .map_err(|_| vm.new_runtime_error("lost sys.stderr".to_owned()))
}

pub(crate) fn sysconfigdata_name() -> String {
    format!(
        "_sysconfigdata_{}_{}_{}",
        sys::ABIFLAGS,
        sys::PLATFORM,
        sys::MULTIARCH
    )
}
