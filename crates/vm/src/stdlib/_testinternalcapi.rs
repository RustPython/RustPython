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
        AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
        builtins::{
            PyBytesRef, PyCode, PyDict, PyDictRef, PyStrRef, PyType, PyTypeRef,
            descriptor::PyWrapper,
        },
        common::hash::PyHash,
        dict_inner,
        frame::FrameObject,
        function::OptionalArg,
        protocol::PyMappingMethods,
        types::{AsMapping, PyTypeFlags},
    };
    use std::collections::HashMap;

    // ADAPTIVE_WARMUP_VALUE + 1
    #[pyattr]
    const SPECIALIZATION_THRESHOLD: usize = 2;

    // ADAPTIVE_COOLDOWN_VALUE + 1
    #[pyattr]
    const SPECIALIZATION_COOLDOWN: usize = 53;

    #[pyattr]
    const SHARED_KEYS_MAX_SIZE: usize = crate::object::SHARED_KEYS_MAX_SIZE;

    // Free-threaded builds do not prefix objects with PyGC_Head.
    #[pyattr]
    const SIZEOF_PYGC_HEAD: usize = 0;

    #[pyattr]
    const SIZEOF_MANAGED_PRE_HEADER: usize = 2 * core::mem::size_of::<*const ()>();

    #[pyattr]
    const SIZEOF_PYOBJECT: usize = core::mem::size_of::<crate::PyObject>();

    #[pyattr]
    const SIZEOF_TIME_T: usize = 8;

    // JUMP_BACKWARD_INITIAL_VALUE + 1
    #[pyattr]
    const TIER2_THRESHOLD: usize = 4096;

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

    #[pyfunction]
    fn compiler_cleandoc(doc: PyStrRef) -> String {
        clean_doc(doc.to_str().unwrap_or(""))
    }

    #[pyfunction]
    fn dict_getitem_knownhash(
        dict: PyObjectRef,
        key: PyObjectRef,
        hash: PyHash,
        vm: &VirtualMachine,
    ) -> PyResult {
        let Some(dict) = dict.downcast_ref::<PyDict>() else {
            return Err(vm.new_system_error("expected a dict"));
        };
        dict.get_item_known_hash(&key, hash, vm)?
            .ok_or_else(|| vm.new_key_error(key))
    }

    #[pyfunction]
    fn pymem_getallocatorsname() -> &'static str {
        if cfg!(debug_assertions) {
            "mimalloc_debug"
        } else {
            "mimalloc"
        }
    }

    #[pyfunction]
    fn get_c_recursion_remaining(vm: &VirtualMachine) -> usize {
        vm.recursion_limit
            .get()
            .saturating_sub(vm.current_recursion_depth())
    }

    #[pyfunction]
    fn get_stack_pointer() -> usize {
        let marker = 0u8;
        core::ptr::from_ref(&marker) as usize
    }

    #[pyfunction]
    fn get_stack_margin() -> usize {
        VirtualMachine::STACK_MARGIN_BYTES
    }

    #[pyfunction]
    fn get_next_dict_keys_version() -> u32 {
        dict_inner::peek_next_keys_version()
    }

    #[pyfunction]
    fn type_assign_specific_version_unsafe(ty: PyTypeRef, version: u32) {
        ty.assign_specific_version(version);
    }

    #[pyfunction]
    fn get_tracked_heap_size(vm: &VirtualMachine) -> usize {
        vm.state.gc.get_objects(None).len()
    }

    #[pyfunction]
    fn get_long_lived_total(vm: &VirtualMachine) -> usize {
        vm.state.gc.get_objects(None).len()
    }

    #[pyfunction]
    fn get_co_framesize(code: PyRef<PyCode>) -> usize {
        code.localspluskinds.len() + code.max_stackdepth as usize + 1
    }

    #[pyfunction]
    fn iframe_getcode(frame: PyRef<FrameObject>) -> PyRef<PyCode> {
        frame.f_code()
    }

    #[pyfunction]
    fn iframe_getline(frame: PyRef<FrameObject>) -> usize {
        frame.f_lineno()
    }

    #[pyfunction]
    fn iframe_getlasti(frame: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        frame.get_attr("f_lasti", vm)
    }

    #[pyfunction]
    fn get_static_builtin_types(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![vm.ctx.types.object_type.to_owned()];
        while let Some(cls) = stack.pop() {
            let ptr = cls.as_object() as *const PyObject as usize;
            if !seen.insert(ptr) {
                continue;
            }
            if cls.slots.flags.has_feature(PyTypeFlags::HEAPTYPE) {
                continue;
            }
            out.push(cls.clone().into());
            for weak in cls.subclasses.read().iter() {
                if let Some(sub) = weak.upgrade()
                    && let Ok(sub) = sub.downcast::<PyType>()
                {
                    stack.push(sub);
                }
            }
        }
        out
    }

    #[pyfunction]
    fn identify_type_slot_wrappers(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        let object = vm.ctx.types.object_type;
        object
            .get_attributes(&vm.ctx)
            .into_iter()
            .filter(|(_, value)| value.downcastable::<PyWrapper>())
            .map(|(name, _)| name.to_owned().into())
            .collect()
    }

    #[pyfunction]
    fn get_configs(vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let settings = &vm.state.config.settings;
        let global = vm.ctx.new_dict();
        global.set_item("isolated", vm.ctx.new_bool(settings.isolated).into(), vm)?;
        global.set_item("dev_mode", vm.ctx.new_bool(settings.dev_mode).into(), vm)?;
        global.set_item("verbose", vm.ctx.new_int(settings.verbose).into(), vm)?;
        global.set_item("quiet", vm.ctx.new_bool(settings.quiet).into(), vm)?;
        global.set_item("inspect", vm.ctx.new_bool(settings.inspect).into(), vm)?;
        global.set_item(
            "interactive",
            vm.ctx.new_bool(settings.interactive).into(),
            vm,
        )?;
        global.set_item(
            "optimize",
            vm.ctx.new_int(settings.optimize as i32).into(),
            vm,
        )?;
        global.set_item(
            "hash_seed",
            vm.ctx.new_int(settings.hash_seed.unwrap_or(0)).into(),
            vm,
        )?;
        let configs = vm.ctx.new_dict();
        configs.set_item("config", global.into(), vm)?;
        Ok(configs)
    }

    #[pyfunction]
    fn get_interp_settings(id: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let _ = id;
        let flags = vm.state.feature_flags;
        let settings = vm.ctx.new_dict();
        let mut bits = 0u64;
        if flags.use_main_obmalloc {
            bits |= 1 << 5;
        }
        if flags.check_multi_interp_extensions {
            bits |= 1 << 8;
        }
        settings.set_item("feature_flags", vm.ctx.new_int(bits).into(), vm)?;
        settings.set_item("own_gil", vm.ctx.new_bool(true).into(), vm)?;
        Ok(settings)
    }

    #[pyfunction]
    fn create_interpreter(
        config: OptionalArg<PyObjectRef>,
        _whence: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<i64> {
        let cfg = match config.into_option() {
            Some(obj) if !vm.is_none(&obj) => {
                crate::stdlib::_interpreters::config_from_pyobject(&obj, vm)?
            }
            _ => crate::vm::InterpreterConfig::ISOLATED,
        };
        #[cfg(feature = "threading")]
        {
            let interp = crate::Interpreter::create_subinterpreter_from_vm(vm, cfg)
                .map_err(|msg| crate::stdlib::_interpreters::interpreter_error(vm, msg))?;
            Ok(crate::vm::runtime::store_owned_interpreter(interp))
        }
        #[cfg(not(feature = "threading"))]
        {
            let _ = cfg;
            Err(vm.new_runtime_error("isolated interpreters require threading"))
        }
    }

    #[pyfunction(name = "_PyTime_AsTimespec")]
    fn pytime_as_timespec(ns: i64) -> (i64, i64) {
        let sec = ns.div_euclid(1_000_000_000);
        let nsec = ns.rem_euclid(1_000_000_000);
        (sec, nsec)
    }

    #[pyfunction(name = "_PyTime_AsTimespec_clamp")]
    fn pytime_as_timespec_clamp(ns: i64) -> (i64, i64) {
        pytime_as_timespec(ns)
    }

    #[pyfunction(name = "_PyTime_AsTimeval_clamp")]
    fn pytime_as_timeval_clamp(ns: i64, _rnd: OptionalArg<i32>) -> (i64, i64) {
        let us = ns.div_euclid(1000);
        let sec = us.div_euclid(1_000_000);
        let usec = us.rem_euclid(1_000_000);
        (sec, usec)
    }

    #[pyfunction]
    fn test_edit_cost() {}

    #[pyfunction]
    fn hamt(vm: &VirtualMachine) -> PyRef<Hamt> {
        Hamt::default().into_ref(&vm.ctx)
    }

    #[pyclass(no_attr, name = "hamt", module = "_testinternalcapi")]
    #[derive(Default, Debug, PyPayload)]
    struct Hamt {
        buckets: HashMap<PyHash, Vec<(PyObjectRef, PyObjectRef)>>,
        len: usize,
    }

    #[pyclass(with(AsMapping))]
    impl Hamt {
        #[pymethod]
        fn set(
            &self,
            key: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let hash = key.hash(vm)?;
            let mut next = self.clone_map();
            let bucket = next.buckets.entry(hash).or_default();
            for (k, slot) in bucket.iter_mut() {
                if vm.bool_eq(k, &key)? {
                    *slot = value;
                    return Ok(next.into_ref(&vm.ctx));
                }
            }
            bucket.push((key, value));
            next.len += 1;
            Ok(next.into_ref(&vm.ctx))
        }

        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let hash = key.hash(vm)?;
            if let Some(bucket) = self.buckets.get(&hash) {
                for (k, v) in bucket {
                    if vm.bool_eq(k, &key)? {
                        return Ok(v.clone());
                    }
                }
            }
            Ok(default.unwrap_or_none(vm))
        }

        #[pymethod]
        fn delete(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let hash = key.hash(vm)?;
            let mut next = self.clone_map();
            if let Some(bucket) = next.buckets.get_mut(&hash) {
                let mut idx = None;
                for (i, (k, _)) in bucket.iter().enumerate() {
                    if vm.bool_eq(k, &key)? {
                        idx = Some(i);
                        break;
                    }
                }
                if let Some(i) = idx {
                    bucket.remove(i);
                    next.len -= 1;
                    if bucket.is_empty() {
                        next.buckets.remove(&hash);
                    }
                }
            }
            Ok(next.into_ref(&vm.ctx))
        }

        #[pymethod]
        fn keys(&self, vm: &VirtualMachine) -> crate::builtins::PyListRef {
            let keys: Vec<PyObjectRef> = self
                .buckets
                .values()
                .flatten()
                .map(|(k, _)| k.clone())
                .collect();
            vm.ctx.new_list(keys)
        }

        #[pymethod]
        fn values(&self, vm: &VirtualMachine) -> crate::builtins::PyListRef {
            let values: Vec<PyObjectRef> = self
                .buckets
                .values()
                .flatten()
                .map(|(_, v)| v.clone())
                .collect();
            vm.ctx.new_list(values)
        }

        #[pymethod]
        fn items(&self, vm: &VirtualMachine) -> crate::builtins::PyListRef {
            let items: Vec<PyObjectRef> = self
                .buckets
                .values()
                .flatten()
                .map(|(k, v)| vm.ctx.new_tuple(vec![k.clone(), v.clone()]).into())
                .collect();
            vm.ctx.new_list(items)
        }

        fn clone_map(&self) -> Self {
            Self {
                buckets: self.buckets.clone(),
                len: self.len,
            }
        }
    }

    impl AsMapping for Hamt {
        fn as_mapping() -> &'static PyMappingMethods {
            static METHODS: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(Hamt::mapping_downcast(mapping).len)),
                ..PyMappingMethods::NOT_IMPLEMENTED
            };
            &METHODS
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

