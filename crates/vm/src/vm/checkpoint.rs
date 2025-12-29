use crate::{
    PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{PyBytesRef, PyDictRef},
    compiler,
    convert::TryFromObject,
    frame::FrameRef,
    scope::Scope,
};
use crate::AsObject;
use crate::bytecode;
use crate::builtins::function::PyFunction;
use std::fs;

const CHECKPOINT_VERSION: u32 = 1;

struct CheckpointSnapshot {
    source_path: String,
    lasti: u32,
    stack: Vec<PyObjectRef>,
    globals: Vec<(String, PyObjectRef)>,
}

impl CheckpointSnapshot {
    fn to_pydict(&self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        let payload = vm.ctx.new_dict();
        payload.set_item(
            "version",
            vm.ctx.new_int(CHECKPOINT_VERSION).into(),
            vm,
        )?;
        payload.set_item(
            "source_path",
            vm.ctx.new_str(self.source_path.clone()).into(),
            vm,
        )?;
        payload.set_item("lasti", vm.ctx.new_int(self.lasti).into(), vm)?;
        payload.set_item("stack", vm.ctx.new_list(self.stack.clone()).into(), vm)?;

        let globals = vm.ctx.new_dict();
        for (key, value) in &self.globals {
            globals.set_item(key.as_str(), value.clone(), vm)?;
        }
        payload.set_item("globals", globals.into(), vm)?;
        Ok(payload)
    }

    fn from_pydict(vm: &VirtualMachine, dict: PyDictRef) -> PyResult<Self> {
        let version: u32 = dict.get_item("version", vm)?.try_into_value(vm)?;
        if version != CHECKPOINT_VERSION {
            return Err(vm.new_value_error(format!(
                "unsupported checkpoint version: {version}"
            )));
        }

        let source_path: String = dict.get_item("source_path", vm)?.try_into_value(vm)?;
        let lasti: u32 = dict.get_item("lasti", vm)?.try_into_value(vm)?;
        let stack: Vec<PyObjectRef> = dict.get_item("stack", vm)?.try_into_value(vm)?;

        let globals_obj = dict.get_item("globals", vm)?;
        let globals_dict = PyDictRef::try_from_object(vm, globals_obj)?;
        let mut globals = Vec::new();
        for (key, value) in &globals_dict {
            let key = key
                .downcast_ref::<crate::builtins::PyStr>()
                .ok_or_else(|| vm.new_type_error("checkpoint globals key must be str".to_owned()))?;
            globals.push((key.as_str().to_owned(), value));
        }

        Ok(Self {
            source_path,
            lasti,
            stack,
            globals,
        })
    }
}

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
    let globals = extract_globals_from_dict(vm, &frame.globals)?;

    let snapshot = CheckpointSnapshot {
        source_path: frame.code.source_path.as_str().to_owned(),
        lasti: resume_lasti,
        stack: Vec::new(),
        globals,
    };

    write_snapshot(vm, path, snapshot)?;
    Ok(())
}

pub(crate) fn save_checkpoint_from_exec(
    vm: &VirtualMachine,
    source_path: &str,
    lasti: u32,
    globals: &PyDictRef,
    path: &str,
) -> PyResult<()> {
    let globals = extract_globals_from_dict(vm, globals)?;
    let snapshot = CheckpointSnapshot {
        source_path: source_path.to_owned(),
        lasti,
        stack: Vec::new(),
        globals,
    };
    write_snapshot(vm, path, snapshot)
}

