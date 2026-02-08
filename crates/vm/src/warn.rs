use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyResult, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyDictRef, PyListRef, PyStr, PyStrInterned, PyStrRef, PyTuple,
        PyTupleRef, PyTypeRef,
    },
    convert::TryFromObject,
};
use core::sync::atomic::{AtomicUsize, Ordering};
use rustpython_common::lock::OnceCell;

pub struct WarningsState {
    pub filters: PyListRef,
    pub once_registry: PyDictRef,
    pub default_action: PyStrRef,
    pub filters_version: AtomicUsize,
    pub context_var: OnceCell<PyObjectRef>,
    lock_count: AtomicUsize,
}

impl WarningsState {
    fn create_default_filters(ctx: &Context) -> PyListRef {
        // init_filters(): non-debug default filter set.
        ctx.new_list(vec![
            ctx.new_tuple(vec![
                ctx.new_str("default").into(),
                ctx.none(),
                ctx.exceptions.deprecation_warning.as_object().to_owned(),
                ctx.new_str("__main__").into(),
                ctx.new_int(0).into(),
            ])
            .into(),
            ctx.new_tuple(vec![
                ctx.new_str("ignore").into(),
                ctx.none(),
                ctx.exceptions.deprecation_warning.as_object().to_owned(),
                ctx.none(),
                ctx.new_int(0).into(),
            ])
            .into(),
            ctx.new_tuple(vec![
                ctx.new_str("ignore").into(),
                ctx.none(),
                ctx.exceptions
                    .pending_deprecation_warning
                    .as_object()
                    .to_owned(),
                ctx.none(),
                ctx.new_int(0).into(),
            ])
            .into(),
            ctx.new_tuple(vec![
                ctx.new_str("ignore").into(),
                ctx.none(),
                ctx.exceptions.import_warning.as_object().to_owned(),
                ctx.none(),
                ctx.new_int(0).into(),
            ])
            .into(),
            ctx.new_tuple(vec![
                ctx.new_str("ignore").into(),
                ctx.none(),
                ctx.exceptions.resource_warning.as_object().to_owned(),
                ctx.none(),
                ctx.new_int(0).into(),
            ])
            .into(),
        ])
    }

    pub fn init_state(ctx: &Context) -> Self {
        Self {
            filters: Self::create_default_filters(ctx),
            once_registry: ctx.new_dict(),
            default_action: ctx.new_str("default"),
            filters_version: AtomicUsize::new(0),
            context_var: OnceCell::new(),
            lock_count: AtomicUsize::new(0),
        }
    }

