use crate::{
    PyPayload, PyResult, VirtualMachine,
    builtins::{PyDictRef, code::PyCode},
    convert::TryFromObject,
    frame::FrameRef,
    scope::Scope,
    vm::snapshot,
};
use crate::bytecode;
use crate::builtins::function::PyFunction;
use std::fs;

#[allow(dead_code)]
pub(crate) fn save_checkpoint(vm: &VirtualMachine, path: &str) -> PyResult<()> {
    let frame = vm
        .current_frame()
        .ok_or_else(|| vm.new_runtime_error("checkpoint requires an active frame".to_owned()))?;
    let frame = frame.to_owned();

    ensure_supported_frame(vm, &frame)?;
    let resume_lasti = compute_resume_lasti(vm, &frame)?;

    let stack = frame.checkpoint_stack(vm)?;
    if !stack.is_empty() {
        return Err(vm.new_value_error(
            "checkpoint requires an empty value stack".to_owned(),
        ));
    }
    let data = save_checkpoint_bytes_from_exec(
        vm,
        frame.code.source_path.as_str(),
        resume_lasti,
        &frame.code,
        &frame.globals,
    )?;
    fs::write(path, data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn save_checkpoint_bytes(vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let frame = vm
        .current_frame()
        .ok_or_else(|| vm.new_runtime_error("checkpoint requires an active frame".to_owned()))?;
    let frame = frame.to_owned();

    ensure_supported_frame(vm, &frame)?;
    let resume_lasti = compute_resume_lasti(vm, &frame)?;

    let stack = frame.checkpoint_stack(vm)?;
    if !stack.is_empty() {
        return Err(vm.new_value_error(
            "checkpoint requires an empty value stack".to_owned(),
        ));
    }
    save_checkpoint_bytes_from_exec(
        vm,
        frame.code.source_path.as_str(),
        resume_lasti,
        &frame.code,
        &frame.globals,
    )
}

pub(crate) fn save_checkpoint_from_exec(
    vm: &VirtualMachine,
    source_path: &str,
    lasti: u32,
    code: &PyCode,
    globals: &PyDictRef,
    path: &str,
) -> PyResult<()> {
    let data = save_checkpoint_bytes_from_exec(vm, source_path, lasti, code, globals)?;
    fs::write(path, data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    Ok(())
}

pub(crate) fn save_checkpoint_bytes_from_exec(
    vm: &VirtualMachine,
    source_path: &str,
    lasti: u32,
    code: &PyCode,
    globals: &PyDictRef,
) -> PyResult<Vec<u8>> {
    snapshot::dump_checkpoint_state(vm, source_path, lasti, code, globals)
}

pub(crate) fn resume_script_from_checkpoint(
    vm: &VirtualMachine,
    _scope: Scope,
    script_path: &str,
    checkpoint_path: &str,
) -> PyResult<()> {
    let data = fs::read(checkpoint_path)
        .map_err(|err| vm.new_os_error(format!("checkpoint read failed: {err}")))?;
    resume_script_from_bytes(vm, script_path, &data)
}

pub(crate) fn resume_script_from_bytes(
    vm: &VirtualMachine,
    script_path: &str,
    data: &[u8],
) -> PyResult<()> {
    let (state, objects) = snapshot::load_checkpoint_state(vm, data)?;
    if state.source_path != script_path {
        return Err(vm.new_value_error(format!(
            "checkpoint source_path '{}' does not match script '{}'",
            state.source_path, script_path
        )));
    }

    let code = snapshot::decode_code_object(vm, &state.code)
        .map_err(|err| vm.new_value_error(format!("checkpoint code invalid: {err:?}")))?;
    let code_obj: crate::PyRef<PyCode> = vm.ctx.new_pyref(PyCode::new(code));

    let globals_obj = objects
        .get(state.root as usize)
        .cloned()
        .ok_or_else(|| vm.new_runtime_error("checkpoint globals missing".to_owned()))?;
    let module_dict = PyDictRef::try_from_object(vm, globals_obj)?;

    if !module_dict.contains_key("__file__", vm) {
        module_dict.set_item("__file__", vm.ctx.new_str(script_path).into(), vm)?;
        module_dict.set_item("__cached__", vm.ctx.none(), vm)?;
    }

    let scope = Scope::with_builtins(None, module_dict.clone(), vm);
    let func = PyFunction::new(code_obj.clone(), module_dict, vm)?;
    let func_obj = func.into_ref(&vm.ctx).into();
    let frame = crate::frame::Frame::new(code_obj, scope, vm.builtins.dict(), &[], Some(func_obj), vm)
        .into_ref(&vm.ctx);

    if state.lasti as usize >= frame.code.instructions.len() {
        return Err(vm.new_value_error(
            "checkpoint lasti is out of range for current bytecode".to_owned(),
        ));
    }
    frame.set_lasti(state.lasti);
    vm.run_frame(frame).map(drop)
}

#[allow(dead_code)]
fn compute_resume_lasti(vm: &VirtualMachine, frame: &FrameRef) -> PyResult<u32> {
    let lasti = frame.lasti();
    let next = frame
        .code
        .instructions
        .get(lasti as usize)
        .ok_or_else(|| vm.new_runtime_error("checkpoint out of range".to_owned()))?;
    if next.op != bytecode::Instruction::PopTop {
        return Err(vm.new_value_error(
            "checkpoint() must be used as a standalone statement".to_owned(),
        ));
    }
    lasti
        .checked_add(1)
        .ok_or_else(|| vm.new_runtime_error("checkpoint lasti overflow".to_owned()))
}

#[allow(dead_code)]
fn ensure_supported_frame(vm: &VirtualMachine, frame: &FrameRef) -> PyResult<()> {
    if vm.frames.borrow().len() != 1 {
        return Err(vm.new_runtime_error(
            "checkpoint only supports top-level module frames".to_owned(),
        ));
    }
    if frame.code.flags.contains(bytecode::CodeFlags::IS_OPTIMIZED) {
        return Err(vm.new_runtime_error(
            "checkpoint does not support optimized locals".to_owned(),
        ));
    }
    if !frame.code.cellvars.is_empty() || !frame.code.freevars.is_empty() {
        return Err(vm.new_runtime_error(
            "checkpoint does not support closures/freevars".to_owned(),
        ));
    }
    Ok(())
}
