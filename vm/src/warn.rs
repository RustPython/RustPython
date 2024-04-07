use crate::{
    builtins::{
        PyDict, PyDictRef, PyListRef, PyStr, PyStrInterned, PyStrRef, PyTuple, PyTupleRef,
        PyTypeRef,
    },
    convert::{IntoObject, TryFromObject},
    types::PyComparisonOp,
    AsObject, Context, Py, PyObjectRef, PyResult, VirtualMachine,
};

pub struct WarningsState {
    filters: PyListRef,
    _once_registry: PyDictRef,
    default_action: PyStrRef,
    filters_version: usize,
}

impl WarningsState {
    fn create_filter(ctx: &Context) -> PyListRef {
        ctx.new_list(vec![ctx
            .new_tuple(vec![
                ctx.new_str("__main__").into(),
                ctx.types.none_type.as_object().to_owned(),
                ctx.exceptions.warning.as_object().to_owned(),
                ctx.new_str("ACTION").into(),
                ctx.new_int(0).into(),
            ])
            .into()])
    }

    pub fn init_state(ctx: &Context) -> WarningsState {
        WarningsState {
            filters: Self::create_filter(ctx),
            _once_registry: PyDict::new_ref(ctx),
            default_action: ctx.new_str("default"),
            filters_version: 0,
        }
    }
}

fn check_matched(obj: &PyObjectRef, arg: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
    if obj.class().is(vm.ctx.types.none_type) {
        return Ok(true);
    }

    if obj.rich_compare_bool(arg, PyComparisonOp::Eq, vm)? {
        return Ok(false);
    }

    let result = obj.call((arg.to_owned(),), vm);
    Ok(result.is_ok())
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
            .load(std::sync::atomic::Ordering::SeqCst)
    {
        match vm.import("warnings", 0) {
            Ok(module) => module,
            Err(_) => return Ok(None),
        }
    } else {
        // TODO: finalizing support
        return Ok(None);
    };
    Ok(Some(module.get_attr(attr_name, vm)?))
}

pub fn warn(
    message: PyStrRef,
    category: Option<PyTypeRef>,
    stack_level: isize,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let (filename, lineno, module, registry) = setup_context(stack_level, vm)?;
    warn_explicit(
        category, message, filename, lineno, module, registry, None, source, vm,
    )
}

fn get_default_action(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    Ok(vm.state.warnings.default_action.clone().into())
    // .map_err(|_| {
    //     vm.new_value_error(format!(
    //         "_warnings.defaultaction must be a string, not '{}'",
    //         vm.state.warnings.default_action
    //     ))
    // })
}

fn get_filter(
    category: PyObjectRef,
    text: PyObjectRef,
    lineno: usize,
    module: PyObjectRef,
    mut _item: PyTupleRef,
    vm: &VirtualMachine,
) -> PyResult {
    let filters = vm.state.warnings.filters.as_object().to_owned();

    let filters: PyListRef = filters
        .try_into_value(vm)
        .map_err(|_| vm.new_value_error("_warnings.filters must be a list".to_string()))?;

    /* WarningsState.filters could change while we are iterating over it. */
    for i in 0..filters.borrow_vec().len() {
        let tmp_item = if let Some(tmp_item) = filters.borrow_vec().get(i).cloned() {
            let tmp_item = PyTupleRef::try_from_object(vm, tmp_item)?;
            (tmp_item.len() == 5).then_some(tmp_item)
        } else {
            None
        }
        .ok_or_else(|| vm.new_value_error(format!("_warnings.filters item {i} isn't a 5-tuple")))?;

        /* Python code: action, msg, cat, mod, ln = item */
        let action = if let Some(action) = tmp_item.first() {
            action.str(vm).map(|action| action.into_object())
        } else {
            Err(vm.new_type_error("action must be a string".to_string()))
        };

        let good_msg = if let Some(msg) = tmp_item.get(1) {
            check_matched(msg, &text, vm)?
        } else {
            false
        };

        let is_subclass = if let Some(cat) = tmp_item.get(2) {
            category.fast_isinstance(cat.class())
        } else {
            false
        };

        let good_mod = if let Some(item_mod) = tmp_item.get(3) {
            check_matched(item_mod, &module, vm)?
        } else {
            false
        };

        let ln = tmp_item.get(4).map_or(0, |ln_obj| {
            ln_obj.try_int(vm).map_or(0, |ln| ln.as_u32_mask() as _)
        });

        if good_msg && good_mod && is_subclass && (ln == 0 || lineno == ln) {
            _item = tmp_item;
            return action;
        }
    }

    get_default_action(vm)
}

fn already_warned(
    registry: PyObjectRef,
    key: PyObjectRef,
    should_set: bool,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    let version_obj = registry.get_item(identifier!(&vm.ctx, version), vm).ok();
    let filters_version = vm.ctx.new_int(vm.state.warnings.filters_version).into();

    match version_obj {
        Some(version_obj)
            if version_obj.try_int(vm).is_ok() || version_obj.is(&filters_version) =>
        {
            let already_warned = registry.get_item(key.as_ref(), vm)?;
            if already_warned.is_true(vm)? {
                return Ok(true);
            }
        }
        _ => {
            let registry = registry.dict();
            if let Some(registry) = registry.as_ref() {
                registry.clear();
                let r = registry.set_item("version", filters_version, vm);
                if r.is_err() {
                    return Ok(false);
                }
            }
        }
    }

    /* This warning wasn't found in the registry, set it. */
    if !should_set {
        return Ok(false);
    }

    let item = vm.ctx.true_value.clone().into();
    let _ = registry.set_item(key.as_ref(), item, vm); // ignore set error
    Ok(true)
}

