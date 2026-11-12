//! Low-level multiple-interpreter primitives (`_interpreters`).
//!
//! Mirrors CPython `Modules/_interpretersmodule.c`.

pub(crate) use _interpreters::{config_from_pyobject, init_xi_types, module_def, parse_id};
#[cfg_attr(not(feature = "threading"), allow(unused_imports))]
pub(crate) use _interpreters::{
    interpreter_error, interpreter_not_found, not_shareable_error, xibufferview_from_buffer,
};

#[pymodule]
pub(crate) mod _interpreters {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyCode, PyDict, PyException, PyFunction, PyModule, PyStr, PyTuple,
        },
        class::PyClassImpl,
        function::{
            ArgIntoBool, ArgumentError, DefaultRepr, FromArgs, FuncArgs, OptionalArg, Param,
            ParamKind,
        },
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
    pub(crate) fn parse_id(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
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
    pub(crate) fn config_from_pyobject(
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<InterpreterConfig> {
        parse_config(Some(obj), vm)
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

    struct NewConfigArgs(FuncArgs);

    impl FromArgs for NewConfigArgs {
        // CPython uses a bespoke positional-count error before reading overrides.
        const BINDING_PARAMS: Option<&'static [Param]> = None;

        const PARAMS: Option<&'static [Param]> = Some(&[
            Param {
                name: "name",
                kind: ParamKind::PositionalOnly,
                default: Some(DefaultRepr::Str("isolated")),
            },
            Param::var_keyword("overrides"),
        ]);

        fn from_args(_vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            Ok(Self(core::mem::take(args)))
        }
    }

    // These match the Clinic converters and their order.
    // IDs stay objects until the implementation resolves them, after flags.
    #[derive(FromArgs)]
    struct ListAllArgs {
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        require_ready: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct CreateArgs<'a> {
        #[pyarg(any, optional, py_default = "'isolated'")]
        config: OptionalArg<&'a PyObject>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        reqrefs: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct InterpreterIdArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct WhenceArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct ShareableArgs<'a> {
        #[pyarg(any)]
        obj: &'a PyObject,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct ExecArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(any)]
        code: &'a PyObject,
        #[pyarg(any, optional, py_default = "{}")]
        shared: OptionalArg<&'a Py<PyDict>>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct RunStringArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(any)]
        script: &'a Py<PyStr>,
        #[pyarg(any, optional, py_default = "{}")]
        shared: OptionalArg<&'a Py<PyDict>>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct RunFuncArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(any)]
        func: &'a PyObject,
        #[pyarg(any, optional, py_default = "{}")]
        shared: OptionalArg<&'a Py<PyDict>>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct CallArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(any)]
        callable: &'a PyObject,
        #[pyarg(any, optional, py_default = "()")]
        args: OptionalArg<&'a Py<PyTuple>>,
        #[pyarg(any, optional, py_default = "{}")]
        kwargs: OptionalArg<&'a Py<PyDict>>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        // Clinic converts this flag even though the implementation ignores it.
        preserve_exc: ArgIntoBool,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct SetMainAttrsArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(any)]
        updates: &'a Py<PyDict>,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct IncrefArgs<'a> {
        #[pyarg(any)]
        id: &'a PyObject,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        implieslink: ArgIntoBool,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        restrict: ArgIntoBool,
    }

    #[derive(FromArgs)]
    #[from_args(borrowed)]
    struct CaptureExceptionArgs<'a> {
        #[pyarg(any, optional, py_default = "None")]
        exc: OptionalArg<&'a PyObject>,
    }

    #[pyfunction]
    fn new_config(NewConfigArgs(args): NewConfigArgs, vm: &VirtualMachine) -> PyResult {
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
    fn create(args: CreateArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let requires_refs = args.reqrefs.into_bool();
        let config = parse_config(args.config.into_option(), vm)?;
        new_interpreter(config, requires_refs, vm)
    }

    /// `_PyXI_NewInterpreter`, registered as a runtime-owned interpreter.
    #[cfg(feature = "threading")]
    fn new_interpreter(config: InterpreterConfig, reqrefs: bool, vm: &VirtualMachine) -> PyResult {
        let interp =
            crate::Interpreter::create_subinterpreter_from_vm(vm, config).map_err(|msg| {
                let cause = vm.new_runtime_error(msg.to_owned());
                let exc = interpreter_error(vm, "interpreter creation failed");
                exc.set_context(Some(cause));
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
    fn destroy(args: InterpreterIdArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
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
    fn list_all(args: ListAllArgs, vm: &VirtualMachine) -> PyObjectRef {
        let reqready = args.require_ready.into_bool();
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
        vm.ctx.new_list(items).into()
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
    fn is_running(args: InterpreterIdArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "check if running for", vm)?;
        Ok(vm.ctx.new_bool(crossinterp::is_running(id)).into())
    }

    #[pyfunction]
    fn whence(args: WhenceArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let id = parse_id(args.id, vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        Ok(vm.ctx.new_int(state.whence.as_i32()).into())
    }

    #[pyfunction]
    fn get_config(args: InterpreterIdArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let restricted = args.restrict.into_bool();
        let id_obj = args.id;
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
    fn is_shareable(args: ShareableArgs<'_>, vm: &VirtualMachine) -> bool {
        crossinterp::is_shareable(args.obj, vm)
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

    #[pyfunction]
    fn exec(args: ExecArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "exec code for", vm)?;
        let code = crossinterp::script_code(args.code, vm)?;
        run_code_in(id, code, args.shared.into_option(), vm)
    }

    #[pyfunction]
    fn run_string(args: RunStringArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "run a string in", vm)?;
        let code = crossinterp::script_code(args.script.as_object(), vm)?;
        run_code_in(id, code, args.shared.into_option(), vm)
    }

    #[pyfunction]
    fn run_func(args: RunFuncArgs<'_>, vm: &VirtualMachine) -> PyResult {
        const FUNCNAME: &str = "_interpreters.run_func";
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "run a function in", vm)?;
        if !args.func.downcastable::<PyFunction>() && !args.func.downcastable::<PyCode>() {
            return Err(bad_argument(
                FUNCNAME,
                "argument 2",
                "a function",
                args.func,
                vm,
            ));
        }
        let code = crossinterp::script_code(args.func, vm)?;
        run_code_in(id, code, args.shared.into_option(), vm)
    }

    #[pyfunction]
    fn call(args: CallArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let _ = args.preserve_exc;
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "make a call in", vm)?;

        let callable = args.callable;
        if !callable.is_callable() {
            let repr = callable.repr(vm)?;
            return Err(vm.new_type_error(format!("expected a callable, got {repr}")));
        }
        let func = SharedValue::from_callable(callable, vm)?;
        let packed_args = match args.args.as_ref().into_option() {
            Some(o) if !o.as_slice().is_empty() => {
                Some(SharedValue::from_object(o.as_object(), Fallback::Full, vm)?)
            }
            _ => None,
        };
        let packed_kwargs = match args.kwargs.as_ref().into_option() {
            Some(o) if !o.is_empty() => {
                Some(SharedValue::from_object(o.as_object(), Fallback::Full, vm)?)
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
    fn set_main_attrs(args: SetMainAttrsArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "update __main__ for", vm)?;
        let updates = args.updates;
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
    fn incref(args: IncrefArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let implies_link = args.implieslink.into_bool();
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
        resolve_interp(Some(id), restricted, true, "incref", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        if implies_link {
            // Decref to 0 will destroy the interpreter.
            state.require_idref.store(true, Ordering::Release);
        }
        state.id_refcount.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    #[pyfunction]
    fn decref(args: InterpreterIdArgs<'_>, vm: &VirtualMachine) -> PyResult<()> {
        let restricted = args.restrict.into_bool();
        let id = parse_id(args.id, vm)?;
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
    fn capture_exception(args: CaptureExceptionArgs<'_>, vm: &VirtualMachine) -> PyResult {
        let exc = match args.exc.into_option().filter(|exc| !vm.is_none(exc)) {
            Some(e) => e
                .downcast_ref::<crate::builtins::PyBaseException>()
                .ok_or_else(|| {
                    let repr = e
                        .repr(vm)
                        .map_or_else(|_| e.class().name().to_string(), |s| s.to_string());
                    vm.new_type_error(format!("expected exception, got {repr}"))
                })?,
            // `PyErr_GetRaisedException`: an exception is in flight only while
            // no Python code is running, so there is never one to capture here.
            None => return Ok(vm.ctx.none()),
        };
        Ok(ExcInfo::capture(exc, vm).into_namespace(vm))
    }
}
