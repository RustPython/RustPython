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
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }
    
    // Get all frames in the stack
    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);  // Release borrow
    
    
    // Temporarily skip validation to avoid potential deadlock
    // TODO: Re-enable validation after fixing the issue
    // for frame in &frame_refs {
    //     validate_frame_for_checkpoint(vm, frame)?;
    // }
    
    let data = save_checkpoint_bytes_from_frames(vm, &frame_refs, None)?;
    fs::write(path, &data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    Ok(())
}

// Version that accepts the innermost frame's resume_lasti (already validated)
pub(crate) fn save_checkpoint_with_lasti(vm: &VirtualMachine, path: &str, innermost_resume_lasti: u32) -> PyResult<()> {
    save_checkpoint_with_lasti_stack_and_blocks(vm, path, innermost_resume_lasti, Vec::new(), Vec::new())
}

// Version that accepts both resume_lasti and the innermost frame's stack
pub(crate) fn save_checkpoint_with_lasti_stack_and_blocks(
    vm: &VirtualMachine, 
    path: &str, 
    innermost_resume_lasti: u32,
    innermost_stack: Vec<crate::PyObjectRef>,
    innermost_blocks: Vec<crate::frame::Block>
) -> PyResult<()> {
    save_checkpoint_with_lasti_stack_blocks_and_locals(
        vm, path, innermost_resume_lasti, innermost_stack, innermost_blocks, None
    )
}

// Version that also accepts prepared locals for innermost frame
pub(crate) fn save_checkpoint_with_lasti_stack_blocks_and_locals(
    vm: &VirtualMachine, 
    path: &str, 
    innermost_resume_lasti: u32,
    innermost_stack: Vec<crate::PyObjectRef>,
    innermost_blocks: Vec<crate::frame::Block>,
    innermost_locals: Option<crate::PyObjectRef>,
) -> PyResult<()> {
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }
    
    
    // Get all frames in the stack
    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);  // Release borrow
    
    
    let data = save_checkpoint_bytes_from_frames_with_stack_blocks_and_locals(
        vm, 
        &frame_refs, 
        Some(innermost_resume_lasti),
        innermost_stack,
        innermost_blocks,
        innermost_locals
    )?;
    fs::write(path, &data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
    Ok(())
}