pub(crate) fn resume_script_from_checkpoint(
    vm: &VirtualMachine,
    scope: Scope,
    script_path: &str,
    checkpoint_path: &str,
) -> PyResult<()> {
    let snapshot = load_checkpoint(vm, checkpoint_path)?;
    if snapshot.source_path != script_path {
        return Err(vm.new_value_error(format!(
            "checkpoint source_path '{}' does not match script '{}'",
            snapshot.source_path, script_path
        )));
    }

    let source = fs::read_to_string(script_path)
        .map_err(|err| vm.new_os_error(format!("failed reading script '{script_path}': {err}")))?;

    let code_obj = vm
        .compile(&source, compiler::Mode::Exec, script_path.to_owned())
        .map_err(|err| vm.new_syntax_error(&err, Some(&source)))?;

    let module_dict = scope.globals.clone();
    if !module_dict.contains_key("__file__", vm) {
        module_dict.set_item("__file__", vm.ctx.new_str(script_path).into(), vm)?;
        module_dict.set_item("__cached__", vm.ctx.none(), vm)?;
    }

    for (key, value) in snapshot.globals {
        module_dict.set_item(key.as_str(), value, vm)?;
    }

    let scope = Scope::with_builtins(None, module_dict.clone(), vm);
    let func = PyFunction::new(code_obj.clone(), module_dict, vm)?;
    let func_obj = func.into_ref(&vm.ctx).into();
    let frame = crate::frame::Frame::new(code_obj, scope, vm.builtins.dict(), &[], Some(func_obj), vm)
        .into_ref(&vm.ctx);

    if snapshot.lasti as usize >= frame.code.instructions.len() {
        return Err(vm.new_value_error(
            "checkpoint lasti is out of range for current bytecode".to_owned(),
        ));
    }
    frame.set_lasti(snapshot.lasti);
    frame.restore_stack(snapshot.stack, vm)?;
    vm.run_frame(frame).map(drop)
}

fn load_checkpoint(vm: &VirtualMachine, path: &str) -> PyResult<CheckpointSnapshot> {
    let data = fs::read(path)
        .map_err(|err| vm.new_os_error(format!("checkpoint read failed: {err}")))?;
    let payload = marshal_loads(vm, data)?;
    let dict = PyDictRef::try_from_object(vm, payload)?;
    CheckpointSnapshot::from_pydict(vm, dict)
}

fn write_snapshot(vm: &VirtualMachine, path: &str, snapshot: CheckpointSnapshot) -> PyResult<()> {
    let payload = snapshot.to_pydict(vm)?;
    let data = marshal_dumps(vm, payload.into())?;
    fs::write(path, data).map_err(|err| vm.new_os_error(format!("checkpoint write failed: {err}")))?;
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

fn extract_globals_from_dict(
    vm: &VirtualMachine,
    dict: &PyDictRef,
) -> PyResult<Vec<(String, PyObjectRef)>> {
    let mut globals: Vec<(String, PyObjectRef)> = Vec::new();
    for (key, value) in dict {
        let key = key
            .downcast_ref::<crate::builtins::PyStr>()
            .ok_or_else(|| vm.new_type_error("checkpoint globals key must be str".to_owned()))?;
        let key_str = key.as_str();
        if key_str.starts_with("__") {
            continue;
        }
        if !is_marshaled_value(&value, vm) {
            continue;
        }
        globals.push((key_str.to_owned(), value));
    }
    Ok(globals)
}

fn is_marshaled_value(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
    if vm.is_none(obj) {
        return true;
    }
    obj.fast_isinstance(vm.ctx.types.int_type)
        || obj.fast_isinstance(vm.ctx.types.bool_type)
        || obj.fast_isinstance(vm.ctx.types.float_type)
        || obj.fast_isinstance(vm.ctx.types.complex_type)
        || obj.fast_isinstance(vm.ctx.types.str_type)
        || obj.fast_isinstance(vm.ctx.types.bytes_type)
        || obj.fast_isinstance(vm.ctx.types.bytearray_type)
        || obj.fast_isinstance(vm.ctx.types.list_type)
        || obj.fast_isinstance(vm.ctx.types.tuple_type)
        || obj.fast_isinstance(vm.ctx.types.dict_type)
        || obj.fast_isinstance(vm.ctx.types.set_type)
        || obj.fast_isinstance(vm.ctx.types.frozenset_type)
        || obj.fast_isinstance(vm.ctx.types.ellipsis_type)
}

fn marshal_dumps(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Vec<u8>> {
    let marshal = vm.import("marshal", 0)?;
    let dumps = marshal.get_attr("dumps", vm)?;
    let data = dumps.call((obj,), vm)?;
    let data: PyBytesRef = data.downcast().map_err(|_| {
        vm.new_type_error("marshal.dumps did not return bytes".to_owned())
    })?;
    Ok(data.as_bytes().to_vec())
}

fn marshal_loads(vm: &VirtualMachine, data: Vec<u8>) -> PyResult<PyObjectRef> {
    let marshal = vm.import("marshal", 0)?;
    let loads = marshal.get_attr("loads", vm)?;
    let data: PyObjectRef = vm.ctx.new_bytes(data).into();
    loads.call((data,), vm)
}
