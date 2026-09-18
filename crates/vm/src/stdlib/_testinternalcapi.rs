pub(crate) use _testinternalcapi::module_def;

use core::sync::atomic::{AtomicUsize, Ordering};

static RARE_SET_CLASS: AtomicUsize = AtomicUsize::new(0);
static RARE_SET_BASES: AtomicUsize = AtomicUsize::new(0);
static RARE_SET_EVAL_FRAME_FUNC: AtomicUsize = AtomicUsize::new(0);
static RARE_BUILTIN_DICT: AtomicUsize = AtomicUsize::new(0);
static RARE_FUNC_MODIFICATION: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn note_set_class() {
    RARE_SET_CLASS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_set_bases() {
    RARE_SET_BASES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_builtin_dict() {
    RARE_BUILTIN_DICT.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn note_func_modification() {
    RARE_FUNC_MODIFICATION.fetch_add(1, Ordering::Relaxed);
}

#[pymodule]
mod _testinternalcapi {
    use super::*;
    use crate::{
        PyObjectRef, PyResult, VirtualMachine,
        builtins::{PyBytesRef, PyCode, PyDictRef, PyStrRef},
        function::OptionalArg,
    };

    // ADAPTIVE_WARMUP_VALUE + 1
    #[pyattr]
    const SPECIALIZATION_THRESHOLD: usize = 2;

    // ADAPTIVE_COOLDOWN_VALUE + 1
    #[pyattr]
    const SPECIALIZATION_COOLDOWN: usize = 53;

    #[pyattr]
    const SHARED_KEYS_MAX_SIZE: usize = crate::object::SHARED_KEYS_MAX_SIZE;

    #[pyfunction]
    fn has_inline_values(obj: PyObjectRef, _vm: &VirtualMachine) -> bool {
        obj.has_inline_values()
    }

    #[pyfunction]
    fn get_recursion_depth(vm: &VirtualMachine) -> usize {
        vm.current_recursion_depth()
    }

    #[pyfunction]
    fn reset_rare_event_counters() {
        RARE_SET_CLASS.store(0, Ordering::Relaxed);
        RARE_SET_BASES.store(0, Ordering::Relaxed);
        RARE_SET_EVAL_FRAME_FUNC.store(0, Ordering::Relaxed);
        RARE_BUILTIN_DICT.store(0, Ordering::Relaxed);
        RARE_FUNC_MODIFICATION.store(0, Ordering::Relaxed);
    }

    #[pyfunction]
    fn get_rare_event_counters(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let dict = vm.ctx.new_dict();
        dict.set_item(
            "set_class",
            vm.ctx
                .new_int(RARE_SET_CLASS.load(Ordering::Relaxed))
                .into(),
            vm,
        )?;
        dict.set_item(
            "set_bases",
            vm.ctx
                .new_int(RARE_SET_BASES.load(Ordering::Relaxed))
                .into(),
            vm,
        )?;
        dict.set_item(
            "set_eval_frame_func",
            vm.ctx
                .new_int(RARE_SET_EVAL_FRAME_FUNC.load(Ordering::Relaxed))
                .into(),
            vm,
        )?;
        dict.set_item(
            "builtin_dict",
            vm.ctx
                .new_int(RARE_BUILTIN_DICT.load(Ordering::Relaxed))
                .into(),
            vm,
        )?;
        dict.set_item(
            "func_modification",
            vm.ctx
                .new_int(RARE_FUNC_MODIFICATION.load(Ordering::Relaxed))
                .into(),
            vm,
        )?;
        Ok(dict)
    }

    #[pyfunction]
    fn set_eval_frame_record(_recorder: PyObjectRef) {
        RARE_SET_EVAL_FRAME_FUNC.fetch_add(1, Ordering::Relaxed);
    }

    #[pyfunction]
    fn set_eval_frame_default() {}

    #[pyfunction]
    fn code_returns_only_none(code: PyRef<PyCode>, _vm: &VirtualMachine) -> bool {
        crate::vm::crossinterp::code_returns_only_none(&code)
    }

    #[pyfunction]
    fn get_co_localskinds(code: PyRef<PyCode>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let kinds = vm.ctx.new_dict();
        for (offset, &kind) in code.localspluskinds.iter().enumerate() {
            kinds.set_item(
                code.localsplus_name(offset),
                vm.ctx.new_int(kind).into(),
                vm,
            )?;
        }
        Ok(kinds)
    }

    #[derive(FromArgs)]
    struct GetCodeVarCountsArgs {
        code: PyObjectRef,
        #[pyarg(any, optional)]
        globalnames: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        attrnames: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        globalsns: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        builtinsns: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn get_code_var_counts(args: GetCodeVarCountsArgs, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let (code, default_globals, default_builtins) = code_or_function(&args.code, vm)?;
        let globalsns = optional_dict(args.globalsns, default_globals, "globalsns", vm)?;
        let builtinsns = optional_dict(args.builtinsns, default_builtins, "builtinsns", vm)?;
        let mut counts = var_counts(&code);
        set_unbound_var_counts(
            &code,
            &mut counts,
            args.globalnames.into_option(),
            args.attrnames.into_option(),
            globalsns.as_deref(),
            builtinsns.as_deref(),
            vm,
        )?;
        counts_to_dict(&counts, vm)
    }

    #[derive(FromArgs)]
    struct VerifyStatelessCodeArgs {
        code: PyObjectRef,
        #[pyarg(any, optional)]
        globalnames: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        globalsns: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        builtinsns: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn verify_stateless_code(args: VerifyStatelessCodeArgs, vm: &VirtualMachine) -> PyResult<()> {
        let _ = args.globalnames;
        let (code, default_globals, default_builtins) = code_or_function(&args.code, vm)?;
        let globalsns = optional_dict(args.globalsns, default_globals, "globalsns", vm)?;
        let builtinsns = optional_dict(args.builtinsns, default_builtins, "builtinsns", vm)?;
        let mut counts = var_counts(&code);
        set_unbound_var_counts(
            &code,
            &mut counts,
            None,
            None,
            globalsns.as_deref(),
            builtinsns.as_deref(),
            vm,
        )?;
        if builtinsns.is_some() {
            // Force CheckNoExternalState to reject leftover unknown names
            // whenever a builtins namespace was supplied.
            counts.unbound.globals.numbuiltin += 1;
        }
        if let Some(errmsg) = check_no_external_state(&counts) {
            return Err(vm.new_value_error(errmsg.to_owned()));
        }
        Ok(())
    }

    #[pyfunction]
    fn normalize_path(filename: PyStrRef, _vm: &VirtualMachine) -> String {
        normpath(filename.to_str().unwrap_or(""))
    }

    #[pyfunction(name = "EncodeLocaleEx")]
    fn encode_locale_ex(
        text: PyStrRef,
        _current_locale: OptionalArg<i32>,
        errors: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        locale_codec(
            text.as_object(),
            "encode",
            errors.into_option(),
            "encode error",
            vm,
        )
    }

    #[pyfunction(name = "DecodeLocaleEx")]
    fn decode_locale_ex(
        encoded: PyBytesRef,
        _current_locale: OptionalArg<i32>,
        errors: OptionalArg<PyStrRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        locale_codec(
            encoded.as_object(),
            "decode",
            errors.into_option(),
            "decode error",
            vm,
        )
    }

    #[pyfunction]
    fn run_in_subinterp_with_config(
        code: PyStrRef,
        config: PyObjectRef,
        _xi: OptionalArg<bool>,
        vm: &VirtualMachine,
    ) -> PyResult<i32> {
        let config = crate::stdlib::_interpreters::config_from_pyobject(&config, vm)?;
        #[cfg(feature = "threading")]
        {
            run_string_in_new_subinterp(code.to_str().unwrap_or(""), config, vm)
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = (code, config);
            Err(vm.new_runtime_error("isolated interpreters require threading"))
        }
    }
}

use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyRef, PyResult, VirtualMachine,
    builtins::{PyCode, PyDict, PyDictRef, PyFunction, PySet, PyStr, PyStrInterned, PyStrRef},
    bytecode::{
        CO_FAST_ARG, CO_FAST_ARG_KW, CO_FAST_ARG_POS, CO_FAST_ARG_VAR, CO_FAST_CELL, CO_FAST_FREE,
        CO_FAST_HIDDEN, Instruction,
    },
    function::OptionalArg,
};
use std::collections::HashSet;

type CodeNamespaces = (PyRef<PyCode>, Option<PyRef<PyDict>>, Option<PyRef<PyDict>>);

fn code_or_function(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CodeNamespaces> {
    if let Ok(func) = obj.to_owned().downcast::<PyFunction>() {
        let builtins = func.builtins.clone().downcast::<PyDict>().ok();
        return Ok((
            (*func.code).to_owned(),
            Some(func.globals.clone()),
            builtins,
        ));
    }
    if let Ok(code) = obj.to_owned().downcast::<PyCode>() {
        return Ok((code, None, None));
    }
    Err(vm.new_type_error("argument must be a code object or a function"))
}

fn optional_dict(
    override_ns: OptionalArg<PyObjectRef>,
    default: Option<PyRef<PyDict>>,
    name: &str,
    vm: &VirtualMachine,
) -> PyResult<Option<PyRef<PyDict>>> {
    match override_ns.into_option() {
        Some(obj) => obj.downcast::<PyDict>().map(Some).map_err(|obj| {
            vm.new_type_error(format!(
                "expected a dict for \"{name}\", got {}",
                obj.class().name()
            ))
        }),
        None => Ok(default),
    }
}

#[derive(Default, Clone, Copy)]
struct ArgsCounts {
    total: i32,
    numposonly: i32,
    numposorkw: i32,
    numkwonly: i32,
    varargs: i32,
    varkwargs: i32,
}

#[derive(Default, Clone, Copy)]
struct CellCounts {
    total: i32,
    numargs: i32,
    numothers: i32,
}

#[derive(Default, Clone, Copy)]
struct HiddenCounts {
    total: i32,
    numpure: i32,
    numcells: i32,
}

#[derive(Default, Clone, Copy)]
struct LocalsCounts {
    total: i32,
    args: ArgsCounts,
    numpure: i32,
    cells: CellCounts,
    hidden: HiddenCounts,
}

#[derive(Default, Clone, Copy)]
struct GlobalCounts {
    total: i32,
    numglobal: i32,
    numbuiltin: i32,
    numunknown: i32,
}

#[derive(Default, Clone, Copy)]
struct UnboundCounts {
    total: i32,
    globals: GlobalCounts,
    numattrs: i32,
    numunknown: i32,
}

#[derive(Default, Clone, Copy)]
struct VarCounts {
    total: i32,
    locals: LocalsCounts,
    numfree: i32,
    unbound: UnboundCounts,
}

fn var_counts(code: &PyCode) -> VarCounts {
    let mut locals = LocalsCounts::default();
    let mut numfree = 0;
    for &kind in &code.localspluskinds {
        if kind & CO_FAST_FREE != 0 {
            numfree += 1;
        } else {
            locals.total += 1;
            if kind & CO_FAST_ARG != 0 {
                locals.args.total += 1;
                if kind & CO_FAST_ARG_VAR != 0 {
                    if kind & CO_FAST_ARG_POS != 0 {
                        locals.args.varargs = 1;
                    } else {
                        locals.args.varkwargs = 1;
                    }
                } else if kind & CO_FAST_ARG_POS != 0 {
                    if kind & CO_FAST_ARG_KW != 0 {
                        locals.args.numposorkw += 1;
                    } else {
                        locals.args.numposonly += 1;
                    }
                } else {
                    locals.args.numkwonly += 1;
                }
                if kind & CO_FAST_CELL != 0 {
                    locals.cells.total += 1;
                    locals.cells.numargs += 1;
                }
            } else if kind & CO_FAST_CELL != 0 {
                locals.cells.total += 1;
                locals.cells.numothers += 1;
                if kind & CO_FAST_HIDDEN != 0 {
                    locals.hidden.total += 1;
                    locals.hidden.numcells += 1;
                }
            } else {
                locals.numpure += 1;
                if kind & CO_FAST_HIDDEN != 0 {
                    locals.hidden.total += 1;
                    locals.hidden.numpure += 1;
                }
            }
        }
    }
    let numunbound = code.names.len() as i32;
    let unbound = UnboundCounts {
        total: numunbound,
        numunknown: numunbound,
        ..UnboundCounts::default()
    };
    VarCounts {
        total: locals.total + numfree + unbound.total,
        locals,
        numfree,
        unbound,
    }
}

fn set_unbound_var_counts(
    code: &PyCode,
    counts: &mut VarCounts,
    globalnames: Option<PyObjectRef>,
    attrnames: Option<PyObjectRef>,
    globalsns: Option<&Py<PyDict>>,
    builtinsns: Option<&Py<PyDict>>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let globalnames = optional_set(globalnames, "globalnames", vm)?;
    let attrnames = optional_set(attrnames, "attrnames", vm)?;
    let (unbound, numdupes) =
        identify_unbound_names(code, globalnames, attrnames, globalsns, builtinsns, vm)?;
    let totalunbound = counts.unbound.total + numdupes;
    let mut unbound = unbound;
    unbound.numunknown = totalunbound - unbound.total;
    unbound.total = totalunbound;
    counts.unbound = unbound;
    counts.total += numdupes;
    Ok(())
}

fn optional_set(
    obj: Option<PyObjectRef>,
    name: &str,
    vm: &VirtualMachine,
) -> PyResult<Option<PyRef<PySet>>> {
    match obj {
        None => Ok(None),
        Some(obj) => obj.downcast::<PySet>().map(Some).map_err(|obj| {
            vm.new_type_error(format!(
                "expected a set for \"{name}\", got {}",
                obj.class().name()
            ))
        }),
    }
}

fn identify_unbound_names(
    code: &PyCode,
    globalnames: Option<PyRef<PySet>>,
    attrnames: Option<PyRef<PySet>>,
    globalsns: Option<&Py<PyDict>>,
    builtinsns: Option<&Py<PyDict>>,
    vm: &VirtualMachine,
) -> PyResult<(UnboundCounts, i32)> {
    let mut seen_globals = HashSet::new();
    let mut seen_attrs = HashSet::new();
    let mut unbound = UnboundCounts::default();
    let mut numdupes = 0;
    for (_, instr, arg) in crate::vm::crossinterp::walk_instructions(code) {
        match instr {
            Instruction::LoadAttr { namei } => {
                let name = code.names[namei.get(arg).name_idx() as usize];
                if name_already_seen(&seen_attrs, attrnames.as_deref(), name, vm)? {
                    continue;
                }
                seen_attrs.insert(name_key(name));
                if let Some(set) = attrnames.as_deref() {
                    set.add(name.to_owned().into(), vm)?;
                }
                unbound.total += 1;
                unbound.numattrs += 1;
                if name_already_seen(&seen_globals, globalnames.as_deref(), name, vm)? {
                    numdupes += 1;
                }
            }
            Instruction::LoadGlobal { namei } => {
                let name = code.names[(namei.get(arg) >> 1) as usize];
                if name_already_seen(&seen_globals, globalnames.as_deref(), name, vm)? {
                    continue;
                }
                seen_globals.insert(name_key(name));
                if let Some(set) = globalnames.as_deref() {
                    set.add(name.to_owned().into(), vm)?;
                }
                unbound.total += 1;
                unbound.globals.total += 1;
                if globalsns.is_some_and(|ns| ns.contains_key(name, vm)) {
                    unbound.globals.numglobal += 1;
                } else if builtinsns.is_some_and(|ns| ns.contains_key(name, vm)) {
                    unbound.globals.numbuiltin += 1;
                } else {
                    unbound.globals.numunknown += 1;
                }
                if name_already_seen(&seen_attrs, attrnames.as_deref(), name, vm)? {
                    numdupes += 1;
                }
            }
            _ => {}
        }
    }
    Ok((unbound, numdupes))
}

fn name_already_seen(
    seen: &HashSet<usize>,
    user_set: Option<&Py<PySet>>,
    name: &'static PyStrInterned,
    vm: &VirtualMachine,
) -> PyResult<bool> {
    if seen.contains(&name_key(name)) {
        return Ok(true);
    }
    if let Some(set) = user_set {
        return set.__contains__(name.as_object(), vm);
    }
    Ok(false)
}

fn name_key(name: &'static PyStrInterned) -> usize {
    core::ptr::from_ref(name.as_object()) as usize
}

fn check_no_external_state(counts: &VarCounts) -> Option<&'static str> {
    if counts.numfree > 0 {
        Some("closures not supported")
    } else if counts.unbound.globals.numglobal > 0
        || (counts.unbound.globals.numbuiltin > 0 && counts.unbound.globals.numunknown > 0)
    {
        Some("globals not supported")
    } else {
        None
    }
}

fn counts_to_dict(counts: &VarCounts, vm: &VirtualMachine) -> PyResult<PyDictRef> {
    let countsobj = vm.ctx.new_dict();
    set_count(&countsobj, "total", counts.total, vm)?;

    let locals = vm.ctx.new_dict();
    countsobj.set_item("locals", locals.clone().into(), vm)?;
    set_count(&locals, "total", counts.locals.total, vm)?;

    let args = vm.ctx.new_dict();
    locals.set_item("args", args.clone().into(), vm)?;
    set_count(&args, "total", counts.locals.args.total, vm)?;
    set_count(&args, "numposonly", counts.locals.args.numposonly, vm)?;
    set_count(&args, "numposorkw", counts.locals.args.numposorkw, vm)?;
    set_count(&args, "numkwonly", counts.locals.args.numkwonly, vm)?;
    set_count(&args, "varargs", counts.locals.args.varargs, vm)?;
    set_count(&args, "varkwargs", counts.locals.args.varkwargs, vm)?;

    set_count(&locals, "numpure", counts.locals.numpure, vm)?;

    let cells = vm.ctx.new_dict();
    locals.set_item("cells", cells.clone().into(), vm)?;
    set_count(&cells, "total", counts.locals.cells.total, vm)?;
    set_count(&cells, "numargs", counts.locals.cells.numargs, vm)?;
    set_count(&cells, "numothers", counts.locals.cells.numothers, vm)?;

    let hidden = vm.ctx.new_dict();
    locals.set_item("hidden", hidden.clone().into(), vm)?;
    set_count(&hidden, "total", counts.locals.hidden.total, vm)?;
    set_count(&hidden, "numpure", counts.locals.hidden.numpure, vm)?;
    set_count(&hidden, "numcells", counts.locals.hidden.numcells, vm)?;

    set_count(&countsobj, "numfree", counts.numfree, vm)?;

    let unbound = vm.ctx.new_dict();
    countsobj.set_item("unbound", unbound.clone().into(), vm)?;
    set_count(&unbound, "total", counts.unbound.total, vm)?;
    set_count(&unbound, "numattrs", counts.unbound.numattrs, vm)?;
    set_count(&unbound, "numunknown", counts.unbound.numunknown, vm)?;

    let globals = vm.ctx.new_dict();
    unbound.set_item("globals", globals.clone().into(), vm)?;
    set_count(&globals, "total", counts.unbound.globals.total, vm)?;
    set_count(&globals, "numglobal", counts.unbound.globals.numglobal, vm)?;
    set_count(
        &globals,
        "numbuiltin",
        counts.unbound.globals.numbuiltin,
        vm,
    )?;
    set_count(
        &globals,
        "numunknown",
        counts.unbound.globals.numunknown,
        vm,
    )?;

    Ok(countsobj)
}

fn set_count(dict: &Py<PyDict>, name: &str, value: i32, vm: &VirtualMachine) -> PyResult<()> {
    dict.set_item(name, vm.ctx.new_int(value).into(), vm)
}

fn locale_error_handler(errors: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
    let Some(errors) = errors else {
        return Ok(vm.ctx.new_str("strict"));
    };
    match errors.to_str() {
        Some("strict" | "surrogateescape" | "surrogatepass") => Ok(errors),
        _ => Err(vm.new_value_error("unsupported error handler")),
    }
}

fn locale_codec(
    obj: &PyObject,
    method: &str,
    errors: Option<PyStrRef>,
    kind: &str,
    vm: &VirtualMachine,
) -> PyResult {
    let errors = locale_error_handler(errors, vm)?;
    let encoding: PyObjectRef = vm.fs_encoding().to_owned().into();
    match vm.call_method(obj, method, (encoding, errors)) {
        Ok(value) => Ok(value),
        Err(exc) => {
            if exc.fast_isinstance(vm.ctx.exceptions.lookup_error) {
                return Err(vm.new_value_error("unsupported error handler"));
            }
            let codec_exc = if method == "encode" {
                vm.ctx.exceptions.unicode_encode_error
            } else {
                vm.ctx.exceptions.unicode_decode_error
            };
            if exc.fast_isinstance(codec_exc) {
                let start = exc
                    .as_object()
                    .get_attr("start", vm)
                    .ok()
                    .and_then(|v| v.try_to_value::<isize>(vm).ok())
                    .unwrap_or(0);
                let reason = exc
                    .as_object()
                    .get_attr("reason", vm)
                    .ok()
                    .and_then(|v| {
                        v.downcast_ref::<PyStr>()
                            .and_then(|s| s.to_str())
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                return Err(vm.new_runtime_error(format!("{kind}: pos={start}, reason={reason}")));
            }
            Err(exc)
        }
    }
}

fn normpath(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut buf: Vec<char> = path.chars().collect();
    let size = buf.len();
    let (drvsize, rootsize) = skiproot(&buf);
    let mut p1 = drvsize + rootsize;
    let mut p2 = p1;
    let min_p2 = p2.saturating_sub(1);
    let mut last_c = if drvsize + rootsize > 0 {
        buf[min_p2]
    } else {
        '\0'
    };

    if p1 < size && buf[p1] == '.' && sep_or_end(&buf, p1 + 1, size) {
        p1 += 1;
        last_c = if p1 < size { buf[p1] } else { '\0' };
        while p1 < size && buf[p1] == '/' {
            p1 += 1;
        }
    }

    while p1 < size {
        let c = buf[p1];
        if last_c == '/' {
            if c == '.' {
                let sep_at_1 = sep_or_end(&buf, p1 + 1, size);
                let sep_at_2 = !sep_at_1 && sep_or_end(&buf, p1 + 2, size);
                if sep_at_2 && p1 + 1 < size && buf[p1 + 1] == '.' {
                    let mut p3 = p2;
                    while p3 != min_p2 && {
                        p3 -= 1;
                        buf[p3] == '/'
                    } {}
                    while p3 != min_p2 && buf[p3 - 1] != '/' {
                        p3 -= 1;
                    }
                    if p2 == min_p2
                        || (buf.get(p3) == Some(&'.')
                            && buf.get(p3 + 1) == Some(&'.')
                            && (p3 + 2 >= size || buf[p3 + 2] == '/'))
                    {
                        buf[p2] = '.';
                        p2 += 1;
                        buf[p2] = '.';
                        p2 += 1;
                        last_c = '.';
                    } else if buf.get(p3) == Some(&'/') {
                        p2 = p3 + 1;
                    } else {
                        p2 = p3;
                    }
                    p1 += 1;
                } else if sep_at_1 {
                } else {
                    buf[p2] = c;
                    p2 += 1;
                    last_c = c;
                }
            } else if c != '/' {
                buf[p2] = c;
                p2 += 1;
                last_c = c;
            }
        } else {
            buf[p2] = c;
            p2 += 1;
            last_c = c;
        }
        p1 += 1;
    }

    if p2 != min_p2 {
        while p2 != 0 {
            p2 -= 1;
            if p2 == min_p2 || buf[p2] != '/' {
                break;
            }
        }
        buf.truncate(p2 + 1);
    } else if p2 > 0 {
        buf.truncate(p2);
    } else {
        buf.clear();
    }
    buf.into_iter().collect()
}

fn skiproot(path: &[char]) -> (usize, usize) {
    if path.first() != Some(&'/') {
        return (0, 0);
    }
    let c1 = path.get(1).copied().unwrap_or('\0');
    let c2 = path.get(2).copied().unwrap_or('\0');
    if c1 != '/' || c2 == '/' {
        (0, 1)
    } else {
        (0, 2)
    }
}

fn sep_or_end(path: &[char], idx: usize, size: usize) -> bool {
    idx >= size || path[idx] == '/'
}

#[cfg(feature = "threading")]
fn run_string_in_new_subinterp(
    source: &str,
    config: crate::vm::InterpreterConfig,
    vm: &VirtualMachine,
) -> PyResult<i32> {
    let interp = crate::Interpreter::create_subinterpreter_from_vm(vm, config).map_err(|msg| {
        let cause = vm.new_runtime_error(msg.to_owned());
        let exc =
            crate::stdlib::_interpreters::interpreter_error(vm, "sub-interpreter creation failed");
        exc.set___context__(Some(cause));
        exc
    })?;
    let id = crate::vm::runtime::store_owned_interpreter(interp);
    let outcome = crate::vm::crossinterp::with_interpreter(id, vm, |target| {
        match target.compile(source, crate::compiler::Mode::Exec, "<string>") {
            Ok(code) => {
                let ns = target.main_namespace()?;
                let scope = crate::scope::Scope::with_builtins(None, ns, target);
                match target.run_code_obj(code, scope) {
                    Ok(_) => Ok(0),
                    Err(exc) => {
                        target.print_exception(exc);
                        Ok(-1)
                    }
                }
            }
            Err(err) => {
                let exc = err.into_pyexception(target, Some(source));
                target.print_exception(exc);
                Ok(-1)
            }
        }
    });
    let _ = crate::vm::runtime::destroy_owned_interpreter(id);
    outcome
}