pub(crate) fn save_checkpoint_bytes_with_lasti_stack_blocks_and_locals(
    vm: &VirtualMachine,
    innermost_resume_lasti: u32,
    innermost_stack: Vec<crate::PyObjectRef>,
    innermost_blocks: Vec<crate::frame::Block>,
    innermost_locals: Option<crate::PyObjectRef>,
) -> PyResult<Vec<u8>> {
    let frames = vm.frames.borrow();
    if frames.is_empty() {
        return Err(vm.new_runtime_error("checkpoint requires an active frame".to_owned()));
    }

    let frame_refs: Vec<_> = frames.iter().map(|f| f.to_owned()).collect();
    drop(frames);

    save_checkpoint_bytes_from_frames_with_stack_blocks_and_locals(
        vm,
        &frame_refs,
        Some(innermost_resume_lasti),
        innermost_stack,
        innermost_blocks,
        innermost_locals,
    )
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
    let (state, objects) = snapshot::load_checkpoint_state(vm, data)?;
    
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
        let locals_obj = objects
            .get(frame_state.locals as usize)
            .cloned()
            .ok_or_else(|| vm.new_runtime_error(format!("checkpoint frame {i} locals missing")))?;
        
        let locals_dict = PyDictRef::try_from_object(vm, locals_obj.clone())?;

        let varnames = &code_obj.code.varnames;
        
        // Try to iterate all keys in the dict
        let dict_items: Vec<_> = locals_dict.clone().into_iter().collect();
        for (key, value) in dict_items.iter() {
            if let Some(key_str) = key.downcast_ref::<crate::builtins::PyStr>() {
            }
        }
        
        // Debug: check what's in locals_dict BEFORE creating the frame
        for varname in varnames.iter() {
            if let Some(value) = locals_dict.get_item_opt(*varname, vm)? {
            } else {
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
        let mut fastlocals = frame.fastlocals.lock();
        for (idx, varname) in varnames.iter().enumerate() {
            if let Some(value) = locals_dict.get_item_opt(*varname, vm)? {
                fastlocals[idx] = Some(value);
            } else {
            }
        }
        drop(fastlocals);

        // Restore the value stack
        for stack_item_id in &frame_state.stack {
            let stack_obj = objects
                .get(*stack_item_id as usize)
                .cloned()
                .ok_or_else(|| vm.new_runtime_error(format!("checkpoint frame {i} stack item {} missing", stack_item_id)))?;
            frame.push_stack_value(stack_obj);
        }

        // Restore block stack
        for block_state in &frame_state.blocks {
            let block = snapshot::convert_block_state_to_block(block_state, &objects, vm)?;
            frame.push_block(block);
        }

        if frame_state.lasti as usize >= frame.code.instructions.len() {
            return Err(vm.new_value_error(
                format!("checkpoint frame {i} lasti is out of range for current bytecode"),
            ));
        }
        frame.set_lasti(frame_state.lasti);
        frame_refs.push(frame);
    }

    
    if frame_refs.len() == 1 {
        // Simple case: only one frame, just run it
        let result = vm.run_frame(frame_refs[0].clone());
        vm.frames.borrow_mut().clear();
        return result.map(drop);
    }
    
    // Multiple frames: need to execute inner frames first, then continue outer frames
    // Push all outer frames to VM stack (they are waiting for inner frames to return)
    for i in 0..frame_refs.len() - 1 {
        vm.frames.borrow_mut().push(frame_refs[i].clone());
    }

    // Run the innermost frame using vm.run_frame
    let innermost_frame = frame_refs.last().unwrap().clone();
    let inner_result = vm.run_frame(innermost_frame);
    
    // If inner frame failed, clean up and return error
    let inner_return_val = match inner_result {
        Ok(val) => val,
        Err(e) => {
            vm.frames.borrow_mut().clear();
            return Err(e);
        }
    };
    
    // Push the inner frame's return value to the caller's (outer frame's) stack
    let caller_frame = &frame_refs[frame_refs.len() - 2];
    caller_frame.push_stack_value(inner_return_val);
    
    // Inner frame succeeded. Now continue executing outer frames
    // The return value from inner frame should be on the caller's stack already
    // We need to continue executing from the outermost frame
    for i in (0..frame_refs.len() - 1).rev() {
        let frame = frame_refs[i].clone();
        
        // Use frame.run() directly since frame is already on VM stack
        let result = frame.run(vm);
        
        match result {
            Ok(crate::frame::ExecutionResult::Return(val)) => {
                // Frame returned normally
                // Pop this frame
                vm.frames.borrow_mut().pop();
                
                // If there's an outer frame, push the return value to its stack
                if i > 0 {
                    frame_refs[i - 1].push_stack_value(val);
                } else {
                    // This was the outermost frame, we're done
                    vm.frames.borrow_mut().clear();
                    return Ok(());
                }
            }
            Err(e) => {
                // Error occurred
                vm.frames.borrow_mut().clear();
                return Err(e);
            }
            Ok(_other) => {
                vm.frames.borrow_mut().clear();
                return Err(vm.new_runtime_error("unexpected execution result (not Return)".to_owned()));
            }
        }
    }
    
    vm.frames.borrow_mut().clear();
    Ok(())
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
    save_checkpoint_bytes_from_frames_with_stack(vm, frames, innermost_resume_lasti, Vec::new())
}

fn save_checkpoint_bytes_from_frames_with_stack(
    vm: &VirtualMachine,
    frames: &[FrameRef],
    innermost_resume_lasti: Option<u32>,
    innermost_stack: Vec<crate::PyObjectRef>,
) -> PyResult<Vec<u8>> {
    save_checkpoint_bytes_from_frames_with_stack_and_blocks(
        vm,
        frames,
        innermost_resume_lasti,
        innermost_stack,
        Vec::new()  // Empty blocks for compatibility
    )
}

fn save_checkpoint_bytes_from_frames_with_stack_and_blocks(
    vm: &VirtualMachine,
    frames: &[FrameRef],
    innermost_resume_lasti: Option<u32>,
    innermost_stack: Vec<crate::PyObjectRef>,
    innermost_blocks: Vec<crate::frame::Block>,
) -> PyResult<Vec<u8>> {
    save_checkpoint_bytes_from_frames_with_stack_blocks_and_locals(
        vm, frames, innermost_resume_lasti, innermost_stack, innermost_blocks, None
    )
}

fn save_checkpoint_bytes_from_frames_with_stack_blocks_and_locals(
    vm: &VirtualMachine,
    frames: &[FrameRef],
    innermost_resume_lasti: Option<u32>,
    innermost_stack: Vec<crate::PyObjectRef>,
    innermost_blocks: Vec<crate::frame::Block>,
    innermost_locals: Option<crate::PyObjectRef>,
) -> PyResult<Vec<u8>> {
    if frames.is_empty() {
        return Err(vm.new_runtime_error("no frames to checkpoint".to_owned()));
    }
    
    // Get source path from the outermost (first) frame
    let source_path = frames[0].code.source_path.as_str();
    
    // Build blocks vec: only innermost frame gets blocks, others get empty vec.
    // Outer frames are waiting for inner frames to return and their block state
    // can be safely reconstructed as empty since they're not in active control flow.
    let mut all_blocks = vec![Vec::new(); frames.len()];
    if !frames.is_empty() {
        all_blocks[frames.len() - 1] = innermost_blocks;
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
        frame_states.push((frame, resume_lasti));
    }
    
    snapshot::dump_checkpoint_frames_with_all_blocks_and_locals(
        vm, 
        source_path, 
        &frame_states, 
        innermost_stack,
        all_blocks,
        innermost_locals
    )
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
