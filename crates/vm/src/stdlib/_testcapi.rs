pub(crate) use _testcapi::module_def;

#[pymodule]
mod _testcapi {
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{
            PyBaseException, PyCode, PyInt, PyModule, PyTupleRef, PyType, PyTypeRef, PyUtf8StrRef,
        },
        common::str::wchar_t,
        function::{ArgBytesLike, ItemDoc, KwArgs, PosArgs, PyMethodDef, PyMethodFlags},
        types::{Constructor, DefaultConstructor},
    };

    #[pyattr]
    const CHAR_MAX: core::ffi::c_char = core::ffi::c_char::MAX;
    #[pyattr]
    const CHAR_MIN: core::ffi::c_char = core::ffi::c_char::MIN;
    #[pyattr]
    const UCHAR_MAX: u8 = u8::MAX;
    #[pyattr]
    const SHRT_MAX: i16 = i16::MAX;
    #[pyattr]
    const SHRT_MIN: i16 = i16::MIN;
    #[pyattr]
    const USHRT_MAX: u16 = u16::MAX;
    #[pyattr]
    const INT_MAX: core::ffi::c_int = core::ffi::c_int::MAX;
    #[pyattr]
    const INT_MIN: core::ffi::c_int = core::ffi::c_int::MIN;
    #[pyattr]
    const UINT_MAX: core::ffi::c_uint = core::ffi::c_uint::MAX;
    #[pyattr]
    const LONG_MAX: core::ffi::c_long = core::ffi::c_long::MAX;
    #[pyattr]
    const LONG_MIN: core::ffi::c_long = core::ffi::c_long::MIN;
    #[pyattr]
    const ULONG_MAX: core::ffi::c_ulong = core::ffi::c_ulong::MAX;
    #[pyattr]
    const FLT_MAX: f64 = f32::MAX as f64;
    #[pyattr]
    const FLT_MIN: f64 = f32::MIN_POSITIVE as f64;
    #[pyattr]
    const DBL_MAX: f64 = f64::MAX;
    #[pyattr]
    const DBL_MIN: f64 = f64::MIN_POSITIVE;
    #[pyattr]
    const LLONG_MAX: i64 = i64::MAX;
    #[pyattr]
    const LLONG_MIN: i64 = i64::MIN;
    #[pyattr]
    const ULLONG_MAX: u64 = u64::MAX;
    #[pyattr]
    const PY_SSIZE_T_MAX: isize = isize::MAX;
    #[pyattr]
    const PY_SSIZE_T_MIN: isize = isize::MIN;
    #[pyattr]
    const SIZE_MAX: usize = usize::MAX;
    #[pyattr]
    const SIZEOF_WCHAR_T: usize = core::mem::size_of::<wchar_t>();
    #[pyattr]
    const SIZEOF_VOID_P: usize = core::mem::size_of::<*const ()>();
    #[cfg(any(unix, windows))]
    #[pyattr]
    const SIZEOF_TIME_T: usize = core::mem::size_of::<libc::time_t>();
    #[cfg(unix)]
    #[pyattr]
    const SIZEOF_PID_T: usize = core::mem::size_of::<libc::pid_t>();
    #[cfg(windows)]
    #[pyattr]
    const SIZEOF_PID_T: usize = core::mem::size_of::<core::ffi::c_int>();
    #[pyattr(name = "Py_Version")]
    const PY_VERSION: usize = crate::version::VERSION_HEX;
    #[pyattr(name = "the_number_three")]
    const THE_NUMBER_THREE: i32 = 3;
    #[pyattr]
    const INT32_MIN: i32 = i32::MIN;
    #[pyattr]
    const INT32_MAX: i32 = i32::MAX;
    #[pyattr]
    const UINT32_MAX: u32 = u32::MAX;
    #[pyattr]
    const INT64_MIN: i64 = i64::MIN;
    #[pyattr]
    const INT64_MAX: i64 = i64::MAX;
    #[pyattr]
    const UINT64_MAX: u64 = u64::MAX;

    // MIPS without NaN2008 and PA-RISC use the most significant mantissa
    // bit as the signaling bit.
    #[pyattr(name = "nan_msb_is_signaling")]
    const NAN_MSB_IS_SIGNALING: bool = cfg!(any(target_arch = "mips", target_arch = "mips64"));

    /// PyConfig_Get
    fn config_get_value(name: &str, vm: &VirtualMachine) -> PyResult {
        let settings = &vm.state.config.settings;
        let ctx = &vm.ctx;
        let sys_attr = match name {
            "argv" => Some("argv"),
            "base_exec_prefix" => Some("base_exec_prefix"),
            "base_executable" => Some("_base_executable"),
            "base_prefix" => Some("base_prefix"),
            "exec_prefix" => Some("exec_prefix"),
            "executable" => Some("executable"),
            "module_search_paths" => Some("path"),
            "platlibdir" => Some("platlibdir"),
            "prefix" => Some("prefix"),
            "pycache_prefix" => Some("pycache_prefix"),
            "stdlib_dir" => Some("_stdlib_dir"),
            "warnoptions" => Some("warnoptions"),
            "xoptions" => Some("_xoptions"),
            "orig_argv" => Some("orig_argv"),
            _ => None,
        };
        if let Some(attr) = sys_attr {
            return vm.sys_module.get_attr(attr, vm);
        }
        let value: PyObjectRef = match name {
            "bytes_warning" => ctx.new_int(settings.bytes_warning).into(),
            "cpu_count" => ctx
                .new_int(settings.cpu_count.map_or(-1, |n| n.get()))
                .into(),
            "inspect" => ctx.new_bool(settings.inspect).into(),
            "int_max_str_digits" => ctx.new_int(vm.state.int_max_str_digits.load()).into(),
            "interactive" => ctx.new_bool(settings.interactive).into(),
            "optimization_level" => ctx.new_int(settings.optimize).into(),
            "parser_debug" => ctx.new_bool(settings.debug != 0).into(),
            "quiet" => ctx.new_bool(settings.quiet).into(),
            "use_environment" => ctx.new_bool(!settings.ignore_environment).into(),
            "verbose" => ctx.new_int(settings.verbose).into(),
            "write_bytecode" => {
                let dont_write_bytecode = vm.sys_module.get_attr("dont_write_bytecode", vm)?;
                ctx.new_bool(!dont_write_bytecode.try_to_bool(vm)?).into()
            }
            "buffered_stdio" => ctx.new_bool(settings.buffered_stdio).into(),
            "check_hash_pycs_mode" => ctx
                .new_str(settings.check_hash_pycs_mode.to_string())
                .into(),
            "code_debug_ranges" => ctx.new_bool(settings.code_debug_ranges).into(),
            "dev_mode" => ctx.new_bool(settings.dev_mode).into(),
            "faulthandler" => ctx.new_bool(settings.faulthandler).into(),
            "hash_seed" => ctx.new_int(settings.hash_seed.unwrap_or(0)).into(),
            "thread_inherit_context" => ctx.new_int(settings.thread_inherit_context as i32).into(),
            "context_aware_warnings" => ctx.new_int(settings.context_aware_warnings as i32).into(),
            "install_signal_handlers" => ctx.new_bool(settings.install_signal_handlers).into(),
            "isolated" => ctx.new_bool(settings.isolated).into(),
            "safe_path" => ctx.new_bool(settings.safe_path).into(),
            "site_import" => ctx.new_bool(settings.import_site).into(),
            "use_hash_seed" => ctx.new_bool(settings.hash_seed.is_some()).into(),
            "user_site_directory" => ctx.new_bool(settings.user_site_directory).into(),
            "warn_default_encoding" => ctx.new_bool(settings.warn_default_encoding).into(),
            "utf8_mode" => ctx.new_bool(settings.utf8_mode != 0).into(),
            _ => {
                return Err(vm.new_value_error(format!("unknown config option name: {name}")));
            }
        };
        Ok(value)
    }

    #[pyfunction]
    fn config_get(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        config_get_value(name.as_str(), vm)
    }

    #[pyfunction]
    fn config_getint(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<i32> {
        let name = name.as_str();
        let obj = config_get_value(name, vm)?;
        let Some(int) = obj.downcast_ref::<PyInt>() else {
            return Err(vm.new_type_error(format!("config option {name} is not an int")));
        };
        int.try_to_primitive::<i32>(vm).map_err(|_| {
            vm.new_overflow_error(format!(
                "config option {name} value does not fit into a C int"
            ))
        })
    }

    #[pyfunction]
    fn code_offset_to_line(code: PyObjectRef, offset: i32, vm: &VirtualMachine) -> PyResult<i32> {
        let Some(code) = code.downcast_ref::<PyCode>() else {
            return Err(vm.new_type_error("first arg must be a code object"));
        };
        Ok(code.addr2line(offset))
    }

    #[pyfunction]
    fn run_in_subinterp(code: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<i32> {
        #[cfg(feature = "threading")]
        {
            crate::stdlib::_testinternalcapi::run_string_in_new_subinterp(
                code.as_str(),
                crate::vm::InterpreterConfig::LEGACY,
                vm,
            )
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = code;
            Err(vm.new_runtime_error("sub-interpreter creation failed"))
        }
    }

    #[cfg(all(not(windows), feature = "threading"))]
    static WAIT_DONE: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>> =
        std::sync::Mutex::new(None);

    // These can be used to test things that care about the existence of another
    // thread that the threading module doesn't know about.
    #[cfg(all(not(windows), feature = "threading"))]
    #[pyfunction]
    fn _spawn_pthread_waiter(vm: &VirtualMachine) -> PyResult<()> {
        let mut wait_done = WAIT_DONE.lock().unwrap();
        if wait_done.is_some() {
            return Err(vm.new_runtime_error("thread already running"));
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        *wait_done = Some(sender);
        std::thread::spawn(move || {
            let _ = receiver.recv();
            *WAIT_DONE.lock().unwrap() = None;
        });
        Ok(())
    }

    #[cfg(all(not(windows), feature = "threading"))]
    #[pyfunction]
    fn _end_spawned_pthread(vm: &VirtualMachine) -> PyResult<()> {
        let wait_done = WAIT_DONE.lock().unwrap();
        let Some(sender) = wait_done.as_ref() else {
            return Err(vm.new_runtime_error("call _spawn_pthread_waiter 1st"));
        };
        let _ = sender.send(());
        Ok(())
    }

    /// PyUnicode_FromFormat for the object conversions.
    fn unicode_from_format(
        format: &str,
        objs: &[PyObjectRef],
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let mut out = String::with_capacity(format.len());
        let mut objs = objs.iter();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            let spec = chars.next();
            if spec == Some('%') {
                out.push('%');
                continue;
            }
            let obj = objs
                .next()
                .ok_or_else(|| vm.new_system_error("not enough arguments for format string"))?;
            let text = match spec {
                Some('R') => obj.repr(vm)?,
                Some('S' | 'U') => obj.str(vm)?,
                Some('A') => obj.ascii(vm)?,
                _ => {
                    return Err(vm.new_system_error(format!(
                        "PyUnicode_FromFormatV() expects a valid format character, got {format:?}"
                    )));
                }
            };
            out.push_str(&text.to_string_lossy());
        }
        Ok(out)
    }

    #[pyfunction]
    fn err_formatunraisable(
        exc: PyObjectRef,
        fmt: Option<PyUtf8StrRef>,
        objs: PosArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if vm.is_none(&exc) {
            return Ok(());
        }
        let exc = exc
            .downcast::<PyBaseException>()
            .map_err(|_| vm.new_type_error("exceptions must derive from BaseException"))?;
        let msg = fmt
            .map(|fmt| unicode_from_format(fmt.as_str(), &objs.into_vec(), vm))
            .transpose()?;
        vm.run_unraisable(exc, msg, vm.ctx.none());
        Ok(())
    }

    #[pyfunction(name = "Py_CompileStringExFlags")]
    fn py_compile_string_ex_flags(
        the_string: ArgBytesLike,
        filename: PyUtf8StrRef,
        start: i32,
        flags: i32,
        vm: &VirtualMachine,
    ) -> PyResult {
        const PY_CF_IGNORE_COOKIE: i32 = 0x0800;
        let mode = match start {
            256 => "single",
            257 => "exec",
            258 => "eval",
            345 => "func_type",
            _ => return Err(vm.new_system_error("invalid start symbol")),
        };
        let bytes = the_string.borrow_buf().to_vec();
        if bytes.contains(&0) {
            return Err(vm.new_value_error("embedded null byte"));
        }
        let source: PyObjectRef = vm.ctx.new_bytes(bytes).into();
        let source = if flags & PY_CF_IGNORE_COOKIE != 0 {
            vm.call_method(&source, "decode", ("utf-8",))?
        } else {
            source
        };
        let compile = vm.builtins.get_attr("compile", vm)?;
        compile.call(
            (source, filename, mode, flags & !PY_CF_IGNORE_COOKIE, true),
            vm,
        )
    }

    #[pyfunction(name = "PyTime_AsSecondsDouble")]
    fn test_pytime_as_seconds_double(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<f64> {
        use crate::stdlib::_testinternalcapi::pytime;
        let ts = pytime::pylong_as_int64(&obj, vm)?;
        Ok(pytime::pytime_as_seconds_double(ts))
    }

    fn type_arg<'a>(arg: &'a PyObjectRef, vm: &VirtualMachine) -> PyResult<&'a Py<PyType>> {
        arg.downcast_ref::<PyType>()
            .ok_or_else(|| vm.new_type_error("argument must be a type"))
    }

    /// type->tp_version_tag
    #[pyfunction]
    fn type_get_version(ty: PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        Ok(type_arg(&ty, vm)?
            .tp_version_tag
            .load(core::sync::atomic::Ordering::Acquire))
    }

    /// PyType_Modified
    #[pyfunction]
    fn type_modified(ty: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        type_arg(&ty, vm)?.modified();
        Ok(())
    }

    /// PyUnstable_Type_AssignVersionTag
    #[pyfunction]
    fn type_assign_version(ty: PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        Ok((type_arg(&ty, vm)?.version_for_specialization(vm) != 0) as i32)
    }

    fn meth_varargs(zelf: PyObjectRef, args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((zelf, vm.ctx.new_tuple(args.into_vec())))
    }

    fn meth_varargs_keywords(
        zelf: PyObjectRef,
        args: PosArgs,
        kwargs: KwArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let kwargs: PyObjectRef = if kwargs.is_empty() {
            vm.ctx.none()
        } else {
            let dict = vm.ctx.new_dict();
            for (key, value) in kwargs {
                dict.set_item(vm.ctx.new_str(key).as_object(), value, vm)?;
            }
            dict.into()
        };
        Ok(vm.new_tuple((zelf, vm.ctx.new_tuple(args.into_vec()), kwargs)))
    }

    fn meth_o(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((zelf, obj))
    }

    fn meth_noargs(zelf: PyObjectRef) -> PyObjectRef {
        zelf
    }

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        const METHODS: &[PyMethodDef] = &[
            PyMethodDef::new_const(
                "meth_varargs",
                meth_varargs,
                PyMethodFlags::empty(),
                ItemDoc::NONE,
            ),
            PyMethodDef::new_const(
                "meth_varargs_keywords",
                meth_varargs_keywords,
                PyMethodFlags::empty(),
                ItemDoc::NONE,
            ),
            PyMethodDef::new_const("meth_o", meth_o, PyMethodFlags::empty(), ItemDoc::NONE),
            PyMethodDef::new_const(
                "meth_noargs",
                meth_noargs,
                PyMethodFlags::empty(),
                ItemDoc::NONE,
            ),
            PyMethodDef::new_const(
                "meth_fastcall",
                meth_varargs,
                PyMethodFlags::empty(),
                ItemDoc::NONE,
            ),
            PyMethodDef::new_const(
                "meth_fastcall_keywords",
                meth_varargs_keywords,
                PyMethodFlags::empty(),
                ItemDoc::NONE,
            ),
        ];
        for definition in METHODS {
            let function = definition.build_bound_function(&vm.ctx, module.to_owned().into());
            function
                .as_object()
                .set_attr("__module__", vm.ctx.new_str("_testcapi"), vm)?;
            module
                .dict()
                .set_item(definition.name, function.into(), vm)?;
        }
        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "MethInstance", module = "_testcapi")]
    #[derive(Debug, Default, PyPayload)]
    struct MethInstance;

    impl DefaultConstructor for MethInstance {}

    #[pyclass(with(Constructor))]
    impl MethInstance {
        #[pymethod]
        fn meth_varargs(zelf: PyObjectRef, args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(zelf, args, vm)
        }

        #[pymethod]
        fn meth_varargs_keywords(
            zelf: PyObjectRef,
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(zelf, args, kwargs, vm)
        }

        #[pymethod]
        fn meth_o(zelf: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
            meth_o(zelf, obj, vm)
        }

        #[pymethod]
        fn meth_noargs(zelf: PyObjectRef) -> PyObjectRef {
            meth_noargs(zelf)
        }

        #[pymethod]
        fn meth_fastcall(zelf: PyObjectRef, args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(zelf, args, vm)
        }

        #[pymethod]
        fn meth_fastcall_keywords(
            zelf: PyObjectRef,
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(zelf, args, kwargs, vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "MethClass", module = "_testcapi")]
    #[derive(Debug, Default, PyPayload)]
    struct MethClass;

    impl DefaultConstructor for MethClass {}

    #[pyclass(with(Constructor))]
    impl MethClass {
        #[pyclassmethod]
        fn meth_varargs(cls: PyTypeRef, args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(cls.into(), args, vm)
        }

        #[pyclassmethod]
        fn meth_varargs_keywords(
            cls: PyTypeRef,
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(cls.into(), args, kwargs, vm)
        }

        #[pyclassmethod]
        fn meth_o(cls: PyTypeRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
            meth_o(cls.into(), obj, vm)
        }

        #[pyclassmethod]
        fn meth_noargs(cls: PyTypeRef) -> PyObjectRef {
            meth_noargs(cls.into())
        }

        #[pyclassmethod]
        fn meth_fastcall(cls: PyTypeRef, args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(cls.into(), args, vm)
        }

        #[pyclassmethod]
        fn meth_fastcall_keywords(
            cls: PyTypeRef,
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(cls.into(), args, kwargs, vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "MethStatic", module = "_testcapi")]
    #[derive(Debug, Default, PyPayload)]
    struct MethStatic;

    impl DefaultConstructor for MethStatic {}

    // A static method has no self; _null_to_none turns it into None.
    #[pyclass(with(Constructor))]
    impl MethStatic {
        #[pystaticmethod]
        fn meth_varargs(args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(vm.ctx.none(), args, vm)
        }

        #[pystaticmethod]
        fn meth_varargs_keywords(
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(vm.ctx.none(), args, kwargs, vm)
        }

        #[pystaticmethod]
        fn meth_o(obj: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
            meth_o(vm.ctx.none(), obj, vm)
        }

        #[pystaticmethod]
        fn meth_noargs(vm: &VirtualMachine) -> PyObjectRef {
            meth_noargs(vm.ctx.none())
        }

        #[pystaticmethod]
        fn meth_fastcall(args: PosArgs, vm: &VirtualMachine) -> PyTupleRef {
            meth_varargs(vm.ctx.none(), args, vm)
        }

        #[pystaticmethod]
        fn meth_fastcall_keywords(
            args: PosArgs,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            meth_varargs_keywords(vm.ctx.none(), args, kwargs, vm)
        }
    }
}
