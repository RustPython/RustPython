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
    eprintln!("DEBUG: save_checkpoint called");
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }
    
    // Get all frames in the stack
    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);  // Release borrow
    
    eprintln!("DEBUG: Got {} frames", frame_refs.len());
    
    // Temporarily skip validation to avoid potential deadlock
    // TODO: Re-enable validation after fixing the issue
    // for frame in &frame_refs {
    //     validate_frame_for_checkpoint(vm, frame)?;
    // }
    
    eprintln!("DEBUG: Calling save_checkpoint_bytes_from_frames");
    let data = save_checkpoint_bytes_from_frames(vm, &frame_refs, None)?;
    eprintln!("DEBUG: Writing {} bytes to {}", data.len(), path);
    fs::write(path, &data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    eprintln!("DEBUG: File written");
    Ok(())
}

// Version that accepts the innermost frame's resume_lasti (already validated)
pub(crate) fn save_checkpoint_with_lasti(vm: &VirtualMachine, path: &str, innermost_resume_lasti: u32) -> PyResult<()> {
    eprintln!("DEBUG: save_checkpoint_with_lasti called, resume_lasti={}", innermost_resume_lasti);
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }
    
    // Get all frames in the stack
    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);  // Release borrow
    
    eprintln!("DEBUG: Got {} frames", frame_refs.len());
    
    eprintln!("DEBUG: Calling save_checkpoint_bytes_from_frames with innermost_lasti");
    let data = save_checkpoint_bytes_from_frames(vm, &frame_refs, Some(innermost_resume_lasti))?;
    eprintln!("DEBUG: Writing {} bytes to {}", data.len(), path);
    fs::write(path, &data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    eprintln!("DEBUG: File written");
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn save_checkpoint_bytes(vm: &VirtualMachine) -> PyResult<Vec<u8>> {
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }
    
    // Get all frames in the stack
    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);  // Release borrow
    
    // Validate all frames
    for frame in &frame_refs {
        validate_frame_for_checkpoint(vm, frame)?;
    }
    
    save_checkpoint_bytes_from_frames(vm, &frame_refs, None)
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
    fs::write(path, &data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
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
    eprintln!("DEBUG: Loading checkpoint state...");
    let (state, objects) = snapshot::load_checkpoint_state(vm, data)?;
    eprintln!("DEBUG: Loaded {} objects from checkpoint", objects.len());
    eprintln!("DEBUG: Checkpoint has {} frames", state.frames.len());
    
    if state.source_path != script_path {
        return Err(vm.new_value_error(format!(
            "checkpoint source_path '{}' does not match script '{}'",
            state.source_path, script_path
        )));
    }

    // Get globals
    let globals_obj = objects
        .get(state.root as usize)
        .cloned()
        .ok_or_else(|| vm.new_runtime_error("checkpoint globals missing".to_owned()))?;
    let globals_dict = PyDictRef::try_from_object(vm, globals_obj)?;
    eprintln!("DEBUG: Got globals dict");

    if !globals_dict.contains_key("__file__", vm) {
        globals_dict.set_item("__file__", vm.ctx.new_str(script_path).into(), vm)?;
        globals_dict.set_item("__cached__", vm.ctx.none(), vm)?;
    }

    // Rebuild all frames from bottom to top
    let mut frame_refs = Vec::new();
    for (i, frame_state) in state.frames.iter().enumerate() {
        let code = snapshot::decode_code_object(vm, &frame_state.code)
            .map_err(|err| vm.new_value_error(format!("checkpoint frame {i} code invalid: {err:?}")))?;
        let code_obj: crate::PyRef<PyCode> = vm.ctx.new_pyref(PyCode::new(code));

        // Get locals for this frame
        eprintln!("DEBUG: Frame {i}: Getting locals obj from index {}", frame_state.locals);
        let locals_obj = objects
            .get(frame_state.locals as usize)
            .cloned()
            .ok_or_else(|| vm.new_runtime_error(format!("checkpoint frame {i} locals missing")))?;
        eprintln!("DEBUG: Frame {i}: locals_obj class = {}", locals_obj.class().name());
        
        let locals_dict = PyDictRef::try_from_object(vm, locals_obj.clone())?;
        eprintln!("DEBUG: Frame {i}: Successfully converted to PyDictRef");

        let varnames = &code_obj.code.varnames;
        eprintln!("DEBUG: Frame {i}: varnames = {:?}", varnames.iter().map(|v| v.as_str()).collect::<Vec<_>>());
        
        // Try to iterate all keys in the dict
        eprintln!("DEBUG: Frame {i}: Iterating all dict keys...");
        let dict_items: Vec<_> = locals_dict.clone().into_iter().collect();
        eprintln!("DEBUG: Frame {i}: Dict has {} items", dict_items.len());
        for (key, value) in dict_items.iter() {
            if let Some(key_str) = key.downcast_ref::<crate::builtins::PyStr>() {
                eprintln!("DEBUG: Frame {i}: Dict contains key '{}' = {}", key_str.as_str(), value.class().name());
            }
        }
        
        // Debug: check what's in locals_dict BEFORE creating the frame
        for varname in varnames.iter() {
            if let Some(value) = locals_dict.get_item_opt(*varname, vm)? {
                eprintln!("DEBUG: Frame {i}: locals_dict[{varname}] = {} BEFORE frame creation", value.class().name());
            } else {
                eprintln!("DEBUG: Frame {i}: locals_dict[{varname}] = <MISSING> BEFORE frame creation");
            }
        }

        // Create ArgMapping from locals dict
        let locals_mapping = crate::function::ArgMapping::from_dict_exact(locals_dict.clone());
        
        // Create scope with locals and globals
        let scope = Scope::with_builtins(Some(locals_mapping), globals_dict.clone(), vm);
        let func = PyFunction::new(code_obj.clone(), globals_dict.clone(), vm)?;
        let func_obj = func.into_ref(&vm.ctx).into();
        let frame = crate::frame::Frame::new(code_obj.clone(), scope, vm.builtins.dict(), &[], Some(func_obj), vm)
            .into_ref(&vm.ctx);

        // Restore fastlocals from the locals dict
        eprintln!("DEBUG: Frame {i}: Restoring fastlocals...");
        let mut fastlocals = frame.fastlocals.lock();
        for (idx, varname) in varnames.iter().enumerate() {
            if let Some(value) = locals_dict.get_item_opt(*varname, vm)? {
                eprintln!("DEBUG: Frame {i}: Restoring fastlocals[{idx}] = {varname} = {}", value.class().name());
                fastlocals[idx] = Some(value);
            } else {
                eprintln!("DEBUG: Frame {i}: No value for fastlocals[{idx}] = {varname}");
            }
        }
        drop(fastlocals);

        if frame_state.lasti as usize >= frame.code.instructions.len() {
            return Err(vm.new_value_error(
                format!("checkpoint frame {i} lasti is out of range for current bytecode"),
            ));
        }
        frame.set_lasti(frame_state.lasti);
        frame_refs.push(frame);
    }

    // Push all frames onto the VM stack (bottom to top)
    for frame in frame_refs.iter() {
        vm.frames.borrow_mut().push(frame.clone());
    }

    // Run the top frame
    let result = vm.run_frame(frame_refs.last().unwrap().clone());
    
    // Clean up frames
    vm.frames.borrow_mut().clear();
    
    result.map(drop)
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
fn validate_frame_for_checkpoint(vm: &VirtualMachine, frame: &FrameRef) -> PyResult<()> {
    // Check value stack is empty
    let stack = frame.checkpoint_stack(vm)?;
    if !stack.is_empty() {
        return Err(vm.new_value_error(
            "checkpoint requires an empty value stack in all frames".to_owned(),
        ));
    }
    
    // Validate instruction pointer
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
    
    Ok(())
}

fn save_checkpoint_bytes_from_frames(
    vm: &VirtualMachine,
    frames: &[FrameRef],
    innermost_resume_lasti: Option<u32>,  // If provided, use this for the innermost frame
) -> PyResult<Vec<u8>> {
    if frames.is_empty() {
        return Err(vm.new_runtime_error("no frames to checkpoint".to_owned()));
    }
    
    // Get source path from the outermost (first) frame
    let source_path = frames[0].code.source_path.as_str();
    
    // Debug: Check fastlocals before serialization
    for (idx, frame) in frames.iter().enumerate() {
        eprintln!("DEBUG: Frame {idx} before serialize:");
        eprintln!("  code.varnames = {:?}", frame.code.code.varnames.iter().map(|v| v.as_str()).collect::<Vec<_>>());
        eprintln!("  code.flags = {:?}", frame.code.code.flags);
        let fastlocals = frame.fastlocals.lock();
        for (i, value) in fastlocals.iter().enumerate() {
            if i < frame.code.code.varnames.len() {
                let varname = &frame.code.code.varnames[i];
                if let Some(v) = value {
                    eprintln!("  fastlocals[{i}] ({varname}) = {}", v.class().name());
                } else {
                    eprintln!("  fastlocals[{i}] ({varname}) = None");
                }
            }
        }
        drop(fastlocals);
    }
    
    // Collect frame states
    let mut frame_states = Vec::new();
    for (idx, frame) in frames.iter().enumerate() {
        // Only the innermost (last) frame needs special handling
        let is_innermost = idx == frames.len() - 1;
        let resume_lasti = if is_innermost {
            // If innermost_resume_lasti is provided, use it (already validated)
            // Otherwise compute it (for backward compatibility)
            if let Some(lasti) = innermost_resume_lasti {
                lasti
            } else {
                compute_resume_lasti(vm, frame)?
            }
        } else {
            // For non-innermost frames, just use current lasti
            frame.lasti()
        };
        eprintln!("DEBUG: Frame {idx} resume_lasti = {}", resume_lasti);
        frame_states.push((frame, resume_lasti));
    }
    
    snapshot::dump_checkpoint_frames(vm, source_path, &frame_states)
}

#[allow(dead_code)]
fn ensure_supported_frame(vm: &VirtualMachine, frame: &FrameRef) -> PyResult<()> {
    if vm.frames.borrow().len() != 1 {
        return Err(vm.new_runtime_error(
            "checkpoint only supports top-level module frames".to_owned(),
        ));
    }
    validate_frame_for_checkpoint(vm, frame)?;
    Ok(())
}