fn normalize_module(filename: &Py<PyStr>, vm: &VirtualMachine) -> Option<PyObjectRef> {
    let obj = match filename.char_len() {
        0 => vm.new_pyobj("<unknown>"),
        len if len >= 3 && filename.as_str().ends_with(".py") => {
            vm.new_pyobj(&filename.as_str()[..len - 3])
        }
        _ => filename.as_object().to_owned(),
    };
    Some(obj)
}

#[allow(clippy::too_many_arguments)]
fn warn_explicit(
    category: Option<PyTypeRef>,
    message: PyStrRef,
    filename: PyStrRef,
    lineno: usize,
    module: Option<PyObjectRef>,
    registry: PyObjectRef,
    source_line: Option<PyObjectRef>,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let registry: PyObjectRef = registry
        .try_into_value(vm)
        .map_err(|_| vm.new_type_error("'registry' must be a dict or None".to_owned()))?;

    // Normalize module.
    let module = match module.or_else(|| normalize_module(&filename, vm)) {
        Some(module) => module,
        None => return Ok(()),
    };

    // Normalize message.
    let text = message.as_str();

    let category = if let Some(category) = category {
        if !category.fast_issubclass(vm.ctx.exceptions.warning) {
            return Err(vm.new_type_error(format!(
                "category must be a Warning subclass, not '{}'",
                category.class().name()
            )));
        }
        category
    } else {
        vm.ctx.exceptions.user_warning.to_owned()
    };

    let category = if message.fast_isinstance(vm.ctx.exceptions.warning) {
        message.class().to_owned()
    } else {
        category
    };

    // Create key.
    let key = PyTuple::new_ref(
        vec![
            vm.ctx.new_int(3).into(),
            vm.ctx.new_str(text).into(),
            category.as_object().to_owned(),
            vm.ctx.new_int(lineno).into(),
        ],
        &vm.ctx,
    );

    if !vm.is_none(registry.as_object()) && already_warned(registry, key.into_object(), false, vm)?
    {
        return Ok(());
    }

    let item = vm.ctx.new_tuple(vec![]);
    let action = get_filter(
        category.as_object().to_owned(),
        vm.ctx.new_str(text).into(),
        lineno,
        module,
        item,
        vm,
    )?;

    if action.str(vm)?.as_str().eq("error") {
        return Err(vm.new_type_error(message.to_string()));
    }

    if action.str(vm)?.as_str().eq("ignore") {
        return Ok(());
    }

    call_show_warning(
        // t_state,
        category,
        message,
        filename,
        lineno, // lineno_obj,
        source_line,
        source,
        vm,
    )
}

fn call_show_warning(
    category: PyTypeRef,
    message: PyStrRef,
    filename: PyStrRef,
    lineno: usize,
    source_line: Option<PyObjectRef>,
    source: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let Some(show_fn) =
        get_warnings_attr(vm, identifier!(&vm.ctx, _showwarnmsg), source.is_some())?
    else {
        return show_warning(filename, lineno, message, category, source_line, vm);
    };
    if !show_fn.is_callable() {
        return Err(
            vm.new_type_error("warnings._showwarnmsg() must be set to a callable".to_owned())
        );
    }
    let Some(warnmsg_cls) = get_warnings_attr(vm, identifier!(&vm.ctx, WarningMessage), false)?
    else {
        return Err(vm.new_type_error("unable to get warnings.WarningMessage".to_owned()));
    };

    let msg = warnmsg_cls.call(
        vec![
            message.into(),
            category.into(),
            filename.into(),
            vm.new_pyobj(lineno),
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
    _filename: PyStrRef,
    _lineno: usize,
    text: PyStrRef,
    category: PyTypeRef,
    _source_line: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let stderr = crate::stdlib::sys::PyStderr(vm);
    writeln!(stderr, "{}: {}", category.name(), text.as_str(),);
    Ok(())
}

/// filename, module, and registry are new refs, globals is borrowed
/// Returns `Ok` on success, or `Err` on error (no new refs)
fn setup_context(
    mut stack_level: isize,
    vm: &VirtualMachine,
) -> PyResult<
    // filename, lineno, module, registry
    (PyStrRef, usize, Option<PyObjectRef>, PyObjectRef),
> {
    let __warningregistry__ = "__warningregistry__";
    let __name__ = "__name__";

    let mut f = vm.current_frame().as_deref().cloned();

    // Stack level comparisons to Python code is off by one as there is no
    // warnings-related stack level to avoid.
    if stack_level <= 0 || f.as_ref().map_or(false, |frame| frame.is_internal_frame()) {
        loop {
            stack_level -= 1;
            if stack_level <= 0 {
                break;
            }
            if let Some(tmp) = f {
                f = tmp.f_back(vm);
            } else {
                break;
            }
        }
    } else {
        loop {
            stack_level -= 1;
            if stack_level <= 0 {
                break;
            }
            if let Some(tmp) = f {
                f = tmp.next_external_frame(vm);
            } else {
                break;
            }
        }
    }

    let (globals, filename, lineno) = if let Some(f) = f {
        (f.globals.clone(), f.code.source_path, f.f_lineno())
    } else {
        (vm.current_globals().clone(), vm.ctx.intern_str("sys"), 1)
    };

    let registry = if let Ok(registry) = globals.get_item(__warningregistry__, vm) {
        registry
    } else {
        let registry = PyDict::new_ref(&vm.ctx);
        globals.set_item(__warningregistry__, registry.clone().into(), vm)?;
        registry.into()
    };

    // Setup module.
    let module = globals
        .get_item(__name__, vm)
        .unwrap_or_else(|_| vm.new_pyobj("<string>"));
    Ok((filename.to_owned(), lineno, Some(module), registry))
}
