//! Low-level multiple-interpreter primitives (`_interpreters`).
//!
//! Mirrors CPython `Modules/_interpretersmodule.c`.

pub(crate) use _interpreters::module_def;
#[cfg_attr(not(feature = "threading"), allow(unused_imports))]
pub(crate) use _interpreters::{interpreter_error, interpreter_not_found, not_shareable_error};

#[pymodule]
pub(crate) mod _interpreters {
    use crate::{
        AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyCode, PyDict, PyException, PyFunction, PyInt, PyStr},
        function::{FuncArgs, OptionalArg},
        types::Constructor,
        vm::{
            InterpreterConfig, InterpreterWhence,
            crossinterp::{self, ExcInfo, SharedValue},
            runtime,
        },
    };
    use core::sync::atomic::Ordering;

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
    #[pyexception(name = "InterpreterError", base = PyException)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyInterpreterError(PyException);

    #[pyexception]
    impl PyInterpreterError {}

    #[pyattr]
    #[pyexception(name = "InterpreterNotFoundError", base = PyInterpreterError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyInterpreterNotFoundError(PyInterpreterError);

    #[pyexception]
    impl PyInterpreterNotFoundError {}

    #[pyattr]
    #[pyexception(name = "NotShareableError", base = crate::exceptions::types::PyTypeError)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyNotShareableError(crate::exceptions::types::PyTypeError);

    #[pyexception]
    impl PyNotShareableError {}

    pub(crate) fn interpreter_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyInterpreterError::class(&vm.ctx).to_owned(),
            msg.into().into(),
        )
    }

    pub(crate) fn interpreter_not_found(
        vm: &VirtualMachine,
        id: i64,
    ) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyInterpreterNotFoundError::class(&vm.ctx).to_owned(),
            format!("interpreter {id} not found").into(),
        )
    }

    pub(crate) fn not_shareable_error(
        vm: &VirtualMachine,
        msg: impl Into<String>,
    ) -> crate::builtins::PyBaseExceptionRef {
        vm.new_exception_msg(
            PyNotShareableError::class(&vm.ctx).to_owned(),
            msg.into().into(),
        )
    }

    fn parse_id(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i64> {
        let Some(n) = obj.downcast_ref::<PyInt>() else {
            return Err(vm.new_type_error(format!(
                "interpreter ID must be an int, got {}",
                obj.class()
            )));
        };
        let id = n.try_to_primitive::<i64>(vm)?;
        if id < 0 {
            return Err(vm.new_value_error(format!(
                "interpreter ID must be a non-negative int, got {id}"
            )));
        }
        Ok(id)
    }

    fn resolve_interp(
        id: i64,
        restricted: bool,
        reqready: bool,
        op: &str,
        vm: &VirtualMachine,
    ) -> PyResult<runtime::InterpreterInfo> {
        let info = runtime::list_interpreters()
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| interpreter_not_found(vm, id))?;
        if reqready {
            let state =
                runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
            if !state.ready.load(Ordering::Acquire) {
                return Err(interpreter_error(
                    vm,
                    format!("cannot {op} interpreter {id} (not ready)"),
                ));
            }
        }
        if restricted && info.whence != InterpreterWhence::Stdlib {
            return Err(interpreter_error(
                vm,
                format!("cannot {op} unrecognized interpreter {id}"),
            ));
        }
        Ok(info)
    }

    fn require_dict<'a>(
        obj: &'a PyObject,
        func: &str,
        pos: i32,
        vm: &'a VirtualMachine,
    ) -> PyResult<&'a crate::Py<PyDict>> {
        obj.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error(format!(
                "{func} argument {pos} must be dict, not {}",
                obj.class()
            ))
        })
        // downcast_ref already returns Option<&Py<PyDict>>
    }

    fn parse_config(obj: Option<&PyObject>, vm: &VirtualMachine) -> PyResult<InterpreterConfig> {
        let Some(obj) = obj else {
            return Ok(InterpreterConfig::ISOLATED);
        };
        if vm.is_none(obj) {
            return Ok(InterpreterConfig::ISOLATED);
        }
        if let Some(s) = obj.downcast_ref::<PyStr>() {
            let name = s.to_string();
            return InterpreterConfig::named(&name)
                .ok_or_else(|| vm.new_value_error(format!("unsupported config name '{name}'")));
        }
        // Namespace / object with attributes.
        let mut cfg = InterpreterConfig::ISOLATED;
        let get_bool = |name: &'static str| -> PyResult<Option<bool>> {
            match obj.get_attr(name, vm) {
                Ok(v) => Ok(Some(v.is_true(vm)?)),
                Err(_) => {
                    let _ = vm.pop_exception();
                    Ok(None)
                }
            }
        };
        if let Some(v) = get_bool("use_main_obmalloc")? {
            cfg.use_main_obmalloc = v;
        }
        if let Some(v) = get_bool("allow_fork")? {
            cfg.allow_fork = v;
        }
        if let Some(v) = get_bool("allow_exec")? {
            cfg.allow_exec = v;
        }
        if let Some(v) = get_bool("allow_threads")? {
            cfg.allow_threads = v;
        }
        if let Some(v) = get_bool("allow_daemon_threads")? {
            cfg.allow_daemon_threads = v;
        }
        if let Some(v) = get_bool("check_multi_interp_extensions")? {
            cfg.check_multi_interp_extensions = v;
        }
        Ok(cfg)
    }

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
    fn create(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let config_obj = args.args.first().cloned();
        let reqrefs = args
            .kwargs
            .get("reqrefs")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let config = parse_config(config_obj.as_deref(), vm)?;
        #[cfg(feature = "threading")]
        {
            let interp = crate::Interpreter::create_subinterpreter_from_vm(vm, config);
            if reqrefs {
                interp
                    .global_state
                    .require_idref
                    .store(true, Ordering::Release);
                interp
                    .global_state
                    .id_refcount
                    .fetch_add(1, Ordering::AcqRel);
            }
            let id = runtime::store_owned_interpreter(interp);
            return Ok(vm.ctx.new_int(id).into());
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = (config, reqrefs);
            Err(vm.new_runtime_error("isolated interpreters require threading"))
        }
    }

    #[pyfunction]
    fn destroy(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let id_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("destroy() missing required argument: 'id'"))?;
        let restricted = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let id = parse_id(id_obj, vm)?;
        resolve_interp(id, restricted, false, "destroy", vm)?;
        if id == vm.state.interpreter_id {
            return Err(interpreter_error(
                vm,
                "cannot destroy the current interpreter",
            ));
        }
        if crossinterp::is_running(id)
            && !runtime::lookup_interpreter(id).is_some_and(|s| s.is_main)
        {
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
        let reqready = args
            .kwargs
            .get("require_ready")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let mut items = Vec::new();
        for info in runtime::list_interpreters() {
            if reqready {
                let ready = runtime::lookup_interpreter(info.id)
                    .is_some_and(|s| s.ready.load(Ordering::Acquire));
                if !ready {
                    continue;
                }
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
        let id_obj = args
            .args
            .first()
            .ok_or_else(|| vm.new_type_error("is_running() missing required argument: 'id'"))?;
        let restricted = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let id = parse_id(id_obj, vm)?;
        resolve_interp(id, restricted, true, "check if running for", vm)?;
        Ok(vm.ctx.new_bool(crossinterp::is_running(id)).into())
    }

    #[pyfunction]
    fn whence(id: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let id = parse_id(&id, vm)?;
        let info = runtime::list_interpreters()
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| interpreter_not_found(vm, id))?;
        Ok(vm.ctx.new_int(info.whence.as_i32()).into())
    }

    #[pyfunction]
    fn get_config(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let id_obj = args.args.first();
        let restricted = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let id = match id_obj {
            Some(o) if vm.is_none(o) => vm.state.interpreter_id,
            Some(o) => parse_id(o, vm)?,
            None => vm.state.interpreter_id,
        };
        resolve_interp(id, restricted, false, "get the config of", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        let flags = state.feature_flags;
        let cfg = InterpreterConfig {
            use_main_obmalloc: false,
            allow_fork: flags.allow_fork,
            allow_exec: flags.allow_exec,
            allow_threads: flags.allow_threads,
            allow_daemon_threads: flags.allow_daemon_threads,
            check_multi_interp_extensions: flags.check_multi_interp_extensions,
            gil: if flags.allow_fork {
                crate::vm::InterpreterGil::Shared
            } else {
                crate::vm::InterpreterGil::Own
            },
        };
        Ok(config_namespace(cfg, vm))
    }

    #[pyfunction]
    fn new_config(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let name = match args.args.first() {
            Some(o) => o
                .downcast_ref::<PyStr>()
                .map_or_else(|| "isolated".to_owned(), |s| s.to_string()),
            None => "isolated".to_owned(),
        };
        let mut cfg = InterpreterConfig::named(&name)
            .ok_or_else(|| vm.new_value_error(format!("unsupported config name '{name}'")))?;
        if let Some(v) = args.kwargs.get("allow_fork") {
            cfg.allow_fork = v.clone().is_true(vm)?;
        }
        if let Some(v) = args.kwargs.get("allow_exec") {
            cfg.allow_exec = v.clone().is_true(vm)?;
        }
        if let Some(v) = args.kwargs.get("allow_threads") {
            cfg.allow_threads = v.clone().is_true(vm)?;
        }
        if let Some(v) = args.kwargs.get("allow_daemon_threads") {
            cfg.allow_daemon_threads = v.clone().is_true(vm)?;
        }
        if let Some(v) = args.kwargs.get("use_main_obmalloc") {
            cfg.use_main_obmalloc = v.clone().is_true(vm)?;
        }
        if let Some(v) = args.kwargs.get("check_multi_interp_extensions") {
            cfg.check_multi_interp_extensions = v.clone().is_true(vm)?;
        }
        Ok(config_namespace(cfg, vm))
    }

    #[pyfunction]
    fn is_shareable(obj: PyObjectRef, vm: &VirtualMachine) -> bool {
        crossinterp::is_shareable(&obj, vm)
    }

    fn run_code_in(
        id: i64,
        code: PyRef<PyCode>,
        shared: Option<&PyObject>,
        restrict: bool,
        func_name: &str,
        vm: &VirtualMachine,
    ) -> PyResult {
        resolve_interp(id, restrict, true, "exec code for", vm)?;
        #[cfg(feature = "threading")]
        {
            let shared_vals = if let Some(shared) = shared {
                let dict = require_dict(shared, func_name, 3, vm)?;
                let mut out = Vec::new();
                for (key, value) in dict {
                    let name = crossinterp::utf8_key(&key, vm)?.to_owned();
                    let val = SharedValue::from_object(&value, vm)?;
                    out.push((name, val));
                }
                Some(out)
            } else {
                None
            };
            return match crossinterp::with_interpreter(id, vm, |target| {
                if let Some(vals) = &shared_vals {
                    let ns = target.main_namespace()?;
                    for (name, val) in vals {
                        ns.set_item(name.as_str(), val.clone().into_object(target)?, target)?;
                    }
                }
                let ns = target.main_namespace()?;
                let scope = crate::scope::Scope::with_builtins(None, ns, target);
                match target.run_code_obj(code.clone(), scope) {
                    Ok(_) => Ok(None),
                    Err(exc) => Ok(Some(ExcInfo::capture(&exc, target))),
                }
            }) {
                Ok(None) => Ok(vm.ctx.none()),
                Ok(Some(info)) => Ok(info.into_namespace(vm)),
                Err(e) => Err(e),
            };
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = (code, shared, func_name);
            Err(vm.new_runtime_error("isolated interpreters require threading"))
        }
    }

    #[pyfunction]
    fn exec(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let id = parse_id(
            args.args
                .first()
                .ok_or_else(|| vm.new_type_error("_interpreters.exec() missing argument 1"))?,
            vm,
        )?;
        let code_obj = args
            .args
            .get(1)
            .ok_or_else(|| vm.new_type_error("_interpreters.exec() missing argument 2"))?;
        let shared = args.args.get(2).or_else(|| args.kwargs.get("shared"));
        if let Some(s) = shared {
            require_dict(s, "_interpreters.exec()", 3, vm)?;
        }
        let restrict = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let code = crossinterp::script_code(code_obj, vm)?;
        run_code_in(
            id,
            code,
            shared.map(|o| o.as_ref()),
            restrict,
            "_interpreters.exec",
            vm,
        )
    }

    #[pyfunction]
    fn run_string(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let id = parse_id(
            args.args.first().ok_or_else(|| {
                vm.new_type_error("_interpreters.run_string() missing argument 1")
            })?,
            vm,
        )?;
        let script = args
            .args
            .get(1)
            .ok_or_else(|| vm.new_type_error("_interpreters.run_string() missing argument 2"))?;
        if script.downcast_ref::<PyStr>().is_none() {
            return Err(vm.new_type_error(format!(
                "_interpreters.run_string() argument 2 must be a string, not {}",
                script.class()
            )));
        }
        let shared = args.args.get(2).or_else(|| args.kwargs.get("shared"));
        if let Some(s) = shared {
            require_dict(s, "_interpreters.run_string()", 3, vm)?;
        }
        let restrict = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let code = crossinterp::script_code(script, vm)?;
        run_code_in(
            id,
            code,
            shared.map(|o| o.as_ref()),
            restrict,
            "_interpreters.run_string",
            vm,
        )
    }

    #[pyfunction]
    fn run_func(args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let id = parse_id(
            args.args
                .first()
                .ok_or_else(|| vm.new_type_error("_interpreters.run_func() missing argument 1"))?,
            vm,
        )?;
        let func = args
            .args
            .get(1)
            .ok_or_else(|| vm.new_type_error("_interpreters.run_func() missing argument 2"))?;
        if func.downcast_ref::<PyFunction>().is_none() && func.downcast_ref::<PyCode>().is_none() {
            return Err(vm.new_type_error(format!(
                "_interpreters.run_func() argument 2 must be a function, not {}",
                func.class()
            )));
        }
        let shared = args.args.get(2).or_else(|| args.kwargs.get("shared"));
        if let Some(s) = shared {
            require_dict(s, "_interpreters.run_func()", 3, vm)?;
        }
        let restrict = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let code = crossinterp::script_code(func, vm)?;
        run_code_in(
            id,
            code,
            shared.map(|o| o.as_ref()),
            restrict,
            "_interpreters.run_func",
            vm,
        )
    }

    #[pyfunction(name = "set___main___attrs")]
    fn set_main_attrs(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let id = parse_id(
            args.args.first().ok_or_else(|| {
                vm.new_type_error("_interpreters.set___main___attrs() missing argument 1")
            })?,
            vm,
        )?;
        let updates = args.args.get(1).ok_or_else(|| {
            vm.new_type_error("_interpreters.set___main___attrs() missing argument 2")
        })?;
        require_dict(updates, "_interpreters.set___main___attrs()", 2, vm)?;
        if updates
            .downcast_ref::<PyDict>()
            .is_some_and(|d| d.is_empty())
        {
            return Err(vm.new_value_error("arg 2 must be a non-empty dict"));
        }
        let restrict = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        resolve_interp(id, restrict, true, "update __main__ for", vm)?;
        let dict = updates.downcast_ref::<PyDict>().unwrap();
        let mut vals = Vec::new();
        for (key, value) in dict {
            let name = crossinterp::utf8_key(&key, vm)?.to_owned();
            let val = SharedValue::from_object(&value, vm)?;
            vals.push((name, val));
        }
        #[cfg(feature = "threading")]
        {
            return crossinterp::with_interpreter(id, vm, |target| {
                let ns = target.main_namespace()?;
                for (name, val) in &vals {
                    ns.set_item(name.as_str(), val.clone().into_object(target)?, target)?;
                }
                Ok(())
            });
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = vals;
            Err(vm.new_runtime_error("isolated interpreters require threading"))
        }
    }

    #[pyfunction]
    fn incref(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let id = parse_id(
            args.args
                .first()
                .ok_or_else(|| vm.new_type_error("incref() missing argument 1"))?,
            vm,
        )?;
        let restricted = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        let implieslink = args
            .kwargs
            .get("implieslink")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        resolve_interp(id, restricted, true, "incref", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        if implieslink {
            state.require_idref.store(true, Ordering::Release);
        }
        state.id_refcount.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    #[pyfunction]
    fn decref(args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        let id = parse_id(
            args.args
                .first()
                .ok_or_else(|| vm.new_type_error("decref() missing argument 1"))?,
            vm,
        )?;
        let restricted = args
            .kwargs
            .get("restrict")
            .map(|o| o.clone().is_true(vm))
            .transpose()?
            .unwrap_or(false);
        resolve_interp(id, restricted, true, "decref", vm)?;
        let state = runtime::lookup_interpreter(id).ok_or_else(|| interpreter_not_found(vm, id))?;
        let prev = state.id_refcount.fetch_sub(1, Ordering::AcqRel);
        if prev <= 1 && state.require_idref.load(Ordering::Acquire) {
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
    fn capture_exception(exc: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let exc = match exc.into_option() {
            Some(e) if !vm.is_none(&e) => e
                .downcast::<crate::builtins::PyBaseException>()
                .map_err(|_| vm.new_type_error("expected exception"))?,
            _ => {
                return Ok(vm.ctx.none());
            }
        };
        Ok(ExcInfo::capture(&exc, vm).into_namespace(vm))
    }
}
