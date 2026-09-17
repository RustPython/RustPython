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
    use crate::{PyObjectRef, PyResult, VirtualMachine, builtins::PyDictRef};

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
}
