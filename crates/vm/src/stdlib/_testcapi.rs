// Native fixtures shared by the standard-library tests. This deliberately
// implements only the fixtures below, not CPython's complete C API test module.
pub(crate) use _testcapi::module_def;
mod heap;
mod meth;
#[cfg(feature = "capi")]
mod monitoring;

#[pymodule]
mod _testcapi {
    use crate::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
        builtins::{PyCode, PyInt, PyModule, PyStrRef, PyType, PyTypeRef, PyUtf8StrRef},
        function::{ArgBytesLike, ArgIntoBool, ItemDoc, PosArgs, PyMethodDef, PyMethodFlags},
        types::Constructor,
    };
    use core::sync::atomic::Ordering;

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        super::meth::extend_module(vm, module)?;
        #[cfg(feature = "capi")]
        super::monitoring::extend_module(vm, module)?;
        Ok(())
    }

    #[pyattr]
    const ONE: u8 = 1;

    #[pyattr(name = "HeapCCollection")]
    fn heap_c_collection(vm: &VirtualMachine) -> PyTypeRef {
        super::heap::heap_c_collection(vm)
    }

    #[pyattr(name = "HeapCTypeMetaclassNullNew")]
    fn heap_ctype_metaclass_null_new(vm: &VirtualMachine) -> PyTypeRef {
        super::heap::heap_ctype_metaclass_null_new(vm)
    }

    #[pyfunction]
    fn pytype_fromspec_meta(meta: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTypeRef> {
        super::heap::pytype_fromspec_meta(meta, vm)
    }

    #[pyfunction]
    fn bad_get(
        descriptor: PyObjectRef,
        object: PyObjectRef,
        class: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        super::heap::bad_get(descriptor, object, class, vm)
    }

    #[cfg(feature = "threading")]
    #[pyfunction]
    fn run_in_subinterp(code: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<i32> {
        if code.as_str().contains('\0') {
            return Err(vm.new_value_error("embedded null character"));
        }
        crate::stdlib::_testinternalcapi::run_string_in_new_subinterp(
            code.as_str(),
            crate::vm::InterpreterConfig::LEGACY,
            crate::vm::InterpreterWhence::LegacyCapi,
            vm,
        )
    }

    #[pyattr]
    const INT_MIN: libc::c_int = libc::c_int::MIN;
    #[pyattr]
    const INT_MAX: libc::c_int = libc::c_int::MAX;
    #[pyattr]
    const UINT_MAX: libc::c_uint = libc::c_uint::MAX;
    #[pyattr]
    const LONG_MIN: libc::c_long = libc::c_long::MIN;
    #[pyattr]
    const LONG_MAX: libc::c_long = libc::c_long::MAX;
    #[pyattr]
    const ULONG_MAX: libc::c_ulong = libc::c_ulong::MAX;
    #[pyattr]
    const ULLONG_MAX: libc::c_ulonglong = libc::c_ulonglong::MAX;
    #[pyattr]
    const PY_SSIZE_T_MIN: isize = isize::MIN;
    #[pyattr]
    const PY_SSIZE_T_MAX: isize = isize::MAX;
    #[pyattr]
    const FLT_MAX: f64 = f32::MAX as f64;
    #[pyattr]
    const FLT_MIN: f64 = f32::MIN_POSITIVE as f64;
    #[pyattr]
    const DBL_MAX: f64 = f64::MAX;
    #[pyattr]
    const DBL_MIN: f64 = f64::MIN_POSITIVE;
    #[pyattr(name = "nan_msb_is_signaling")]
    const NAN_MSB_IS_SIGNALING: bool = f64::NAN.to_bits() & (1_u64 << 51) == 0;

    #[pyfunction]
    fn code_offset_to_line(args: PosArgs, vm: &VirtualMachine) -> PyResult<i32> {
        let [code, offset] = args.as_ref() else {
            return Err(vm.new_type_error("code_offset_to_line takes 2 arguments"));
        };
        let offset: i32 =
            offset.try_index(vm)?.as_bigint().try_into().map_err(|_| {
                vm.new_overflow_error("Python int too large to convert to C int32_t")
            })?;
        let code = code
            .downcast_ref::<PyCode>()
            .ok_or_else(|| vm.new_type_error("first arg must be a code object"))?;
        Ok(code.addr2line(offset))
    }

    #[pyfunction]
    fn config_get(name: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult {
        // Expose configuration stored by this interpreter; other CPython-only
        // configuration names have no corresponding runtime setting here.
        let settings = &vm.state.config.settings;
        let value = match name.as_str() {
            "code_debug_ranges" => vm.ctx.new_bool(settings.code_debug_ranges).into(),
            "isolated" => vm.ctx.new_bool(settings.isolated).into(),
            "dev_mode" => vm.ctx.new_bool(settings.dev_mode).into(),
            "faulthandler" => vm.ctx.new_bool(settings.faulthandler).into(),
            "site_import" => vm.ctx.new_bool(settings.import_site).into(),
            "inspect" => vm.ctx.new_bool(settings.inspect).into(),
            "interactive" => vm.ctx.new_bool(settings.interactive).into(),
            "write_bytecode" => vm.ctx.new_bool(settings.write_bytecode).into(),
            "warn_default_encoding" => vm.ctx.new_bool(settings.warn_default_encoding).into(),
            "thread_inherit_context" => vm.ctx.new_bool(settings.thread_inherit_context).into(),
            "context_aware_warnings" => vm.ctx.new_bool(settings.context_aware_warnings).into(),
            "optimization_level" => vm.ctx.new_int(settings.optimize).into(),
            "verbose" => vm.ctx.new_int(settings.verbose).into(),
            "bytes_warning" => vm.ctx.new_int(settings.bytes_warning).into(),
            "lazy_imports" => vm.ctx.new_int(settings.lazy_imports).into(),
            _ => {
                return Err(
                    vm.new_value_error(format!("unknown config option name: {}", name.as_str()))
                );
            }
        };
        Ok(value)
    }

    #[pyfunction(name = "PyTime_AsSecondsDouble")]
    fn pytime_as_seconds_double(args: PosArgs, vm: &VirtualMachine) -> PyResult<f64> {
        let [object] = args.as_ref() else {
            return Err(vm.new_type_error(format!(
                "function takes exactly 1 argument ({} given)",
                args.as_ref().len()
            )));
        };
        let Some(nanoseconds) = object.downcast_ref::<PyInt>() else {
            return Err(vm.new_type_error(format!(
                "expect int, got {}",
                object.class().fully_qualified_name(vm)?
            )));
        };
        let nanoseconds = nanoseconds
            .as_bigint()
            .try_into()
            .map_err(|_| vm.new_overflow_error("int too big to convert"))?;
        Ok(crate::stdlib::time::pytime::as_seconds_double(nanoseconds))
    }

    #[pyfunction]
    fn type_get_version(ty: PyTypeRef) -> u32 {
        ty.tp_version_tag().load(Ordering::Acquire)
    }

    #[pyfunction]
    fn type_assign_version(ty: PyTypeRef, vm: &VirtualMachine) -> i32 {
        i32::from(ty.version_for_specialization(vm) != 0)
    }

    #[pyfunction]
    fn type_modified(ty: PyTypeRef) {
        ty.modified();
    }

    #[pyfunction]
    fn set_errno(value: i32) {
        crate::host_env::os::set_errno(value);
    }

    #[pyfunction(name = "PyImport_GetLazyImportsMode")]
    fn get_lazy_imports_mode(vm: &VirtualMachine) -> &'static str {
        if vm.state.lazy_imports.all.load(Ordering::Acquire) {
            "all"
        } else {
            "normal"
        }
    }

    #[pyfunction(name = "PyImport_SetLazyImportsMode")]
    fn set_lazy_imports_mode(mode: PyUtf8StrRef, vm: &VirtualMachine) -> PyResult<()> {
        let all = match mode.as_str() {
            "normal" => false,
            "all" => true,
            _ => return Err(vm.new_value_error("invalid mode")),
        };
        vm.state.lazy_imports.all.store(all, Ordering::Release);
        Ok(())
    }

    #[pyfunction(name = "PyImport_GetLazyImportsFilter")]
    fn get_lazy_imports_filter(vm: &VirtualMachine) -> PyObjectRef {
        vm.state
            .lazy_imports
            .filter()
            .unwrap_or_else(|| vm.ctx.none())
    }

    #[pyfunction(name = "PyImport_SetLazyImportsFilter")]
    fn set_lazy_imports_filter(filter: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let filter = if vm.is_none(&filter) {
            None
        } else {
            if !filter.is_callable() {
                return Err(vm.new_value_error("filter provided but is not callable"));
            }
            Some(filter)
        };
        vm.state.lazy_imports.set_filter(filter);
        Ok(())
    }

    #[pyfunction]
    fn lazy_import_without_frame(name: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let lazy_import = vm.builtins.get_attr("__lazy_import__", vm)?;
        let saved = crate::vm::thread::set_current_frame(core::ptr::null());
        let _restore = scopeguard::guard(saved, |frame| {
            let _ = crate::vm::thread::set_current_frame(frame);
        });
        lazy_import.call((name,), vm)
    }

    #[cfg(all(unix, feature = "threading"))]
    struct NativeWaiter {
        state: alloc::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
        thread: std::thread::JoinHandle<()>,
    }

    #[cfg(all(unix, feature = "threading"))]
    static NATIVE_WAITER: std::sync::Mutex<Option<NativeWaiter>> = std::sync::Mutex::new(None);

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _spawn_pthread_waiter(vm: &VirtualMachine) -> PyResult<()> {
        let mut slot = NATIVE_WAITER.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(vm.new_runtime_error("thread already running"));
        }
        let state =
            alloc::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let child_state = state.clone();
        // A host thread with no Python thread state is intentional: the fixture
        // exercises fork detection beyond threading.active_count().
        let thread = std::thread::Builder::new()
            .spawn(move || {
                let (lock, wake) = &*child_state;
                let released = lock.lock().unwrap_or_else(|e| e.into_inner());
                drop(wake.wait_while(released, |released| !*released));
            })
            .map_err(|err| vm.new_runtime_error(err.to_string()))?;
        *slot = Some(NativeWaiter { state, thread });
        Ok(())
    }

    #[cfg(all(unix, feature = "threading"))]
    #[pyfunction]
    fn _end_spawned_pthread(vm: &VirtualMachine) -> PyResult<()> {
        let waiter = NATIVE_WAITER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or_else(|| vm.new_runtime_error("call _spawn_pthread_waiter 1st"))?;
        let (lock, wake) = &*waiter.state;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_one();
        vm.allow_threads(|| waiter.thread.join())
            .map_err(|_| vm.new_runtime_error("native waiter thread panicked"))
    }

    // The `y` argument format used by CPython accepts buffers that do not
    // require a release callback, and rejects embedded NUL bytes.
    struct FatalMessage(Vec<u8>);

    impl TryFromObject for FatalMessage {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let slots = &obj.class().slots;
            if slots.has_release_buffer.load() || slots.python_release_buffer.load() {
                return Err(vm.new_type_error(format!(
                    "fatal_error() argument 1 must be read-only bytes-like object, not {}",
                    obj.class().name()
                )));
            }
            let buffer = ArgBytesLike::try_from_object(vm, obj)?;
            let bytes = buffer.borrow_buf();
            if bytes.contains(&0) {
                return Err(vm.new_value_error("embedded null byte"));
            }
            Ok(Self(bytes.to_vec()))
        }
    }

    #[derive(FromArgs)]
    struct FatalErrorArgs {
        message: FatalMessage,
        #[pyarg(positional, default = false)]
        release_gil: ArgIntoBool,
    }

    #[pyfunction]
    fn fatal_error(args: FatalErrorArgs, vm: &VirtualMachine) -> PyResult<()> {
        // Capture the Python stack while attached. Fatal diagnostics and the
        // abort itself must not depend on Python's replaceable stderr object.
        let mut diagnostic = b"Fatal Python error: _testcapi_fatal_error_impl: ".to_vec();
        diagnostic.extend_from_slice(&args.message.0);
        diagnostic.extend_from_slice(
            b"\nPython runtime state: initialized\n\nStack (most recent call first):\n",
        );
        let mut frame = vm.current_frame();
        while let Some(current) = frame {
            let code = current.f_code();
            diagnostic.extend_from_slice(
                format!(
                    "  File \"{}\", line {} in {}\n",
                    code.source_path().as_wtf8(),
                    current.lineno(),
                    code.obj_name.as_wtf8()
                )
                .as_bytes(),
            );
            frame = current.f_back(vm);
        }
        let abort = || {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&diagnostic);
            rustpython_host_env::os::abort()
        };
        if args.release_gil.into_bool() {
            vm.allow_threads(abort)
        } else {
            abort()
        }
    }

    #[pyattr]
    #[pyclass(module = "_testcapi", name = "DocStringUnrepresentableSignatureTest")]
    #[derive(Debug, PyPayload)]
    struct DocStringUnrepresentableSignatureTest;

    #[pyclass(with(Constructor))]
    impl DocStringUnrepresentableSignatureTest {
        #[extend_class]
        fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
            // CPython's fixture deliberately accepts any positional arguments;
            // its text signature exists to test module-scope default lookup.
            const WITH_DEFAULT: PyMethodDef = PyMethodDef::new_const(
                "with_default",
                DocStringUnrepresentableSignatureTest::with_default,
                PyMethodFlags::METHOD.union(PyMethodFlags::VARARGS),
                ItemDoc::static_text(
                    "with_default($self, /, x=ONE)\n--\n\nThis instance method has a default parameter value from the module scope.",
                ),
            );
            class.set_str_attr(
                "with_default",
                WITH_DEFAULT.to_proper_method(class, ctx),
                ctx,
            );
        }

        fn with_default(&self, _args: PosArgs<PyObjectRef>) {}
    }

    impl Constructor for DocStringUnrepresentableSignatureTest {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: (), _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self)
        }
    }
}