    pub fn acquire_lock(&self) {
        self.lock_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn release_lock(&self) -> bool {
        let prev = self.lock_count.load(Ordering::SeqCst);
        if prev == 0 {
            return false;
        }
        self.lock_count.fetch_sub(1, Ordering::SeqCst);
        true
    }

    pub fn filters_mutated(&self) {
        self.filters_version.fetch_add(1, Ordering::SeqCst);
    }
}

/// None matches everything; plain strings do exact comparison;
/// regex objects use .match().
fn check_matched(obj: &PyObject, arg: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
    if vm.is_none(obj) {
        return Ok(true);
    }
    if obj.class().is(vm.ctx.types.str_type) {
        return obj.rich_compare_bool(arg, crate::types::PyComparisonOp::Eq, vm);
    }
    let result = vm.call_method(obj, "match", (arg.to_owned(),))?;
    result.is_true(vm)
}

fn get_warnings_attr(
    vm: &VirtualMachine,
    attr_name: &'static PyStrInterned,
    try_import: bool,
) -> PyResult<Option<PyObjectRef>> {
    let module = if try_import
        && !vm
            .state
            .finalizing
            .load(core::sync::atomic::Ordering::SeqCst)
    {
        match vm.import("warnings", 0) {
            Ok(module) => module,
            Err(_) => return Ok(None),
        }
    } else {
        match vm.sys_module.get_attr(identifier!(vm, modules), vm) {
            Ok(modules) => match modules.get_item(vm.ctx.intern_str("warnings"), vm) {
                Ok(module) => module,
                Err(_) => return Ok(None),
            },
            Err(_) => return Ok(None),
        }
    };
    match module.get_attr(attr_name, vm) {
        Ok(attr) => Ok(Some(attr)),
        Err(_) => Ok(None),
    }
}

/// Get the warnings filters list from sys.modules['warnings'].filters,
/// falling back to vm.state.warnings.filters.
fn get_warnings_filters(vm: &VirtualMachine) -> PyResult<PyListRef> {
    if let Some(filters_obj) = get_warnings_attr(vm, identifier!(&vm.ctx, filters), false)?
        && let Ok(filters) = filters_obj.try_into_value::<PyListRef>(vm)
    {
        return Ok(filters);
    }
    Ok(vm.state.warnings.filters.clone())
}

/// Get the default action from sys.modules['warnings']._defaultaction,
/// falling back to vm.state.warnings.default_action.
fn get_default_action(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    if let Some(action) = get_warnings_attr(vm, identifier!(&vm.ctx, defaultaction), false)? {
        if !action.class().is(vm.ctx.types.str_type) {
            return Err(vm.new_type_error(format!(
                "_warnings.defaultaction must be a string, not '{}'",
                action.class().name()
            )));
        }
        return Ok(action);
    }
    Ok(vm.state.warnings.default_action.clone().into())
}

/// Get the once registry from sys.modules['warnings']._onceregistry,
/// falling back to vm.state.warnings.once_registry.
fn get_once_registry(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    if let Some(registry) = get_warnings_attr(vm, identifier!(&vm.ctx, onceregistry), false)? {
        if !registry.class().is(vm.ctx.types.dict_type) {
            return Err(vm.new_type_error(format!(
                "_warnings.onceregistry must be a dict, not '{}'",
                registry.class().name()
            )));
        }
        return Ok(registry);
    }
    Ok(vm.state.warnings.once_registry.clone().into())
}

fn already_warned(
    registry: &PyObject,
    key: PyObjectRef,
    should_set: bool,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    if vm.is_none(registry) {
        return Ok(false);
    }

    let current_version = vm.state.warnings.filters_version.load(Ordering::SeqCst);
    let version_obj = registry.get_item(identifier!(&vm.ctx, version), vm).ok();

    let version_matches = version_obj.as_ref().is_some_and(|v| {
        v.try_int(vm)
            .map(|i| i.as_u32_mask() as usize == current_version)
            .unwrap_or(false)
    });

    if version_matches {
        if let Ok(val) = registry.get_item(key.as_ref(), vm)
            && val.is_true(vm)?
        {
            return Ok(true);
        }
    } else if let Ok(dict) = PyDictRef::try_from_object(vm, registry.to_owned()) {
        dict.clear();
        dict.set_item(
            identifier!(&vm.ctx, version),
            vm.ctx.new_int(current_version).into(),
            vm,
        )?;
    }

    if should_set {
        registry.set_item(key.as_ref(), vm.ctx.true_value.clone().into(), vm)?;
    }
    Ok(false)
}

/// Create a `(text, category)` or `(text, category, 0)` key and record
/// it in the registry via `already_warned`.
fn update_registry(
    registry: &PyObject,
    text: &PyObject,
    category: &PyObject,
    add_zero: bool,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let altkey: PyObjectRef = if add_zero {
        PyTuple::new_ref(
            vec![
                text.to_owned(),
                category.to_owned(),
                vm.ctx.new_int(0).into(),
            ],
            &vm.ctx,
        )
        .into()
    } else {
        PyTuple::new_ref(vec![text.to_owned(), category.to_owned()], &vm.ctx).into()
    };
    already_warned(registry, altkey, true, vm)
}

fn normalize_module(filename: &Py<PyStr>, vm: &VirtualMachine) -> PyObjectRef {
    match filename.byte_len() {
        0 => vm.new_pyobj("<unknown>"),
        len if len >= 3 && filename.as_bytes().ends_with(b".py") => {
            vm.new_pyobj(&filename.as_wtf8()[..len - 3])
        }
        _ => filename.as_object().to_owned(),
    }
}

/// Search the global filters list for a matching action.
// TODO: split into filter_search() + get_filter() and support
//       context-aware filters (get_warnings_context_filters).
fn get_filter(
    category: PyObjectRef,
    text: PyObjectRef,
    lineno: usize,
    module: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    let filters = get_warnings_filters(vm)?;

    // filters could change while we are iterating over it.
    // Re-check list length each iteration (matches C behavior).
    let mut i = 0;
    while i < filters.borrow_vec().len() {
        let Some(tmp_item) = filters.borrow_vec().get(i).cloned() else {
            break;
        };
        let tmp_item = PyTupleRef::try_from_object(vm, tmp_item)
            .ok()
            .filter(|t| t.len() == 5)
            .ok_or_else(|| {
                vm.new_value_error(format!("_warnings.filters item {i} isn't a 5-tuple"))
            })?;

        /* action, msg, cat, mod, ln = item */
        let action = &tmp_item[0];
        let good_msg = check_matched(&tmp_item[1], &text, vm)?;
        let is_subclass = category.is_subclass(&tmp_item[2], vm)?;
        let good_mod = check_matched(&tmp_item[3], &module, vm)?;
        let ln: usize = tmp_item[4].try_int(vm).map_or(0, |v| v.as_u32_mask() as _);

        if good_msg && is_subclass && good_mod && (ln == 0 || lineno == ln) {
            return Ok(action.to_owned());
        }
        i += 1;
    }

    get_default_action(vm)
}

pub fn warn(
    message: PyObjectRef,
    category: Option<PyTypeRef>,
    stack_level: isize,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    warn_with_skip(message, category, stack_level, source, None, vm)
}

/// do_warn: resolve context via setup_context, then call warn_explicit.
pub fn warn_with_skip(
    message: PyObjectRef,
    category: Option<PyTypeRef>,
    mut stack_level: isize,
    source: Option<PyObjectRef>,
    skip_file_prefixes: Option<PyTupleRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if let Some(ref prefixes) = skip_file_prefixes
        && !prefixes.is_empty()
        && stack_level < 2
    {
        stack_level = 2;
    }
    let (filename, lineno, module, registry) =
        setup_context(stack_level, skip_file_prefixes.as_ref(), vm)?;
    warn_explicit(
        category, message, filename, lineno, module, registry, None, source, vm,
    )
}

/// Core warning logic matching `warn_explicit()` in `_warnings.c`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn warn_explicit(
    category: Option<PyTypeRef>,
    message: PyObjectRef,
    filename: PyStrRef,
    lineno: usize,
    module: Option<PyObjectRef>,
    registry: PyObjectRef,
    source_line: Option<PyObjectRef>,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    // Normalize module. None → silent return (late-shutdown safety).
    let module = module.unwrap_or_else(|| normalize_module(&filename, vm));
    if vm.is_none(&module) {
        return Ok(());
    }