fn clean_doc(doc: &str) -> String {
    let doc = expandtabs(doc, 8);
    let margin = doc
        .split('\n')
        .skip(1)
        .filter(|line| line.chars().any(|c| c != ' '))
        .map(|line| line.chars().take_while(|c| *c == ' ').count())
        .min()
        .unwrap_or(0);

    let mut cleaned = String::with_capacity(doc.len());
    if let Some(first_line) = doc.split('\n').next() {
        let trimmed = first_line.trim_start();
        if trimmed.len() == first_line.len() && margin == 0 {
            return doc.to_owned();
        }
        cleaned.push_str(trimmed);
    }
    for line in doc.split('\n').skip(1) {
        cleaned.push('\n');
        let skip = line.chars().take(margin).take_while(|c| *c == ' ').count();
        cleaned.push_str(&line[skip..]);
    }
    cleaned
}

fn expandtabs(input: &str, tab_size: usize) -> String {
    let mut expanded = String::with_capacity(input.len());
    let mut col = 0usize;
    let mut next_stop = tab_size;
    for ch in input.chars() {
        match ch {
            '\t' => {
                let n = next_stop - col;
                col += n;
                expanded.extend(core::iter::repeat_n(' ', n));
            }
            '\r' | '\n' => {
                expanded.push(ch);
                col = 0;
                next_stop = 0;
            }
            _ => {
                expanded.push(ch);
                col += 1;
            }
        }
        if col >= next_stop {
            next_stop += tab_size;
        }
    }
    expanded
}

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
