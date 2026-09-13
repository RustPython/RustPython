//! Low-level multiple-interpreter primitives (`_interpreters`).
//!
//! Mirrors CPython `Modules/_interpretersmodule.c`.

pub(crate) use _interpreters::{init_xi_types, module_def};
#[cfg_attr(not(feature = "threading"), allow(unused_imports))]
pub(crate) use _interpreters::{
    interpreter_error, interpreter_not_found, not_shareable_error, xibufferview_from_buffer,
};

#[pymodule]
pub(crate) mod _interpreters {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyCode, PyDict, PyException, PyFunction, PyModule, PyStr},
        class::PyClassImpl,
        function::{ArgSpec, FuncArgs},
        protocol::{BufferMethods, PyBuffer},
        types::{AsBuffer, Constructor},
        vm::{
            InterpreterConfig, InterpreterGil, InterpreterWhence,
            crossinterp::{self, ExcInfo, Fallback, SharedValue},
            runtime,
        },
    };
    use core::sync::atomic::Ordering;
    use num_traits::ToPrimitive;

    #[pyattr]
    pub(crate) const WHENCE_UNKNOWN: i32 = InterpreterWhence::Unknown as i32;
    #[pyattr]
    pub(crate) const WHENCE_RUNTIME: i32 = InterpreterWhence::Runtime as i32;
    #[pyattr]
    pub(crate) const WHENCE_LEGACY_CAPI: i32 = InterpreterWhence::LegacyCapi as i32;
    #[pyattr]
    pub(crate) const WHENCE_CAPI: i32 = InterpreterWhence::Capi as i32;
    #[pyattr]
    pub(crate) const WHENCE_XI: i32 = InterpreterWhence::Xi as i32;
    #[pyattr]
    pub(crate) const WHENCE_STDLIB: i32 = InterpreterWhence::Stdlib as i32;

    #[pyattr]
    #[pyexception(name = "InterpreterError", module = "concurrent.interpreters", base = PyException)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyInterpreterError(PyException);

    #[pyexception]
    impl PyInterpreterError {}

    #[pyattr]
    #[pyexception(name = "InterpreterNotFoundError", module = "concurrent.interpreters", base = PyInterpreterError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyInterpreterNotFoundError(PyInterpreterError);

    #[pyexception]
    impl PyInterpreterNotFoundError {}

    #[pyattr]
    #[pyexception(name = "NotShareableError", module = "concurrent.interpreters", base = crate::exceptions::types::PyTypeError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyNotShareableError(crate::exceptions::types::PyTypeError);

    #[pyexception]
    impl PyNotShareableError {}

    #[pyattr]
    #[pyclass(name = "CrossInterpreterBufferView", module = "_interpreters")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct XiBufferView {
        view: PyBuffer,
    }

    static XI_BUFFER_VIEW_METHODS: BufferMethods = BufferMethods {
        obj_bytes: |buffer| buffer.obj_as::<XiBufferView>().view.obj_bytes(),
        obj_bytes_mut: |buffer| buffer.obj_as::<XiBufferView>().view.obj_bytes_mut(),
        release: |_buffer| {},
        retain: |_buffer| {},
    };

    #[pyclass(with(AsBuffer), flags(BASETYPE, IMMUTABLETYPE, DISALLOW_INSTANTIATION))]
    impl XiBufferView {}

    impl AsBuffer for XiBufferView {
        // xibufferview_getbuf: the view is handed out as-is, but with this
        // object as the exporter so the sending interpreter's object stays out
        // of the receiving one.
        fn as_buffer(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyBuffer> {
            Ok(PyBuffer::new(
                zelf.to_owned().into(),
                zelf.view.desc.clone(),
                &XI_BUFFER_VIEW_METHODS,
            ))
        }
    }

    /// `xibufferview_from_buffer`.
    pub(crate) fn xibufferview_from_buffer(view: PyBuffer, vm: &VirtualMachine) -> PyObjectRef {
        XiBufferView { view }.into_pyobject(vm)
    }

    /// The cross-interpreter exception types belong to the runtime rather than
    /// to this module, so they are created before either of the modules that
    /// raise them finishes importing.
    pub(crate) fn init_xi_types(vm: &VirtualMachine) {
        let module = vm.new_pyobj("concurrent.interpreters");
        for class in [
            PyInterpreterError::make_static_type(),
            PyInterpreterNotFoundError::make_static_type(),
            PyNotShareableError::make_static_type(),
        ] {
            class.set_attr(crate::identifier!(vm, __module__), module.clone());
        }
    }

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        init_xi_types(vm);
        __module_exec(vm, module);
        // register_memoryview_xid
        crossinterp::register_memoryview_xid();
        Ok(())
    }

    pub(crate) fn interpreter_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            PyInterpreterError::class(&vm.ctx).to_owned(),
            msg.into().into(),
        )
    }

    pub(crate) fn interpreter_not_found(vm: &VirtualMachine, id: i64) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            PyInterpreterNotFoundError::class(&vm.ctx).to_owned(),
            format!("unrecognized interpreter ID {id}").into(),
        )
    }

    pub(crate) fn not_shareable_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> PyBaseExceptionRef {
        vm.new_exception_msg(
            PyNotShareableError::class(&vm.ctx).to_owned(),
            msg.into().into(),
        )
    }

    /// `_PyArg_BadArgument`.
    fn bad_argument(
        func: &str,
        pos: &str,
        expected: &str,
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        vm.new_type_error(format!(
            "{func}() {pos} must be {expected}, not {}",
            if vm.is_none(obj) {
                "None".into()
            } else {
                obj.class().name()
            }
        ))
    }

    /// `_PyInterpreterState_ObjectToID`.
    fn parse_id(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        if !obj.number().is_index() {
            return Err(vm.new_type_error(format!(
                "interpreter ID must be an int, got {}",
                obj.class().name()
            )));
        }
        let n = obj.try_index(vm)?;
        let id = n
            .as_bigint()
            .to_i64()
            .ok_or_else(|| vm.new_overflow_error("int too big to convert"))?;
        if id < 0 {
            let repr = obj.repr(vm)?;
            return Err(vm.new_value_error(format!(
                "interpreter ID must be a non-negative int, got {repr}"
            )));
        }
        Ok(id)
    }

    /// `resolve_interp`. `id` is `None` for the current interpreter.
    fn resolve_interp(
        id: Option<i64>,
        restricted: bool,
        reqready: bool,
        op: &str,
        vm: &VirtualMachine,
    ) -> PyResult<i64> {
        // The wording depends on whether an ID was passed, not on which
        // interpreter it names.
        let target = match id {
            Some(id) => format!("interpreter {id}"),
            None => "current interpreter".to_owned(),
        };
        let id = id.unwrap_or(vm.state.interpreter_id);
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        if reqready && !state.ready.load(Ordering::Acquire) {
            return Err(interpreter_error(
                vm,
                format!("cannot {op} {target} (not ready)"),
            ));
        }
        if restricted && state.whence != InterpreterWhence::Stdlib {
            return Err(interpreter_error(
                vm,
                format!("cannot {op} unrecognized {target}"),
            ));
        }
        Ok(id)
    }

    fn require_dict<'a>(
        obj: &'a PyObject,
        func: &str,
        pos: &str,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<PyDict>> {
        obj.downcast_ref::<PyDict>()
            .ok_or_else(|| bad_argument(func, pos, "dict", obj, vm))
    }

    /// The `p` converter: a predicate that only reads truthiness.
    fn flag(slot: Option<&PyObjectRef>, vm: &VirtualMachine) -> PyResult<bool> {
        match slot {
            Some(o) => o.clone().is_true(vm),
            None => Ok(false),
        }
    }

    /// The `O!` converter for `PyDict_Type`.
    fn check_dict(obj: &PyObject, func: &str, pos: &str, vm: &VirtualMachine) -> PyResult<()> {
        require_dict(obj, func, pos, vm).map(drop)
    }

    /// `_config_dict_get_bool`: only exact `True` / `False` are accepted.
    fn config_bool(value: &PyObject, name: &str, vm: &VirtualMachine) -> PyResult<bool> {
        if value.is(&vm.ctx.true_value) {
            Ok(true)
        } else if value.is(&vm.ctx.false_value) {
            Ok(false)
        } else {
            Err(vm.new_type_error(format!("invalid config type: {name}")))
        }
    }

    fn config_gil(value: &PyObject, vm: &VirtualMachine) -> PyResult<InterpreterGil> {
        let s = value
            .downcast_ref::<PyStr>()
            .ok_or_else(|| vm.new_type_error("invalid config type: gil"))?;
        let s = s.to_str().unwrap_or("");
        InterpreterGil::from_name(s).ok_or_else(|| {
            vm.new_value_error(format!("unsupported interpreter config .gil value '{s}'"))
        })
    }

    /// `interp_config_from_dict`.
    fn config_from_dict(
        cfg: &mut InterpreterConfig,
        dict: &Py<PyDict>,
        missing_allowed: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut seen = 0usize;
        macro_rules! copy_bool {
            ($field:ident) => {
                match dict.get_item_opt(stringify!($field), vm)? {
                    Some(v) => {
                        cfg.$field = config_bool(&v, stringify!($field), vm)?;
                        seen += 1;
                    }
                    None if missing_allowed => {}
                    None => {
                        return Err(vm.new_value_error(format!(
                            "missing config key: {}",
                            stringify!($field)
                        )));
                    }
                }
            };
        }
        copy_bool!(use_main_obmalloc);
        copy_bool!(allow_fork);
        copy_bool!(allow_exec);
        copy_bool!(allow_threads);
        copy_bool!(allow_daemon_threads);
        copy_bool!(check_multi_interp_extensions);
        match dict.get_item_opt("gil", vm)? {
            Some(v) => {
                cfg.gil = config_gil(&v, vm)?;
                seen += 1;
            }
            None if missing_allowed => {}
            None => return Err(vm.new_value_error("missing config key: gil")),
        }

        let unused = dict.__len__() - seen;
        if unused > 0 {
            let extra = vm.ctx.new_dict();
            for (key, value) in dict {
                let known = key.downcast_ref::<PyStr>().is_some_and(|s| {
                    matches!(
                        s.to_str().unwrap_or(""),
                        "use_main_obmalloc"
                            | "allow_fork"
                            | "allow_exec"
                            | "allow_threads"
                            | "allow_daemon_threads"
                            | "check_multi_interp_extensions"
                            | "gil"
                    )
                });
                if !known {
                    extra.set_item(&*key, value, vm)?;
                }
            }
            let repr = extra.as_object().repr(vm)?;
            let plural = if unused == 1 { "item" } else { "items" };
            return Err(
                vm.new_value_error(format!("config dict has {unused} extra {plural} ({repr})"))
            );
        }
        Ok(())
    }

    /// `init_named_config`.
    fn named_config(name: Option<&str>, vm: &VirtualMachine) -> PyResult<InterpreterConfig> {
        let name = name.unwrap_or("isolated");
        InterpreterConfig::named(name)
            .ok_or_else(|| vm.new_value_error(format!("unsupported config name '{name}'")))
    }

    /// `config_from_object`.
    fn parse_config(obj: Option<&PyObject>, vm: &VirtualMachine) -> PyResult<InterpreterConfig> {
        let Some(obj) = obj.filter(|o| !vm.is_none(o)) else {
            return named_config(None, vm);
        };
        if let Some(s) = obj.downcast_ref::<PyStr>() {
            return named_config(s.to_str(), vm);
        }
        let dict = obj.get_attr("__dict__", vm).map_err(|_| {
            let repr = obj
                .repr(vm)
                .map_or_else(|_| obj.class().name().to_string(), |s| s.to_string());
            vm.new_type_error(format!("bad config {repr}"))
        })?;
        let dict = dict
            .downcast_ref::<PyDict>()
            .ok_or_else(|| vm.new_type_error("dict expected"))?;
        let mut cfg = InterpreterConfig::ISOLATED;
        config_from_dict(&mut cfg, dict, false, vm)?;
        Ok(cfg)
    }

    /// `_PyInterpreterConfig_AsDict` wrapped in a `types.SimpleNamespace`.
    fn config_namespace(cfg: InterpreterConfig, vm: &VirtualMachine) -> PyObjectRef {
        crate::py_namespace!(vm, {
            "use_main_obmalloc" => vm.ctx.new_bool(cfg.use_main_obmalloc),
            "allow_fork" => vm.ctx.new_bool(cfg.allow_fork),
            "allow_exec" => vm.ctx.new_bool(cfg.allow_exec),
            "allow_threads" => vm.ctx.new_bool(cfg.allow_threads),
            "allow_daemon_threads" => vm.ctx.new_bool(cfg.allow_daemon_threads),
            "check_multi_interp_extensions" => vm.ctx.new_bool(cfg.check_multi_interp_extensions),
            "gil" => vm.ctx.new_str(cfg.gil.as_str()),
        })
        .into()
    }

    fn summary(id: i64, whence: InterpreterWhence, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx
            .new_tuple(vec![
                vm.ctx.new_int(id).into(),
                vm.ctx.new_int(whence.as_i32()).into(),
            ])
            .into()
    }

    #[pyfunction]
    fn new_config(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.new_config";
        if args.args.len() > 1 {
            return Err(vm.new_type_error(format!(
                "{FUNCNAME}() takes at most 1 argument ({} given)",
                args.args.len()
            )));
        }
        // The `s` converter: a str, and nothing else.
        let name = match args.args.first() {
            Some(o) => Some(
                o.downcast_ref::<PyStr>()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| bad_argument(FUNCNAME, "argument 1", "str", o, vm))?
                    .to_owned(),
            ),
            None => None,
        };
        let mut cfg = named_config(name.as_deref(), vm)?;
        if !args.kwargs.is_empty() {
            let overrides = vm.ctx.new_dict();
            for (key, value) in &args.kwargs {
                overrides.set_item(&**key, value.clone(), vm)?;
            }
            config_from_dict(&mut cfg, &overrides, true, vm)?;
        }
        Ok(config_namespace(cfg, vm))
    }

    #[pyfunction]
    fn create(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "create",
            keywords: &["config", "reqrefs"],
            required: 0,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let reqrefs = flag(parsed[1].as_ref(), vm)?;
        let config = parse_config(parsed[0].as_deref(), vm)?;
        new_interpreter(config, reqrefs, vm)
    }

    /// `_PyXI_NewInterpreter`, registered as a runtime-owned interpreter.
    #[cfg(feature = "threading")]
    fn new_interpreter(config: InterpreterConfig, reqrefs: bool, vm: &VirtualMachine) -> PyResult {
        let interp =
            crate::Interpreter::create_subinterpreter_from_vm(vm, config).map_err(|msg| {
                let cause = vm.new_runtime_error(msg.to_owned());
                let exc = interpreter_error(vm, "interpreter creation failed");
                exc.set___context__(Some(cause));
                exc
            })?;
        if reqrefs {
            // Decref to 0 will destroy the interpreter.
            interp
                .global_state
                .require_idref
                .store(true, Ordering::Release);
        }
        let id = runtime::store_owned_interpreter(interp);
        Ok(vm.ctx.new_int(id).into())
    }

    #[cfg(not(feature = "threading"))]
    fn new_interpreter(
        _config: InterpreterConfig,
        _reqrefs: bool,
        vm: &VirtualMachine,
    ) -> PyResult {
        Err(vm.new_runtime_error("isolated interpreters require threading"))
    }

    #[pyfunction]
    fn destroy(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = ArgSpec {
            fname: "destroy",
            keywords: &["id", "restrict"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let restricted = flag(parsed[1].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restricted, false, "destroy", vm)?;
        if id == vm.state.interpreter_id {
            return Err(interpreter_error(
                vm,
                "cannot destroy the current interpreter",
            ));
        }
        if crossinterp::is_running(id) {
            return Err(interpreter_error(vm, "interpreter running"));
        }
        #[cfg(feature = "threading")]
        {
            if runtime::is_owned_interpreter(id) {
                runtime::destroy_owned_interpreter(id)
                    .ok_or_else(|| interpreter_not_found(vm, id))?;
                return Ok(());
            }
        }
        Err(interpreter_not_found(vm, id))
    }

    #[pyfunction]
    fn list_all(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "_interpreters.list_all",
            keywords: &["require_ready"],
            required: 0,
            max_positional: 0,
        }
        .parse(&args, vm)?;
        let reqready = flag(parsed[0].as_ref(), vm)?;
        let mut items = Vec::new();
        for info in runtime::list_interpreters() {
            if reqready
                && !runtime::lookup_interpreter(info.id)
                    .is_some_and(|s| s.ready.load(Ordering::Acquire))
            {
                continue;
            }
            items.push(summary(info.id, info.whence, vm));
        }
        Ok(vm.ctx.new_list(items).into())
    }

    #[pyfunction]
    fn get_current(vm: &VirtualMachine) -> PyObjectRef {
        summary(vm.state.interpreter_id, vm.state.whence, vm)
    }

    #[pyfunction]
    fn get_main(vm: &VirtualMachine) -> PyObjectRef {
        let id = runtime::main_interpreter_id().unwrap_or(0);
        summary(id, InterpreterWhence::Runtime, vm)
    }

    #[pyfunction]
    fn is_running(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "is_running",
            keywords: &["id", "restrict"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let restricted = flag(parsed[1].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restricted, true, "check if running for", vm)?;
        Ok(vm.ctx.new_bool(crossinterp::is_running(id)).into())
    }

    #[pyfunction]
    fn whence(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "whence",
            keywords: &["id"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        Ok(vm.ctx.new_int(state.whence.as_i32()).into())
    }

    #[pyfunction]
    fn get_config(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "get_config",
            keywords: &["id", "restrict"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let restricted = flag(parsed[1].as_ref(), vm)?;
        let id_obj = parsed[0].as_deref().unwrap();
        let id = if vm.is_none(id_obj) {
            None
        } else {
            Some(parse_id(id_obj, vm)?)
        };
        let id = resolve_interp(id, restricted, false, "get the config of", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        Ok(config_namespace(state.config(), vm))
    }

    #[pyfunction]
    fn is_shareable(args: FuncArgs, vm: &VirtualMachine) -> PyResult<bool> {
        let parsed = ArgSpec {
            fname: "is_shareable",
            keywords: &["obj"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        Ok(crossinterp::is_shareable(parsed[0].as_deref().unwrap(), vm))
    }

    /// `_run_in_interpreter` for the script forms: `Ok(None)` on success,
    /// `Ok(Some(excinfo))` when the script raised.
    #[cfg(feature = "threading")]
    fn run_code_in(
        id: i64,
        code: PyRef<PyCode>,
        shared: Option<&Py<PyDict>>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let script = SharedValue::from_code(&code, vm)?;
        let shared_vals = shared
            .map(|dict| {
                let mut out = Vec::new();
                for (key, value) in dict {
                    let name = crossinterp::utf8_key(&key, vm)?.to_owned();
                    let val = SharedValue::from_object(&value, Fallback::XidataOnly, vm)?;
                    out.push((name, val));
                }
                PyResult::Ok(out)
            })
            .transpose()?;
        match crossinterp::with_interpreter(id, vm, |target| {
            let ns = target.main_namespace()?;
            if let Some(vals) = &shared_vals {
                for (name, val) in vals {
                    ns.set_item(name.as_str(), val.clone().into_object(target)?, target)?;
                }
            }
            let code = script
                .clone()
                .into_object(target)?
                .downcast::<PyCode>()
                .map_err(|_| target.new_type_error("expected code"))?;
            let scope = crate::scope::Scope::with_builtins(None, ns, target);
            match target.run_code_obj(code, scope) {
                Ok(_) => Ok(None),
                Err(exc) => Ok(Some(ExcInfo::capture(&exc, target))),
            }
        }) {
            Ok(None) => Ok(vm.ctx.none()),
            Ok(Some(info)) => Ok(info.into_namespace(vm)),
            Err(e) => Err(e),
        }
    }

    #[cfg(not(feature = "threading"))]
    fn run_code_in(
        _id: i64,
        _code: PyRef<PyCode>,
        _shared: Option<&Py<PyDict>>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Err(vm.new_runtime_error("isolated interpreters require threading"))
    }

    /// The shared `(id, <second>, shared=None, *, restrict=False)` prologue.
    /// `second_check` is the converter of the format string's second slot.
    fn script_args(
        args: &FuncArgs,
        func: &'static str,
        second_name: &'static str,
        second_check: fn(&PyObject, &VirtualMachine) -> PyResult<()>,
        op: &str,
        vm: &VirtualMachine,
    ) -> PyResult<(i64, PyObjectRef, Option<PyRef<PyDict>>)> {
        let parsed = ArgSpec {
            fname: func,
            keywords: &["id", second_name, "shared", "restrict"],
            required: 2,
            max_positional: 3,
        }
        .parse_with(
            args,
            |i, obj, vm| match i {
                1 => second_check(obj, vm),
                2 => check_dict(obj, func, "argument 3", vm),
                _ => Ok(()),
            },
            vm,
        )?;
        let restrict = flag(parsed[3].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restrict, true, op, vm)?;
        let shared = parsed[2].clone().map(|o| o.downcast::<PyDict>().unwrap());
        Ok((id, parsed[1].clone().unwrap(), shared))
    }

    #[pyfunction]
    fn exec(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.exec";
        // The code need not be "pure": globals resolve against __main__.
        let (id, code_obj, shared) =
            script_args(&args, FUNCNAME, "code", |_, _| Ok(()), "exec code for", vm)?;
        let code = crossinterp::script_code(&code_obj, vm)?;
        run_code_in(id, code, shared.as_deref(), vm)
    }

    #[pyfunction]
    fn run_string(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.run_string";
        let (id, script, shared) = script_args(
            &args,
            FUNCNAME,
            "script",
            |obj, vm| {
                // The `U` converter.
                obj.downcastable::<PyStr>()
                    .then_some(())
                    .ok_or_else(|| bad_argument(FUNCNAME, "argument 2", "str", obj, vm))
            },
            "run a string in",
            vm,
        )?;
        let code = crossinterp::script_code(&script, vm)?;
        run_code_in(id, code, shared.as_deref(), vm)
    }

    #[pyfunction]
    fn run_func(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.run_func";
        // Globals are not checked either; they resolve against __main__.
        let (id, func, shared) = script_args(
            &args,
            FUNCNAME,
            "func",
            |_, _| Ok(()),
            "run a function in",
            vm,
        )?;
        if !func.downcastable::<PyFunction>() && !func.downcastable::<PyCode>() {
            return Err(bad_argument(
                FUNCNAME,
                "argument 2",
                "a function",
                &func,
                vm,
            ));
        }
        let code = crossinterp::script_code(&func, vm)?;
        run_code_in(id, code, shared.as_deref(), vm)
    }

    #[pyfunction]
    fn call(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.call";
        let parsed = ArgSpec {
            fname: FUNCNAME,
            keywords: &[
                "id",
                "callable",
                "args",
                "kwargs",
                "preserve_exc",
                "restrict",
            ],
            required: 2,
            max_positional: 4,
        }
        .parse_with(
            &args,
            |i, obj, vm| match i {
                2 => obj
                    .downcastable::<crate::builtins::PyTuple>()
                    .then_some(())
                    .ok_or_else(|| bad_argument(FUNCNAME, "argument 3", "tuple", obj, vm)),
                3 => check_dict(obj, FUNCNAME, "argument 4", vm),
                _ => Ok(()),
            },
            vm,
        )?;
        // preserve_exc is accepted and ignored: an unpickled exception is
        // always a new object.
        let restrict = flag(parsed[5].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restrict, true, "make a call in", vm)?;

        let callable = parsed[1].as_deref().unwrap();
        if !callable.is_callable() {
            let repr = callable.repr(vm)?;
            return Err(vm.new_type_error(format!("expected a callable, got {repr}")));
        }
        let func = SharedValue::from_callable(callable, vm)?;
        let packed_args = match parsed[2].as_deref() {
            Some(o)
                if !o
                    .downcast_ref::<crate::builtins::PyTuple>()
                    .unwrap()
                    .is_empty() =>
            {
                Some(SharedValue::from_object(o, Fallback::Full, vm)?)
            }
            _ => None,
        };
        let packed_kwargs = match parsed[3].as_deref() {
            Some(o) if !o.downcast_ref::<PyDict>().unwrap().is_empty() => {
                Some(SharedValue::from_object(o, Fallback::Full, vm)?)
            }
            _ => None,
        };

        call_in(id, func, packed_args, packed_kwargs, vm)
    }

    /// What running the callable in the target interpreter produced.
    #[cfg(feature = "threading")]
    enum CallOutcome {
        Returned(SharedValue),
        Raised(ExcInfo),
        /// An argument could not be unpacked or the result has no
        /// cross-interpreter data; `msg` is what `wrap_notshareable` labelled it.
        NotShareable {
            info: ExcInfo,
            msg: Option<String>,
        },
    }

    #[cfg(feature = "threading")]
    fn call_in(
        id: i64,
        func: SharedValue,
        packed_args: Option<SharedValue>,
        packed_kwargs: Option<SharedValue>,
        vm: &VirtualMachine,
    ) -> PyResult {
        {
            let outcome = crossinterp::with_interpreter(id, vm, |target| {
                // _interp_call_unpack, whose failures are labelled by the part
                // that could not be rebuilt.
                let unpack = || -> Result<_, (PyBaseExceptionRef, &'static str)> {
                    let func = func.clone().into_object(target).map_err(|e| (e, "func"))?;
                    let call_args = match &packed_args {
                        Some(v) => v
                            .clone()
                            .into_object(target)
                            .map_err(|e| (e, "args"))?
                            .downcast::<crate::builtins::PyTuple>()
                            .map_err(|_| (target.new_type_error("expected tuple"), "args"))?
                            .as_slice()
                            .to_vec(),
                        None => Vec::new(),
                    };
                    let mut func_args = FuncArgs::from(call_args);
                    if let Some(v) = &packed_kwargs {
                        let dict = v
                            .clone()
                            .into_object(target)
                            .map_err(|e| (e, "kwargs"))?
                            .downcast::<PyDict>()
                            .map_err(|_| (target.new_type_error("expected dict"), "kwargs"))?;
                        for (key, value) in &*dict {
                            let name = crossinterp::utf8_key(&key, target)
                                .map_err(|e| (e, "kwargs"))?
                                .to_owned();
                            func_args.kwargs.insert(name.into(), value);
                        }
                    }
                    Ok((func, func_args))
                };
                let (func, func_args) = match unpack() {
                    Ok(unpacked) => unpacked,
                    Err((exc, label)) => {
                        return Ok(CallOutcome::NotShareable {
                            info: ExcInfo::capture(&exc, target),
                            msg: Some(format!("{label} not shareable")),
                        });
                    }
                };
                Ok(match func.call(func_args, target) {
                    Ok(res) => match SharedValue::from_object(&res, Fallback::Full, target) {
                        Ok(v) => CallOutcome::Returned(v),
                        Err(exc) => CallOutcome::NotShareable {
                            info: ExcInfo::capture(&exc, target),
                            msg: None,
                        },
                    },
                    Err(exc) => CallOutcome::Raised(ExcInfo::capture(&exc, target)),
                })
            })?;
            let (res, excinfo) = match outcome {
                CallOutcome::Returned(v) => (v.into_object(vm)?, vm.ctx.none()),
                CallOutcome::Raised(info) => (vm.ctx.none(), info.into_namespace(vm)),
                CallOutcome::NotShareable { info, msg } => {
                    return Err(info.into_not_shareable(msg, vm));
                }
            };
            Ok(vm.ctx.new_tuple(vec![res, excinfo]).into())
        }
    }

    #[cfg(not(feature = "threading"))]
    fn call_in(
        _id: i64,
        _func: SharedValue,
        _packed_args: Option<SharedValue>,
        _packed_kwargs: Option<SharedValue>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Err(vm.new_runtime_error("isolated interpreters require threading"))
    }

    #[pyfunction(name = "set___main___attrs")]
    fn set_main_attrs(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        const FUNCNAME: &str = "_interpreters.set___main___attrs";
        let parsed = ArgSpec {
            fname: FUNCNAME,
            keywords: &["id", "updates", "restrict"],
            required: 2,
            max_positional: 2,
        }
        .parse_with(
            &args,
            |i, obj, vm| match i {
                1 => check_dict(obj, FUNCNAME, "argument 2", vm),
                _ => Ok(()),
            },
            vm,
        )?;
        let restrict = flag(parsed[2].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restrict, true, "update __main__ for", vm)?;
        let updates = parsed[1]
            .as_deref()
            .unwrap()
            .downcast_ref::<PyDict>()
            .unwrap();
        if updates.is_empty() {
            return Err(vm.new_value_error("arg 2 must be a non-empty dict"));
        }
        let mut vals = Vec::new();
        for (key, value) in updates {
            let name = crossinterp::utf8_key(&key, vm)?.to_owned();
            let val = SharedValue::from_object(&value, Fallback::XidataOnly, vm)?;
            vals.push((name, val));
        }
        bind_main_attrs(id, vals, vm)
    }

    #[cfg(feature = "threading")]
    fn bind_main_attrs(
        id: i64,
        vals: Vec<(String, SharedValue)>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        crossinterp::with_interpreter(id, vm, |target| {
            let ns = target.main_namespace()?;
            for (name, val) in &vals {
                ns.set_item(name.as_str(), val.clone().into_object(target)?, target)?;
            }
            Ok(())
        })
    }

    #[cfg(not(feature = "threading"))]
    fn bind_main_attrs(
        _id: i64,
        _vals: Vec<(String, SharedValue)>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        Err(vm.new_runtime_error("isolated interpreters require threading"))
    }

    #[pyfunction]
    fn incref(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = ArgSpec {
            fname: "incref",
            keywords: &["id", "implieslink", "restrict"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let implieslink = flag(parsed[1].as_ref(), vm)?;
        let restricted = flag(parsed[2].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restricted, true, "incref", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        if implieslink {
            // Decref to 0 will destroy the interpreter.
            state.require_idref.store(true, Ordering::Release);
        }
        state.id_refcount.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    #[pyfunction]
    fn decref(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let parsed = ArgSpec {
            fname: "decref",
            keywords: &["id", "restrict"],
            required: 1,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let restricted = flag(parsed[1].as_ref(), vm)?;
        let id = parse_id(parsed[0].as_deref().unwrap(), vm)?;
        resolve_interp(Some(id), restricted, true, "decref", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        let prev = state.id_refcount.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 && state.require_idref.load(Ordering::Acquire) {
            #[cfg(feature = "threading")]
            {
                if runtime::is_owned_interpreter(id) && id != vm.state.interpreter_id {
                    let _ = runtime::destroy_owned_interpreter(id);
                }
            }
        }
        Ok(())
    }

    #[pyfunction]
    fn capture_exception(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let parsed = ArgSpec {
            fname: "capture_exception",
            keywords: &["exc"],
            required: 0,
            max_positional: 1,
        }
        .parse(&args, vm)?;
        let exc = match parsed
            .into_iter()
            .next()
            .unwrap()
            .filter(|e| !vm.is_none(e))
        {
            Some(e) => e
                .downcast::<crate::builtins::PyBaseException>()
                .map_err(|e| {
                    let repr = e
                        .repr(vm)
                        .map_or_else(|_| e.class().name().to_string(), |s| s.to_string());
                    vm.new_type_error(format!("expected exception, got {repr}"))
                })?,
            // `PyErr_GetRaisedException`: an exception is in flight only while
            // no Python code is running, so there is never one to capture here.
            None => return Ok(vm.ctx.none()),
        };
        Ok(ExcInfo::capture(&exc, vm).into_namespace(vm))
    }
}