    // Normalize message.
    let is_warning = message.fast_isinstance(vm.ctx.exceptions.warning);
    let (text, category, message) = if is_warning {
        let text = message.str(vm)?;
        let cat = message.class().to_owned();
        (text, cat, message)
    } else {
        // For non-Warning messages, convert to string via str()
        let text = message.str(vm)?;
        let cat = category.unwrap_or_else(|| vm.ctx.exceptions.user_warning.to_owned());
        let instance = cat.as_object().call((text.clone(),), vm)?;
        (text, cat, instance)
    };

    let lineno_obj: PyObjectRef = vm.ctx.new_int(lineno).into();

    // key = (text, category, lineno)
    let key: PyObjectRef = PyTuple::new_ref(
        vec![
            text.clone().into(),
            category.as_object().to_owned(),
            lineno_obj.clone(),
        ],
        &vm.ctx,
    )
    .into();

    // Check if already warned
    if !vm.is_none(&registry) && already_warned(&registry, key.clone(), false, vm)? {
        return Ok(());
    }

    // Get filter action
    let action = get_filter(
        category.as_object().to_owned(),
        text.clone().into(),
        lineno,
        module,
        vm,
    )?;
    let action_str = PyStrRef::try_from_object(vm, action)
        .map_err(|_| vm.new_type_error("action must be a string".to_owned()))?;

    if action_str.as_str() == "error" {
        let exc = PyBaseExceptionRef::try_from_object(vm, message)?;
        return Err(exc);
    }
    if action_str.as_str() == "ignore" {
        return Ok(());
    }

    // For everything except "always"/"all", record in registry then
    // check per-action registries.
    let already = if action_str.as_str() != "always" && action_str.as_str() != "all" {
        if !vm.is_none(&registry) {
            registry.set_item(&*key, vm.ctx.true_value.clone().into(), vm)?;
        }

        match action_str.as_str() {
            "once" => {
                let reg = if vm.is_none(&registry) {
                    get_once_registry(vm)?
                } else {
                    registry.clone()
                };
                update_registry(&reg, text.as_ref(), category.as_object(), false, vm)?
            }
            "module" => {
                if !vm.is_none(&registry) {
                    update_registry(&registry, text.as_ref(), category.as_object(), false, vm)?
                } else {
                    false
                }
            }
            "default" => false,
            other => {
                return Err(vm.new_runtime_error(format!(
                    "Unrecognized action ({other}) in warnings.filters:\n {other}"
                )));
            }
        }
    } else {
        false
    };

    if already {
        return Ok(());
    }

    call_show_warning(
        category,
        text,
        message,
        filename,
        lineno,
        lineno_obj,
        source_line,
        source,
        vm,
    )
}

#[allow(clippy::too_many_arguments)]
fn call_show_warning(
    category: PyTypeRef,
    text: PyStrRef,
    message: PyObjectRef,
    filename: PyStrRef,
    lineno: usize,
    lineno_obj: PyObjectRef,
    source_line: Option<PyObjectRef>,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let Some(show_fn) =
        get_warnings_attr(vm, identifier!(&vm.ctx, _showwarnmsg), source.is_some())?
    else {
        return show_warning(filename, lineno, text, category, source_line, vm);
    };
    if !show_fn.is_callable() {
        return Err(
            vm.new_type_error("warnings._showwarnmsg() must be set to a callable".to_owned())
        );
    }
    let Some(warnmsg_cls) = get_warnings_attr(vm, identifier!(&vm.ctx, WarningMessage), false)?
    else {
        return Err(vm.new_runtime_error("unable to get warnings.WarningMessage".to_owned()));
    };

    let msg = warnmsg_cls.call(
        vec![
            message,
            category.into(),
            filename.into(),
            lineno_obj,
            vm.ctx.none(),
            vm.ctx.none(),
            vm.unwrap_or_none(source),
        ],
        vm,
    )?;
    show_fn.call((msg,), vm)?;
    Ok(())
}

fn show_warning(
    filename: PyStrRef,
    lineno: usize,
    text: PyStrRef,
    category: PyTypeRef,
    _source_line: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let stderr = crate::stdlib::sys::PyStderr(vm);
    writeln!(
        stderr,
        "{}:{}: {}: {}",
        filename,
        lineno,
        category.name(),
        text
    );
    Ok(())
}

/// Check if a frame's filename starts with any of the given prefixes.
fn is_filename_to_skip(frame: &crate::frame::Frame, prefixes: &PyTupleRef) -> bool {
    let filename = frame.f_code().co_filename();
    let filename_s = filename.as_str();
    prefixes.iter().any(|prefix| {
        prefix
            .downcast_ref::<PyStr>()
            .is_some_and(|s| filename_s.starts_with(s.as_str()))
    })
}

/// Like Frame::next_external_frame but also skips frames matching prefixes.
fn next_external_frame_with_skip(
    frame: &crate::frame::FrameRef,
    skip_file_prefixes: Option<&PyTupleRef>,
    vm: &VirtualMachine,
) -> Option<crate::frame::FrameRef> {
    let mut f = frame.f_back(vm);
    loop {
        let current: crate::frame::FrameRef = f.take()?;
        if current.is_internal_frame()
            || skip_file_prefixes.is_some_and(|p| is_filename_to_skip(&current, p))
        {
            f = current.f_back(vm);
        } else {
            return Some(current);
        }
    }
}

/// filename, module, and registry are new refs, globals is borrowed
/// Returns `Ok` on success, or `Err` on error (no new refs)
fn setup_context(
    mut stack_level: isize,
    skip_file_prefixes: Option<&PyTupleRef>,
    vm: &VirtualMachine,
) -> PyResult<(PyStrRef, usize, Option<PyObjectRef>, PyObjectRef)> {
    let mut f = vm.current_frame().as_deref().cloned();

    // Stack level comparisons to Python code is off by one as there is no
    // warnings-related stack level to avoid.
    if stack_level <= 0 || f.as_ref().is_some_and(|frame| frame.is_internal_frame()) {
        while {
            stack_level -= 1;
            stack_level > 0
        } {
            match f {
                Some(tmp) => f = tmp.f_back(vm),
                None => break,
            }
        }
    } else {
        while {
            stack_level -= 1;
            stack_level > 0
        } {
            match f {
                Some(tmp) => f = next_external_frame_with_skip(&tmp, skip_file_prefixes, vm),
                None => break,
            }
        }
    }

    let (globals, filename, lineno) = if let Some(f) = f {
        (f.globals.clone(), f.code.source_path(), f.f_lineno())
    } else if let Some(frame) = vm.current_frame() {
        // We have a frame but it wasn't found during stack walking
        (frame.globals.clone(), vm.ctx.intern_str("<sys>"), 1)
    } else {
        // No frames on the stack - use sys.__dict__ (interp->sysdict)
        let globals = vm
            .sys_module
            .as_object()
            .get_attr(identifier!(vm, __dict__), vm)
            .and_then(|d| {
                d.downcast::<crate::builtins::PyDict>()
                    .map_err(|_| vm.new_type_error("sys.__dict__ is not a dictionary".to_owned()))
            })?;
        (globals, vm.ctx.intern_str("<sys>"), 0)
    };

    let registry = match globals.get_item("__warningregistry__", vm) {
        Ok(r) => r,
        Err(_) => {
            let r = vm.ctx.new_dict();
            globals.set_item("__warningregistry__", r.clone().into(), vm)?;
            r.into()
        }
    };

    // Setup module.
    let module = globals
        .get_item("__name__", vm)
        .unwrap_or_else(|_| vm.new_pyobj("<string>"));
    Ok((filename.to_owned(), lineno, Some(module), registry))
}
