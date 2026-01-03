use crate::{
    AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{
        PyClassMethod, PyDictRef, PyFloat, PyInt, PyList, PyModule, PyStaticMethod, PyStr, PyTuple,
        PyWeak,
        code::{PyCode, CodeObject, PyObjBag},
        dict::PyDict,
        function::{PyCell, PyFunction},
        set::{PyFrozenSet, PySet},
        type_::PyType,
    },
    convert::TryFromObject,
    protocol::PyIterReturn,
};
use rustpython_compiler_core::marshal;
use rustpython_compiler_core::bytecode;
use std::collections::HashMap;

// Block conversion functions are defined at the end of this file

pub(crate) type ObjId = u32;

const SNAPSHOT_VERSION: u32 = 3;

#[derive(Debug)]
pub(crate) struct CheckpointState {
    pub version: u32,
    pub source_path: String,
    pub frames: Vec<FrameState>,  // Frame stack (outermost first)
    pub root: ObjId,               // Global namespace
    pub objects: Vec<ObjectEntry>,
}

#[derive(Debug)]
pub(crate) struct FrameState {
    pub code: Vec<u8>,      // Marshaled code object
    pub lasti: u32,         // Instruction pointer
    pub locals: ObjId,      // Local variables dict
    pub stack: Vec<ObjId>,  // Value stack (for loop iterators, etc.)
    pub blocks: Vec<BlockState>, // Block stack (for loops, try/except)
}

impl Default for FrameState {
    fn default() -> Self {
        Self {
            code: Vec::new(),
            lasti: 0,
            locals: 0,
            stack: Vec::new(),
            blocks: Vec::new(),
        }
    }
}

/// Serializable representation of a block stack entry
#[derive(Debug, Clone)]
pub(crate) struct BlockState {
    pub typ: BlockTypeState,
    pub level: usize,
}

/// Serializable representation of block types
#[derive(Debug, Clone)]
pub(crate) enum BlockTypeState {
    Loop,
    TryExcept { handler: u32 },
    Finally { handler: u32 },
    FinallyHandler {
        reason: Option<UnwindReasonState>,
        prev_exc: Option<ObjId>,
    },
    ExceptHandler {
        prev_exc: Option<ObjId>,
    },
}

/// Serializable representation of unwind reasons
#[derive(Debug, Clone)]
pub(crate) enum UnwindReasonState {
    Returning { value: ObjId },
    Raising { exception: ObjId },
    Break { target: u32 },
    Continue { target: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ObjTag {
    None = 0,
    Bool = 1,
    Int = 2,
    Float = 3,
    Str = 4,
    Bytes = 5,
    List = 6,
    Tuple = 7,
    Dict = 8,
    Set = 9,
    FrozenSet = 10,
    Module = 11,
    Function = 12,
    Code = 13,
    Type = 14,
    BuiltinType = 15,
    Instance = 16,
    Cell = 17,
    BuiltinModule = 18,
    BuiltinDict = 19,
    BuiltinFunction = 20,
    Enumerate = 21,
    Zip = 22,
    Map = 23,
    Filter = 24,
    ListIterator = 25,
    RangeIterator = 26,
    Range = 27,
}

#[derive(Debug)]
pub(crate) struct ObjectEntry {
    tag: ObjTag,
    payload: ObjectPayload,
}

#[derive(Debug)]
enum ObjectPayload {
    None,
    Bool(bool),
    Int(String),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<ObjId>),
    Tuple(Vec<ObjId>),
    Dict(Vec<(ObjId, ObjId)>),
    Set(Vec<ObjId>),
    FrozenSet(Vec<ObjId>),
    Module { name: String, dict: ObjId },
    BuiltinModule { name: String },
    BuiltinDict { name: String },
    Function(FunctionPayload),
    Enumerate { iterator: ObjId, count: i64 },
    Zip { iterators: Vec<ObjId> },
    Map { function: ObjId, iterator: ObjId },
    Filter { function: ObjId, iterator: ObjId },
    ListIterator { list: ObjId, position: usize },
    RangeIterator { range: ObjId, position: usize },
    Range { start: i64, stop: i64, step: i64 },
    BuiltinFunction(BuiltinFunctionPayload),
    Code(Vec<u8>),
    Type(TypePayload),
    BuiltinType { module: String, name: String },
    Instance(InstancePayload),
    Cell(Option<ObjId>),
}

#[derive(Debug)]
struct FunctionPayload {
    code: ObjId,
    globals: ObjId,
    defaults: Option<ObjId>,
    kwdefaults: Option<ObjId>,
    closure: Option<ObjId>,
    name: ObjId,
    qualname: ObjId,
    annotations: ObjId,
    module: ObjId,
    doc: ObjId,
    type_params: ObjId,
}

#[derive(Debug)]
struct TypePayload {
    name: String,
    qualname: String,
    bases: Vec<ObjId>,
    dict: ObjId,
    flags: u64,
    basicsize: usize,
    itemsize: usize,
    member_count: usize,
}

#[derive(Debug)]
struct InstancePayload {
    typ: ObjId,
    state: Option<ObjId>,
    new_args: Option<ObjId>,
    new_kwargs: Option<ObjId>,
}

#[derive(Debug)]
struct BuiltinFunctionPayload {
    name: String,
    module: Option<String>,
    self_obj: Option<ObjId>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum SnapshotError {
    Message(String),
}

impl SnapshotError {
    fn msg(msg: impl Into<String>) -> Self {
        Self::Message(msg.into())
    }
}

pub(crate) fn dump_checkpoint_frames(
    vm: &VirtualMachine,
    source_path: &str,
    frames: &[(&crate::frame::FrameRef, u32)],  // (frame, resume_lasti)
) -> PyResult<Vec<u8>> {
    dump_checkpoint_frames_with_stack(vm, source_path, frames, Vec::new())
}

pub(crate) fn dump_checkpoint_frames_with_stack(
    vm: &VirtualMachine,
    source_path: &str,
    frames: &[(&crate::frame::FrameRef, u32)],  // (frame, resume_lasti)
    innermost_stack: Vec<crate::PyObjectRef>,  // Stack of the innermost frame
) -> PyResult<Vec<u8>> {
    dump_checkpoint_frames_with_stack_and_blocks(
        vm,
        source_path,
        frames,
        innermost_stack,
        Vec::new()  // Empty blocks for compatibility
    )
}

pub(crate) fn dump_checkpoint_frames_with_stack_and_blocks(
    vm: &VirtualMachine,
    source_path: &str,
    frames: &[(&crate::frame::FrameRef, u32)],  // (frame, resume_lasti)
    innermost_stack: Vec<crate::PyObjectRef>,  // Stack of the innermost frame
    innermost_blocks: Vec<crate::frame::Block>,  // Blocks of the innermost frame
) -> PyResult<Vec<u8>> {
    // Build blocks vec with innermost blocks
    let mut all_blocks = vec![Vec::new(); frames.len()];
    if !frames.is_empty() {
        all_blocks[frames.len() - 1] = innermost_blocks;
    }
    dump_checkpoint_frames_with_all_blocks(vm, source_path, frames, innermost_stack, all_blocks)
}

pub(crate) fn dump_checkpoint_frames_with_all_blocks(
    vm: &VirtualMachine,
    source_path: &str,
    frames: &[(&crate::frame::FrameRef, u32)],  // (frame, resume_lasti)
    innermost_stack: Vec<crate::PyObjectRef>,  // Stack of the innermost frame
    all_blocks: Vec<Vec<crate::frame::Block>>,  // Blocks for all frames
) -> PyResult<Vec<u8>> {
    dump_checkpoint_frames_with_all_blocks_and_locals(
        vm, source_path, frames, innermost_stack, all_blocks, None
    )
}

pub(crate) fn dump_checkpoint_frames_with_all_blocks_and_locals(
    vm: &VirtualMachine,
    source_path: &str,
    frames: &[(&crate::frame::FrameRef, u32)],  // (frame, resume_lasti)
    innermost_stack: Vec<crate::PyObjectRef>,  // Stack of the innermost frame
    all_blocks: Vec<Vec<crate::frame::Block>>,  // Blocks for all frames
    innermost_locals: Option<crate::PyObjectRef>,  // Pre-prepared locals for innermost frame
) -> PyResult<Vec<u8>> {
    use crate::builtins::PyDictRef;
    
    // STEP 1: Prepare all locals dicts BEFORE creating SnapshotWriter
    let mut locals_dicts = Vec::new();
    for (idx, (frame, _resume_lasti)) in frames.iter().enumerate() {
        let is_innermost = idx == frames.len() - 1;
        let locals_dict = if idx == 0 {
            // For module-level frame (first frame), use globals as locals
            // This ensures that module-level variables defined during execution are captured
            frame.globals.clone()
        } else if is_innermost && innermost_locals.is_some() {
            // For innermost frame, use pre-prepared locals to avoid deadlock
            PyDictRef::try_from_object(vm, innermost_locals.clone().unwrap())?
        } else {
            // For other function frames, create a new dict and copy fastlocals
            // This is safe because these frames are not actively executing
            let locals_dict = vm.ctx.new_dict();
            
            // Copy fastlocals into the new dict
            let varnames = &frame.code.code.varnames;
            let fastlocals = frame.fastlocals.lock();
            for (idx, varname) in varnames.iter().enumerate() {
                if let Some(value) = &fastlocals[idx] {
                    locals_dict.set_item(*varname, value.clone(), vm)?;
                }
            }
            drop(fastlocals);
            
            // Also copy cell/free vars if any
            if !frame.code.code.cellvars.is_empty() || !frame.code.code.freevars.is_empty() {
                let all_vars = frame.code.code.cellvars.iter().chain(frame.code.code.freevars.iter());
                for (idx, varname) in all_vars.enumerate() {
                    if let Some(cell) = frame.cells_frees.get(idx) {
                        if let Some(value) = cell.get() {
                            locals_dict.set_item(*varname, value, vm)?;
                        }
                    }
                }
            }
            
            locals_dict
        };
        
        locals_dicts.push(locals_dict);
    }
    
    // STEP 2: Collect value stacks from all frames
    let mut stack_items = Vec::new();
    for (idx, (_frame, _resume_lasti)) in frames.iter().enumerate() {
        let is_innermost = idx == frames.len() - 1;
        let stack_result = if is_innermost {
            // Use the provided stack for the innermost frame
            innermost_stack.clone()
        } else {
            // For outer frames, use empty stack
            // They are waiting for inner frames to return, and their stack state
            // will be reconstructed during resume (return value will be pushed)
            Vec::new()
        };
        stack_items.push(stack_result);
    }
    
    // STEP 3: Create writer and do a SINGLE serialization pass
    // Get globals (from the first frame)
    let globals = &frames[0].0.globals;
    
    // STEP 3: Create writer and do a SINGLE serialization pass
    // Create a container tuple that holds: globals, all locals dicts, and all stack lists
    let mut container_items = vec![globals.clone().into()];
    for locals_dict in locals_dicts.iter() {
        container_items.push(locals_dict.clone().into());
    }
    for stack in stack_items.iter() {
        let stack_list = vm.ctx.new_list(stack.clone());
        container_items.push(stack_list.into());
    }
    let container = vm.ctx.new_tuple(container_items);
    
    // Now serialize the container (this will serialize everything in one pass)
    let mut writer = SnapshotWriter::new(vm);
    let container_obj = container.into();
    let _container_id = writer.serialize_obj(&container_obj).map_err(|err| {
        vm.new_value_error(format!("checkpoint snapshot failed: {err:?}"))
    })?;
    
    // Now get the IDs for globals and each locals dict
    let globals_obj = globals.as_object().to_owned();
    let root = writer.get_id(&globals_obj).map_err(|err| {
        vm.new_value_error(format!("globals not found: {err:?}"))
    })?;
    
    // Build frame states with correct locals IDs and stack IDs
    let mut frame_states = Vec::new();
    for (_idx, (((frame, resume_lasti), locals_dict), stack)) in 
        frames.iter().zip(locals_dicts.iter()).zip(stack_items.iter()).enumerate() {
        let code_bytes = serialize_code_object(&frame.code.code);
        
        let locals_obj = locals_dict.clone().into();
        let locals_id = writer.get_id(&locals_obj).map_err(|err| {
            vm.new_value_error(format!("frame {} locals not found: {err:?}", _idx))
        })?;
        
        // Get IDs for all stack items
        let mut stack_ids = Vec::new();
        for stack_item in stack.iter() {
            let item_id = writer.get_id(stack_item).map_err(|err| {
                vm.new_value_error(format!("frame {} stack item not found: {err:?}", _idx))
            })?;
            stack_ids.push(item_id);
        }
        
        // Get blocks from frame
        let blocks = all_blocks.get(_idx).cloned().unwrap_or_else(Vec::new);
        
        // Convert blocks to BlockState for serialization
        let mut block_states = Vec::new();
        for (_block_idx, block) in blocks.iter().enumerate() {
            let block_state = convert_block_to_state(block, &writer)?;
            block_states.push(block_state);
        }
        
        frame_states.push(FrameState {
            code: code_bytes,
            lasti: *resume_lasti,
            locals: locals_id,
            stack: stack_ids,
            blocks: block_states,
        });
    }
    
    let state = CheckpointState {
        version: SNAPSHOT_VERSION,
        source_path: source_path.to_owned(),
        frames: frame_states,
        root,
        objects: writer.objects,
    };
    Ok(encode_checkpoint_state(&state))
}

// Keep the old function for backward compatibility
pub(crate) fn dump_checkpoint_state(
    vm: &VirtualMachine,
    source_path: &str,
    lasti: u32,
    code: &PyCode,
    globals: &PyDictRef,
) -> PyResult<Vec<u8>> {
    let mut writer = SnapshotWriter::new(vm);
    let root = writer.serialize_obj(&globals.as_object().to_owned()).map_err(|err| {
        vm.new_value_error(format!("checkpoint snapshot failed: {err:?}"))
    })?;
    
    let code_bytes = serialize_code_object(&code.code);
    // Convert to new format with single frame
    let frame_state = FrameState {
        code: code_bytes,
        lasti,
        locals: root,  // For module-level, locals == globals
        stack: Vec::new(),  // Legacy path, assume empty stack
        blocks: Vec::new(),  // Legacy path, assume empty blocks
    };
    
    let state = CheckpointState {
        version: SNAPSHOT_VERSION,
        source_path: source_path.to_owned(),
        frames: vec![frame_state],
        root,
        objects: writer.objects,
    };
    Ok(encode_checkpoint_state(&state))
}

pub(crate) fn load_checkpoint_state(
    vm: &VirtualMachine,
    data: &[u8],
) -> PyResult<(CheckpointState, Vec<PyObjectRef>)> {
    let state = decode_checkpoint_state(data)
        .map_err(|err| vm.new_value_error(format!("checkpoint decode failed: {err:?}")))?;
    if state.version != SNAPSHOT_VERSION {
        return Err(vm.new_value_error(format!(
            "unsupported checkpoint version: {}",
            state.version
        )));
    }
    let reader = SnapshotReader::new(vm, &state.objects, state.root);
    let objects = reader
        .restore_all()
        .map_err(|err| vm.new_value_error(format!("checkpoint restore failed: {err:?}")))?;
    Ok((state, objects))
}

pub(crate) fn decode_code_object(
    vm: &VirtualMachine,
    bytes: &[u8],
) -> Result<CodeObject, SnapshotError> {
    deserialize_code_object(vm, bytes)
}

fn serialize_code_object(code: &CodeObject) -> Vec<u8> {
    let mut buf = Vec::new();
    marshal::serialize_code(&mut buf, code);
    buf
}

fn deserialize_code_object(vm: &VirtualMachine, bytes: &[u8]) -> Result<CodeObject, SnapshotError> {
    let mut cursor = marshal::Cursor { data: bytes, position: 0 };
    marshal::deserialize_code(&mut cursor, PyObjBag(&vm.ctx)).map_err(|e| {
        SnapshotError::msg(format!("failed to deserialize code object: {e:?}"))
    })
}

struct SnapshotWriter<'a> {
    vm: &'a VirtualMachine,
    ids: HashMap<usize, ObjId>,
    objects: Vec<ObjectEntry>,
    held: Vec<PyObjectRef>,
    /// Cache for dynamically created Type attribute dicts: type_ptr -> dict_obj
    type_attr_dicts: HashMap<usize, PyObjectRef>,
    /// Cache for instance newargs/kwargs/state: obj_ptr -> (newargs, newkwargs, state)
    instance_data: HashMap<usize, (Option<PyObjectRef>, Option<PyObjectRef>, Option<PyObjectRef>)>,
}

impl<'a> SnapshotWriter<'a> {
    fn new(vm: &'a VirtualMachine) -> Self {
        Self {
            vm,
            ids: HashMap::new(),
            objects: Vec::new(),
            held: Vec::new(),
            type_attr_dicts: HashMap::new(),
            instance_data: HashMap::new(),
        }
    }

    /// Two-pass serialization: first assign IDs, then build payloads
    fn serialize_obj(&mut self, obj: &PyObjectRef) -> Result<ObjId, SnapshotError> {
        // Phase 1: Assign IDs to all reachable objects
        self.assign_ids_phase(obj)?;
        
        // Phase 2: Build payloads for all objects in ID order
        self.build_payloads_phase()?;
        
        // Return the root object's ID
        let ptr = obj.as_object().as_raw() as usize;
        Ok(*self.ids.get(&ptr).unwrap())
    }
    
    /// Phase 1: Recursively assign IDs to all objects in the graph
    fn assign_ids_phase(&mut self, obj: &PyObjectRef) -> Result<(), SnapshotError> {
        // Check recursion depth to prevent stack overflow
        static MAX_DEPTH: usize = 100000;
        if self.held.len() > MAX_DEPTH {
            return Err(SnapshotError::msg("recursion depth exceeded"));
        }
        
        let ptr = obj.as_object().as_raw() as usize;
        if self.ids.contains_key(&ptr) {
            return Ok(()); // Already visited
        }
        
        let id = self.held.len() as ObjId;
        self.ids.insert(ptr, id);
        self.held.push(obj.clone());
        
        // Recursively visit child objects
        self.visit_children(obj)?;
        Ok(())
    }
    
    /// Visit all child objects for ID assignment
    fn visit_children(&mut self, obj: &PyObjectRef) -> Result<(), SnapshotError> {
        let tag = classify_obj(self.vm, obj)?;
        
        match tag {
            ObjTag::None | ObjTag::Bool | ObjTag::Int | ObjTag::Float | 
            ObjTag::Str | ObjTag::Bytes | ObjTag::Code | ObjTag::BuiltinType |
            ObjTag::BuiltinModule | ObjTag::BuiltinDict => {
                // No child objects to visit
                Ok(())
            }
            ObjTag::BuiltinFunction => {
                // Visit __self__ if present
                if let Some(self_obj) = get_attr_opt(self.vm, obj, "__self__")? {
                    if !self.vm.is_none(&self_obj) {
                        self.assign_ids_phase(&self_obj)?;
                    }
                }
                Ok(())
            }
            ObjTag::List => {
                let list = obj.downcast_ref::<PyList>().ok_or_else(|| SnapshotError::msg("expected list"))?;
                for item in list.borrow_vec().iter() {
                    self.assign_ids_phase(item)?;
                }
                Ok(())
            }
            ObjTag::Tuple => {
                let tuple = obj.downcast_ref::<PyTuple>().ok_or_else(|| SnapshotError::msg("expected tuple"))?;
                for item in tuple.iter() {
                    self.assign_ids_phase(item)?;
                }
                Ok(())
            }
            ObjTag::Dict => {
                let dict = PyDictRef::try_from_object(self.vm, obj.clone())
                    .map_err(|_| SnapshotError::msg("expected dict"))?;
                for (key, value) in &dict {
                    self.assign_ids_phase(&key)?;
                    self.assign_ids_phase(&value)?;
                }
                Ok(())
            }
            ObjTag::Set => {
                let set = obj.downcast_ref::<PySet>().ok_or_else(|| SnapshotError::msg("expected set"))?;
                for key in set.elements() {
                    self.assign_ids_phase(&key)?;
                }
                Ok(())
            }
            ObjTag::FrozenSet => {
                let set = obj.downcast_ref::<PyFrozenSet>().ok_or_else(|| SnapshotError::msg("expected frozenset"))?;
                for key in set.elements() {
                    self.assign_ids_phase(&key)?;
                }
                Ok(())
            }
            ObjTag::Module => {
                let dict = obj.dict().ok_or_else(|| SnapshotError::msg("module missing dict"))?;
                self.assign_ids_phase(&dict.into())?;
                Ok(())
            }
            ObjTag::Function => {
                self.assign_ids_phase(&get_attr(self.vm, obj, "__code__")?)?;
                self.assign_ids_phase(&get_attr(self.vm, obj, "__globals__")?)?;
                
                let defaults_obj = get_attr_opt(self.vm, obj, "__defaults__")?.unwrap_or_else(|| self.vm.ctx.none());
                if !self.vm.is_none(&defaults_obj) && defaults_obj.downcast_ref::<PyTuple>().is_some() {
                    self.assign_ids_phase(&defaults_obj)?;
                }
                
                // For kwdefaults and annotations, just visit the original object
                // Conversion will be done in phase 2
                let kwdefaults_obj = get_attr_opt(self.vm, obj, "__kwdefaults__")?.unwrap_or_else(|| self.vm.ctx.none());
                if !self.vm.is_none(&kwdefaults_obj) {
                    self.assign_ids_phase(&kwdefaults_obj)?;
                }
                
                let closure_obj = get_attr(self.vm, obj, "__closure__")?;
                if !self.vm.is_none(&closure_obj) && closure_obj.downcast_ref::<PyTuple>().is_some() {
                    self.assign_ids_phase(&closure_obj)?;
                }
                
                self.assign_ids_phase(&get_attr(self.vm, obj, "__name__")?)?;
                self.assign_ids_phase(&get_attr(self.vm, obj, "__qualname__")?)?;
                self.assign_ids_phase(&get_attr(self.vm, obj, "__annotations__")?)?;
                self.assign_ids_phase(&get_attr(self.vm, obj, "__module__")?)?;
                self.assign_ids_phase(&get_attr(self.vm, obj, "__doc__")?)?;
                
                let type_params_obj = get_attr_opt(self.vm, obj, "__type_params__")?.unwrap_or_else(|| self.vm.ctx.empty_tuple.clone().into());
                self.assign_ids_phase(&type_params_obj)?;
                Ok(())
            }
            ObjTag::Type => {
                let typ = obj.downcast_ref::<PyType>().ok_or_else(|| SnapshotError::msg("expected type"))?;
                
                for base in typ.bases.read().iter() {
                    if base.as_object().as_raw() == self.vm.ctx.types.object_type.as_object().as_raw() {
                        continue;
                    }
                    self.assign_ids_phase(&base.to_owned().into())?;
                }
                
                // Create and cache attributes dict in phase 1
                let dict = self.vm.ctx.new_dict();
                for (key, value) in typ.attributes.read().iter() {
                    if should_skip_type_attr(self.vm, value) {
                        continue;
                    }
                    dict.set_item(key.as_str(), value.clone(), self.vm)
                        .map_err(|_| SnapshotError::msg("type dict build failed"))?;
                    self.assign_ids_phase(value)?;
                }
                let dict_obj: PyObjectRef = dict.into();
                let type_ptr = obj.as_object().as_raw() as usize;
                self.type_attr_dicts.insert(type_ptr, dict_obj.clone());
                self.assign_ids_phase(&dict_obj)?;
                Ok(())
            }
            ObjTag::Instance => {
                let typ = obj.class();
                self.assign_ids_phase(&typ.to_owned().into())?;
                
                let (new_args, new_kwargs) = get_newargs(self.vm, obj)?;
                let state = get_state(self.vm, obj)?;
                
                // Cache for later use in build_payload
                let obj_ptr = obj.as_object().as_raw() as usize;
                self.instance_data.insert(obj_ptr, (new_args.clone(), new_kwargs.clone(), state.clone()));
                
                if let Some(ref args) = new_args {
                    self.assign_ids_phase(args)?;
                }
                if let Some(ref kwargs) = new_kwargs {
                    self.assign_ids_phase(kwargs)?;
                }
                if let Some(ref s) = state {
                    self.assign_ids_phase(s)?;
                }
                Ok(())
            }
            ObjTag::Cell => {
                let cell = obj.downcast_ref::<PyCell>().ok_or_else(|| SnapshotError::msg("expected cell"))?;
                if let Some(contents) = cell.get() {
                    self.assign_ids_phase(&contents)?;
                }
                Ok(())
            }
            ObjTag::Enumerate => {
                // Visit the iterator via __reduce__
                // enumerate.__reduce__() returns (type, (iterator, count))
                if let Some(reduce_fn) = get_attr_opt(self.vm, obj, "__reduce__")? {
                    if let Ok(result) = self.vm.invoke(&reduce_fn, ()) {
                        if let Some(tuple) = result.downcast_ref::<PyTuple>() {
                            if tuple.len() >= 2 {
                                // Get the args tuple: (iterator, count)
                                if let Some(args) = tuple.get(1).and_then(|o| o.downcast_ref::<PyTuple>()) {
                                    if let Some(iterator) = args.get(0) {
                                        self.assign_ids_phase(iterator)?;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            ObjTag::Zip => {
                // Visit all iterators in the zip
                if let Some(iterators) = get_attr_opt(self.vm, obj, "__iterators__")? {
                    if let Some(tuple) = iterators.downcast_ref::<PyTuple>() {
                        for iter in tuple.iter() {
                            self.assign_ids_phase(iter)?;
                        }
                    }
                }
                Ok(())
            }
            ObjTag::Map => {
                // Visit the function and iterator
                if let Some(func) = get_attr_opt(self.vm, obj, "__func__")? {
                    self.assign_ids_phase(&func)?;
                }
                if let Some(iterator) = get_attr_opt(self.vm, obj, "__iterator__")? {
                    self.assign_ids_phase(&iterator)?;
                }
                Ok(())
            }
            ObjTag::Filter => {
                // Visit the predicate and iterator
                if let Some(func) = get_attr_opt(self.vm, obj, "__predicate__")? {
                    self.assign_ids_phase(&func)?;
                }
                if let Some(iterator) = get_attr_opt(self.vm, obj, "__iterator__")? {
                    self.assign_ids_phase(&iterator)?;
                }
                Ok(())
            }
            ObjTag::ListIterator => {
                // Visit the list via __reduce__
                // list_iterator.__reduce__() returns (iter, (list,), position)
                if let Some(reduce_fn) = get_attr_opt(self.vm, obj, "__reduce__")? {
                    if let Ok(result) = self.vm.invoke(&reduce_fn, ()) {
                        if let Some(tuple) = result.downcast_ref::<PyTuple>() {
                            if tuple.len() >= 2 {
                                // Get the args tuple: (list,)
                                if let Some(args) = tuple.get(1).and_then(|o| o.downcast_ref::<PyTuple>()) {
                                    if let Some(list) = args.get(0) {
                                        self.assign_ids_phase(list)?;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            ObjTag::Range => {
                // range object has start, stop, step which are integers
                // No need to assign IDs for these primitive values
                Ok(())
            }
            ObjTag::RangeIterator => {
                // Visit the range via __reduce__
                // range_iterator.__reduce__() returns (iter, (range,), position)
                if let Some(reduce_fn) = get_attr_opt(self.vm, obj, "__reduce__")? {
                    if let Ok(result) = self.vm.invoke(&reduce_fn, ()) {
                        if let Some(tuple) = result.downcast_ref::<PyTuple>() {
                            if tuple.len() >= 2 {
                                // Get the args tuple: (range,)
                                if let Some(args) = tuple.get(1).and_then(|o| o.downcast_ref::<PyTuple>()) {
                                    if let Some(range) = args.get(0) {
                                        self.assign_ids_phase(range)?;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }
    
    /// Phase 2: Build payloads for all objects in ID order
    fn build_payloads_phase(&mut self) -> Result<(), SnapshotError> {
        let count = self.held.len();
        self.objects.reserve(count);
        
        for idx in 0..count {
            let obj = self.held[idx].clone(); // Clone to avoid borrow checker issues
            let tag = classify_obj(self.vm, &obj)?;
            let payload = self.build_payload(tag, &obj)?;
            self.objects.push(ObjectEntry { tag, payload });
        }
        
        Ok(())
    }
    
    /// Get the ID of an already-visited object
    fn get_id(&self, obj: &PyObjectRef) -> Result<ObjId, SnapshotError> {
        let ptr = obj.as_object().as_raw() as usize;
        self.ids.get(&ptr).copied()
            .ok_or_else(|| SnapshotError::msg(format!("object not in ID map: class={}", obj.class().name())))
    }
    
    /// Get ID or assign new ID if object not yet visited (for dynamically created objects)
    fn get_or_assign_id(&mut self, obj: &PyObjectRef) -> Result<ObjId, SnapshotError> {
        let ptr = obj.as_object().as_raw() as usize;
        if let Some(&id) = self.ids.get(&ptr) {
            return Ok(id);
        }
        
        // Object not yet visited, assign ID now
        let id = self.held.len() as ObjId;
        self.ids.insert(ptr, id);
        self.held.push(obj.clone());
        
        // Build payload immediately
        let tag = classify_obj(self.vm, obj)?;
        let payload = self.build_payload(tag, obj)?;
        self.objects.push(ObjectEntry { tag, payload });
        
        Ok(id)
    }
    
    /// Get or create a converted dict object (for kwdefaults/annotations)
    /// Returns the same object on subsequent calls with the same source_ptr
    fn build_payload(&mut self, tag: ObjTag, obj: &PyObjectRef) -> Result<ObjectPayload, SnapshotError> {
        match tag {
            ObjTag::None => Ok(ObjectPayload::None),
            ObjTag::Bool => Ok(ObjectPayload::Bool(obj.clone().is_true(self.vm).unwrap_or(false))),
            ObjTag::Int => {
                let value = obj
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| SnapshotError::msg("expected int"))?;
                Ok(ObjectPayload::Int(value.as_bigint().to_string()))
            }
            ObjTag::Float => {
                let value = obj
                    .downcast_ref::<PyFloat>()
                    .ok_or_else(|| SnapshotError::msg("expected float"))?;
                Ok(ObjectPayload::Float(value.to_f64()))
            }
            ObjTag::Str => {
                let value = obj
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| SnapshotError::msg("expected str"))?;
                Ok(ObjectPayload::Str(value.as_str().to_owned()))
            }
            ObjTag::Bytes => {
                let value = obj
                    .downcast_ref::<crate::builtins::PyBytes>()
                    .ok_or_else(|| SnapshotError::msg("expected bytes"))?;
                Ok(ObjectPayload::Bytes(value.as_bytes().to_vec()))
            }
            ObjTag::List => {
                let list = obj.downcast_ref::<PyList>().ok_or_else(|| SnapshotError::msg("expected list"))?;
                let items = list
                    .borrow_vec()
                    .iter()
                    .map(|item| self.get_id(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ObjectPayload::List(items))
            }
            ObjTag::Tuple => {
                let tuple = obj.downcast_ref::<PyTuple>().ok_or_else(|| SnapshotError::msg("expected tuple"))?;
                let items = tuple
                    .iter()
                    .map(|item| self.get_id(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ObjectPayload::Tuple(items))
            }
            ObjTag::Dict => {
                let dict: PyDictRef = obj
                    .clone()
                    .downcast()
                    .map_err(|_| SnapshotError::msg("expected dict"))?;
                let mut entries = Vec::new();
                for (key, value) in &dict {
                    let key_bytes = snapshot_key_bytes(self.vm, &key)?;
                    let key_id = self.get_id(&key)?;
                    let value_id = self.get_id(&value)?;
                    entries.push((key_bytes, key_id, value_id));
                }
                entries.sort_by(|(a, _, _), (b, _, _)| cbor_key_cmp(a, b));
                let pairs = entries
                    .into_iter()
                    .map(|(_, k, v)| (k, v))
                    .collect();
                Ok(ObjectPayload::Dict(pairs))
            }
            ObjTag::Set => {
                let set = obj.downcast_ref::<PySet>().ok_or_else(|| SnapshotError::msg("expected set"))?;
                let mut entries = Vec::new();
                for key in set.elements() {
                    let key_bytes = snapshot_key_bytes(self.vm, &key)?;
                    let key_id = self.get_id(&key)?;
                    entries.push((key_bytes, key_id));
                }
                entries.sort_by(|(a, _), (b, _)| cbor_key_cmp(a, b));
                let ids = entries.into_iter().map(|(_, id)| id).collect();
                Ok(ObjectPayload::Set(ids))
            }
            ObjTag::FrozenSet => {
                let set = obj.downcast_ref::<PyFrozenSet>().ok_or_else(|| SnapshotError::msg("expected frozenset"))?;
                let mut entries = Vec::new();
                for key in set.elements() {
                    let key_bytes = snapshot_key_bytes(self.vm, &key)?;
                    let key_id = self.get_id(&key)?;
                    entries.push((key_bytes, key_id));
                }
                entries.sort_by(|(a, _), (b, _)| cbor_key_cmp(a, b));
                let ids = entries.into_iter().map(|(_, id)| id).collect();
                Ok(ObjectPayload::FrozenSet(ids))
            }
            ObjTag::Module => {
                obj.downcast_ref::<PyModule>()
                    .ok_or_else(|| SnapshotError::msg("expected module"))?;
                let dict = obj
                    .dict()
                    .ok_or_else(|| SnapshotError::msg("module missing dict"))?;
                let name = get_attr_str(self.vm, obj, "__name__")?.unwrap_or_default();
                let dict_id = self.get_id(&dict.into())?;
                Ok(ObjectPayload::Module { name, dict: dict_id })
            }
            ObjTag::Function => {
                obj.downcast_ref::<PyFunction>()
                    .ok_or_else(|| SnapshotError::msg("expected function"))?;
                let code_obj = get_attr(self.vm, obj, "__code__")?;
                let code = self.get_id(&code_obj)?;
                let globals_obj = get_attr(self.vm, obj, "__globals__")?;
                let globals = self.get_id(&globals_obj)?;
                let defaults_obj = get_attr(self.vm, obj, "__defaults__")?;
                let defaults = if self.vm.is_none(&defaults_obj) {
                    None
                } else if defaults_obj.downcast_ref::<PyTuple>().is_some() {
                    Some(self.get_id(&defaults_obj)?)
                } else {
                    None
                };
                let kwdefaults_obj = get_attr(self.vm, obj, "__kwdefaults__")?;
                let kwdefaults = if self.vm.is_none(&kwdefaults_obj) {
                    None
                } else if PyDictRef::try_from_object(self.vm, kwdefaults_obj.clone()).is_ok() {
                    Some(self.get_id(&kwdefaults_obj)?)
                } else if let Ok(dict) = mapping_to_dict(self.vm, &kwdefaults_obj) {
                    // Create new dict, assign ID dynamically
                    let dict_obj: PyObjectRef = dict.into();
                    let id = self.held.len() as ObjId;
                    let ptr = dict_obj.as_object().as_raw() as usize;
                    self.ids.insert(ptr, id);
                    self.held.push(dict_obj.clone());
                    self.objects.push(ObjectEntry {
                        tag: ObjTag::Dict,
                        payload: ObjectPayload::Dict(Vec::new()), // Will be filled later if needed
                    });
                    Some(id)
                } else {
                    None
                };
                let closure_obj = get_attr(self.vm, obj, "__closure__")?;
                let closure = if self.vm.is_none(&closure_obj) {
                    None
                } else if closure_obj.downcast_ref::<PyTuple>().is_some() {
                    Some(self.get_id(&closure_obj)?)
                } else {
                    None
                };
                let name = self.get_id(&get_attr(self.vm, obj, "__name__")?)?;
                let qualname = self.get_id(&get_attr(self.vm, obj, "__qualname__")?)?;
                let annotations_obj = get_attr(self.vm, obj, "__annotations__")?;
                let annotations = if PyDictRef::try_from_object(self.vm, annotations_obj.clone()).is_ok() {
                    self.get_id(&annotations_obj)?
                } else if let Ok(dict) = mapping_to_dict(self.vm, &annotations_obj) {
                    // Create new dict, assign ID dynamically
                    let dict_obj: PyObjectRef = dict.into();
                    let id = self.held.len() as ObjId;
                    let ptr = dict_obj.as_object().as_raw() as usize;
                    self.ids.insert(ptr, id);
                    self.held.push(dict_obj.clone());
                    self.objects.push(ObjectEntry {
                        tag: ObjTag::Dict,
                        payload: ObjectPayload::Dict(Vec::new()),
                    });
                    id
                } else {
                    // Create empty dict
                    let dict_obj: PyObjectRef = self.vm.ctx.new_dict().into();
                    let id = self.held.len() as ObjId;
                    let ptr = dict_obj.as_object().as_raw() as usize;
                    self.ids.insert(ptr, id);
                    self.held.push(dict_obj);
                    self.objects.push(ObjectEntry {
                        tag: ObjTag::Dict,
                        payload: ObjectPayload::Dict(Vec::new()),
                    });
                    id
                };
                let module = self.get_id(&get_attr(self.vm, obj, "__module__")?)?;
                let doc = self.get_id(&get_attr(self.vm, obj, "__doc__")?)?;
                let type_params_obj = get_attr_opt(self.vm, obj, "__type_params__")?
                    .unwrap_or_else(|| self.vm.ctx.empty_tuple.clone().into());
                let type_params = self.get_id(&type_params_obj)?;
                Ok(ObjectPayload::Function(FunctionPayload {
                    code,
                    globals,
                    defaults,
                    kwdefaults,
                    closure,
                    name,
                    qualname,
                    annotations,
                    module,
                    doc,
                    type_params,
                }))
            }
            ObjTag::Code => {
                let code = obj.downcast_ref::<PyCode>().ok_or_else(|| SnapshotError::msg("expected code"))?;
                Ok(ObjectPayload::Code(serialize_code_object(&code.code)))
            }
            ObjTag::Type => {
                let typ = obj.downcast_ref::<PyType>().ok_or_else(|| SnapshotError::msg("expected type"))?;
                let bases: Vec<ObjId> = typ
                    .bases
                    .read()
                    .iter()
                    .filter_map(|base| {
                        // Skip object type (should not be serialized)
                        if base.as_object().as_raw() == self.vm.ctx.types.object_type.as_object().as_raw() {
                            None
                        } else {
                            Some(self.get_id(&base.to_owned().into()))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                
                // Retrieve cached attributes dict
                let type_ptr = obj.as_object().as_raw() as usize;
                let dict_obj = self.type_attr_dicts.get(&type_ptr)
                    .ok_or_else(|| SnapshotError::msg("type attributes dict not found in cache"))?;
                let dict_id = self.get_id(dict_obj)?;
                
                let qualname_obj = typ.__qualname__(self.vm);
                let qualname = qualname_obj
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| SnapshotError::msg("type __qualname__ must be str"))?
                    .as_str()
                    .to_owned();
                Ok(ObjectPayload::Type(TypePayload {
                    name: typ.name().to_owned(),
                    qualname,
                    bases,
                    dict: dict_id,
                    flags: typ.slots.flags.bits(),
                    basicsize: typ.slots.basicsize,
                    itemsize: typ.slots.itemsize,
                    member_count: typ.slots.member_count,
                }))
            }
            ObjTag::BuiltinType => {
                let typ = obj.downcast_ref::<PyType>().ok_or_else(|| SnapshotError::msg("expected type"))?;
                let module = get_attr_str(self.vm, obj, "__module__")?
                    .unwrap_or_else(|| "builtins".to_owned());
                Ok(ObjectPayload::BuiltinType {
                    module,
                    name: typ.name().to_owned(),
                })
            }
            ObjTag::Instance => {
                let typ = obj.class();
                let typ_id = self.get_id(&typ.to_owned().into())?;
                
                // Retrieve cached instance data
                let obj_ptr = obj.as_object().as_raw() as usize;
                let (new_args, new_kwargs, state) = self.instance_data.get(&obj_ptr)
                    .ok_or_else(|| SnapshotError::msg("instance data not found in cache"))?;
                
                let new_args_id = new_args.as_ref().map(|o| self.get_id(o)).transpose()?;
                let new_kwargs_id = new_kwargs.as_ref().map(|o| self.get_id(o)).transpose()?;
                let state_id = state.as_ref().map(|o| self.get_id(o)).transpose()?;
                Ok(ObjectPayload::Instance(InstancePayload {
                    typ: typ_id,
                    state: state_id,
                    new_args: new_args_id,
                    new_kwargs: new_kwargs_id,
                }))
            }
            ObjTag::Cell => {
                let cell = obj.downcast_ref::<PyCell>().ok_or_else(|| SnapshotError::msg("expected cell"))?;
                let contents = cell.get().map(|o| self.get_id(&o)).transpose()?;
                Ok(ObjectPayload::Cell(contents))
            }
            ObjTag::BuiltinModule => {
                let name = get_attr_str(self.vm, obj, "__name__")?.unwrap_or_default();
                Ok(ObjectPayload::BuiltinModule { name })
            }
            ObjTag::BuiltinDict => {
                Ok(ObjectPayload::BuiltinDict { name: "builtins".to_owned() })
            }
            ObjTag::BuiltinFunction => {
                let name = get_attr_str(self.vm, obj, "__name__")?
                    .ok_or_else(|| SnapshotError::msg("builtin function missing __name__"))?;
                let module = get_attr_str(self.vm, obj, "__module__")?;
                let self_obj = get_attr_opt(self.vm, obj, "__self__")?
                    .and_then(|value| if self.vm.is_none(&value) { None } else { Some(value) })
                    .map(|value| self.get_id(&value))
                    .transpose()?;
                Ok(ObjectPayload::BuiltinFunction(BuiltinFunctionPayload {
                    name,
                    module,
                    self_obj,
                }))
            }
            ObjTag::Enumerate => {
                // Use __reduce__ to get iterator and count
                let reduce_fn = get_attr(self.vm, obj, "__reduce__")?;
                let result = self.vm.invoke(&reduce_fn, ())
                    .map_err(|_| SnapshotError::msg("enumerate __reduce__ failed"))?;
                
                let tuple = result.downcast_ref::<PyTuple>()
                    .ok_or_else(|| SnapshotError::msg("enumerate __reduce__ didn't return tuple"))?;
                
                if tuple.len() < 2 {
                    return Err(SnapshotError::msg("enumerate __reduce__ tuple too short"));
                }
                
                // Get args tuple: (iterator, count)
                let args = tuple.get(1)
                    .and_then(|o| o.downcast_ref::<PyTuple>())
                    .ok_or_else(|| SnapshotError::msg("enumerate __reduce__ args invalid"))?;
                
                let iterator = args.get(0)
                    .ok_or_else(|| SnapshotError::msg("enumerate missing iterator in __reduce__"))?
                    .clone();
                let iterator_id = self.get_id(&iterator)?;
                
                let count_bigint = args.get(1)
                    .and_then(|o| o.downcast_ref::<crate::builtins::int::PyInt>())
                    .ok_or_else(|| SnapshotError::msg("enumerate missing count in __reduce__"))?;
                
                let count = count_bigint.try_to_primitive::<i64>(self.vm).unwrap_or(0);
                
                Ok(ObjectPayload::Enumerate { iterator: iterator_id, count })
            }
            ObjTag::Zip => {
                // Extract iterators from zip object
                let iterators_obj = get_attr_opt(self.vm, obj, "__iterators__")?
                    .ok_or_else(|| SnapshotError::msg("zip missing __iterators__"))?;
                
                let iterators = if let Some(tuple) = iterators_obj.downcast_ref::<PyTuple>() {
                    tuple.iter()
                        .map(|iter| self.get_id(iter))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };
                
                Ok(ObjectPayload::Zip { iterators })
            }
            ObjTag::Map => {
                // Extract function and iterator from map object
                let function = get_attr_opt(self.vm, obj, "__func__")?
                    .ok_or_else(|| SnapshotError::msg("map missing __func__"))?;
                let function_id = self.get_id(&function)?;
                
                let iterator = get_attr_opt(self.vm, obj, "__iterator__")?
                    .ok_or_else(|| SnapshotError::msg("map missing __iterator__"))?;
                let iterator_id = self.get_id(&iterator)?;
                
                Ok(ObjectPayload::Map { function: function_id, iterator: iterator_id })
            }
            ObjTag::Filter => {
                // Extract predicate and iterator from filter object
                let function = get_attr_opt(self.vm, obj, "__predicate__")?
                    .ok_or_else(|| SnapshotError::msg("filter missing __predicate__"))?;
                let function_id = self.get_id(&function)?;
                
                let iterator = get_attr_opt(self.vm, obj, "__iterator__")?
                    .ok_or_else(|| SnapshotError::msg("filter missing __iterator__"))?;
                let iterator_id = self.get_id(&iterator)?;
                
                Ok(ObjectPayload::Filter { function: function_id, iterator: iterator_id })
            }
            ObjTag::ListIterator => {
                // Use __reduce__ to get list and position
                // list_iterator.__reduce__() returns (iter, (list,), position)
                let reduce_fn = get_attr(self.vm, obj, "__reduce__")?;
                let result = self.vm.invoke(&reduce_fn, ())
                    .map_err(|_| SnapshotError::msg("list_iterator __reduce__ failed"))?;
                
                let tuple = result.downcast_ref::<PyTuple>()
                    .ok_or_else(|| SnapshotError::msg("list_iterator __reduce__ didn't return tuple"))?;
                
                if tuple.len() < 3 {
                    return Err(SnapshotError::msg("list_iterator __reduce__ tuple too short"));
                }
                
                // Get args tuple: (list,)
                let args = tuple.get(1)
                    .and_then(|o| o.downcast_ref::<PyTuple>())
                    .ok_or_else(|| SnapshotError::msg("list_iterator __reduce__ args invalid"))?;
                
                let list = args.get(0)
                    .ok_or_else(|| SnapshotError::msg("list_iterator missing list in __reduce__"))?
                    .clone();
                
                let list_id = self.get_id(&list)?;
                
                // Get position (third element of reduce result)
                let position = tuple.get(2)
                    .and_then(|o| o.downcast_ref::<crate::builtins::int::PyInt>())
                    .ok_or_else(|| SnapshotError::msg("list_iterator missing position in __reduce__"))?
                    .try_to_primitive::<usize>(self.vm)
                    .unwrap_or(0);
                
                Ok(ObjectPayload::ListIterator { list: list_id, position })
            }
            ObjTag::Range => {
                // Serialize range object by extracting start, stop, step
                let start = get_attr(self.vm, obj, "start")?
                    .downcast_ref::<crate::builtins::int::PyInt>()
                    .ok_or_else(|| SnapshotError::msg("range.start is not int"))?
                    .try_to_primitive::<i64>(self.vm)
                    .unwrap_or(0);
                
                let stop = get_attr(self.vm, obj, "stop")?
                    .downcast_ref::<crate::builtins::int::PyInt>()
                    .ok_or_else(|| SnapshotError::msg("range.stop is not int"))?
                    .try_to_primitive::<i64>(self.vm)
                    .unwrap_or(0);
                
                let step = get_attr(self.vm, obj, "step")?
                    .downcast_ref::<crate::builtins::int::PyInt>()
                    .ok_or_else(|| SnapshotError::msg("range.step is not int"))?
                    .try_to_primitive::<i64>(self.vm)
                    .unwrap_or(1);
                
                Ok(ObjectPayload::Range { start, stop, step })
            }
            ObjTag::RangeIterator => {
                // Use __reduce__ to get range and position
                // range_iterator.__reduce__() returns (iter, (range,), position)
                let reduce_fn = get_attr(self.vm, obj, "__reduce__")?;
                let result = self.vm.invoke(&reduce_fn, ())
                    .map_err(|_| SnapshotError::msg("range_iterator __reduce__ failed"))?;
                
                let tuple = result.downcast_ref::<PyTuple>()
                    .ok_or_else(|| SnapshotError::msg("range_iterator __reduce__ didn't return tuple"))?;
                
                if tuple.len() < 3 {
                    return Err(SnapshotError::msg("range_iterator __reduce__ tuple too short"));
                }
                
                // Get args tuple: (range,)
                let args = tuple.get(1)
                    .and_then(|o| o.downcast_ref::<PyTuple>())
                    .ok_or_else(|| SnapshotError::msg("range_iterator __reduce__ args invalid"))?;
                
                let range = args.get(0)
                    .ok_or_else(|| SnapshotError::msg("range_iterator missing range in __reduce__"))?
                    .clone();
                
                // Use get_or_assign_id to handle range objects that may not have been visited yet
                let range_id = self.get_or_assign_id(&range)?;
                
                // Get position (third element of reduce result)
                let position = tuple.get(2)
                    .and_then(|o| o.downcast_ref::<crate::builtins::int::PyInt>())
                    .ok_or_else(|| SnapshotError::msg("range_iterator missing position in __reduce__"))?
                    .try_to_primitive::<usize>(self.vm)
                    .unwrap_or(0);
                
                Ok(ObjectPayload::RangeIterator { range: range_id, position })
            }
        }
    }
}

fn classify_obj(vm: &VirtualMachine, obj: &PyObjectRef) -> Result<ObjTag, SnapshotError> {
    if vm.is_none(obj) {
        return Ok(ObjTag::None);
    }
    if obj.fast_isinstance(vm.ctx.types.bool_type) {
        return Ok(ObjTag::Bool);
    }
    if obj.fast_isinstance(vm.ctx.types.int_type) {
        return Ok(ObjTag::Int);
    }
    if obj.fast_isinstance(vm.ctx.types.float_type) {
        return Ok(ObjTag::Float);
    }
    if obj.fast_isinstance(vm.ctx.types.str_type) {
        return Ok(ObjTag::Str);
    }
    if obj.fast_isinstance(vm.ctx.types.bytes_type) {
        return Ok(ObjTag::Bytes);
    }
    if obj.downcast_ref::<PyList>().is_some() {
        return Ok(ObjTag::List);
    }
    if obj.downcast_ref::<PyTuple>().is_some() {
        return Ok(ObjTag::Tuple);
    }
    if obj.downcast_ref::<PyDict>().is_some() {
        if is_builtin_dict(vm, obj) {
            return Ok(ObjTag::BuiltinDict);
        }
        return Ok(ObjTag::Dict);
    }
    if obj.downcast_ref::<PySet>().is_some() {
        return Ok(ObjTag::Set);
    }
    if obj.downcast_ref::<PyFrozenSet>().is_some() {
        return Ok(ObjTag::FrozenSet);
    }
    if obj.downcast_ref::<PyModule>().is_some() {
        if is_builtin_module(vm, obj) {
            return Ok(ObjTag::BuiltinModule);
        }
        return Ok(ObjTag::Module);
    }
    if obj.downcast_ref::<PyFunction>().is_some() {
        return Ok(ObjTag::Function);
    }
    if obj.fast_isinstance(vm.ctx.types.builtin_function_or_method_type) {
        return Ok(ObjTag::BuiltinFunction);
    }
    
    // Check for iterator types by class name
    let class_name_obj = obj.class().name();
    let class_name = class_name_obj.as_ref();
    match class_name {
        "enumerate" => return Ok(ObjTag::Enumerate),
        "zip" => return Ok(ObjTag::Zip),
        "map" => return Ok(ObjTag::Map),
        "filter" => return Ok(ObjTag::Filter),
        "list_iterator" => return Ok(ObjTag::ListIterator),
        "range_iterator" => return Ok(ObjTag::RangeIterator),
        "range" => return Ok(ObjTag::Range),
        _ => {}
    }
    if obj.downcast_ref::<PyCode>().is_some() {
        return Ok(ObjTag::Code);
    }
    if let Some(typ) = obj.downcast_ref::<PyType>() {
        if typ.slots.flags.has_feature(crate::types::PyTypeFlags::HEAPTYPE) {
            return Ok(ObjTag::Type);
        }
        return Ok(ObjTag::BuiltinType);
    }
    if obj.downcast_ref::<PyCell>().is_some() {
        return Ok(ObjTag::Cell);
    }
    Ok(ObjTag::Instance)
}

fn get_attr(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    name: &'static str,
) -> Result<PyObjectRef, SnapshotError> {
    get_attr_opt(vm, obj, name)?
        .ok_or_else(|| SnapshotError::msg(format!("attribute '{name}' missing")))
}

fn get_attr_opt(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    name: &'static str,
) -> Result<Option<PyObjectRef>, SnapshotError> {
    vm.get_attribute_opt(obj.clone(), name)
        .map_err(|_| SnapshotError::msg(format!("attribute '{name}' lookup failed")))
}

fn get_attr_str(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
    name: &'static str,
) -> Result<Option<String>, SnapshotError> {
    let Some(value) = get_attr_opt(vm, obj, name)? else {
        return Ok(None);
    };
    if vm.is_none(&value) {
        return Ok(None);
    }
    let value = value
        .downcast_ref::<PyStr>()
        .ok_or_else(|| SnapshotError::msg(format!("attribute '{name}' must be str")))?;
    Ok(Some(value.as_str().to_owned()))
}

fn is_builtin_module(vm: &VirtualMachine, obj: &PyObjectRef) -> bool {
    let raw = obj.as_object().as_raw();
    if raw == vm.builtins.as_object().as_raw() || raw == vm.sys_module.as_object().as_raw() {
        return true;
    }
    matches!(
        get_attr_str(vm, obj, "__name__").ok().flatten().as_deref(),
        Some("builtins") | Some("sys")
    )
}

fn is_builtin_dict(vm: &VirtualMachine, obj: &PyObjectRef) -> bool {
    let raw = obj.as_object().as_raw();
    raw == vm.builtins.dict().as_object().as_raw()
}

fn should_skip_type_attr(vm: &VirtualMachine, value: &PyObjectRef) -> bool {
    value.fast_isinstance(vm.ctx.types.getset_type)
        || value.fast_isinstance(vm.ctx.types.member_descriptor_type)
        || value.fast_isinstance(vm.ctx.types.method_descriptor_type)
        || value.fast_isinstance(vm.ctx.types.wrapper_descriptor_type)
}

fn get_state(vm: &VirtualMachine, obj: &PyObjectRef) -> Result<Option<PyObjectRef>, SnapshotError> {
    let class_name = obj.class().name();
    
    // Skip __getstate__ for problematic types that cause infinite recursion
    let skip_getstate = &*class_name == "_Feature";
    
    if !skip_getstate {
        if let Some(getstate) = vm.get_attribute_opt(obj.clone(), "__getstate__").map_err(|_| SnapshotError::msg("getstate lookup failed"))? {
            let value = getstate
                .call((), vm)
                .map_err(|_| SnapshotError::msg("__getstate__ failed"))?;
            return Ok(Some(value));
        }
    }
    
    if let Some(dict) = obj.dict() {
        return Ok(Some(dict.into()));
    }
    Ok(None)
}

fn get_newargs(
    vm: &VirtualMachine,
    obj: &PyObjectRef,
) -> Result<(Option<PyObjectRef>, Option<PyObjectRef>), SnapshotError> {
    if obj.fast_isinstance(vm.ctx.types.classmethod_type)
        || obj.fast_isinstance(vm.ctx.types.staticmethod_type)
    {
        let func = get_attr(vm, obj, "__func__")?;
        let args = vm.new_tuple(vec![func]).into();
        return Ok((Some(args), None));
    }
    if let Some(getnewargs_ex) = vm
        .get_attribute_opt(obj.clone(), "__getnewargs_ex__")
        .map_err(|_| SnapshotError::msg("getnewargs_ex lookup failed"))?
    {
        let value = getnewargs_ex
            .call((), vm)
            .map_err(|_| SnapshotError::msg("__getnewargs_ex__ failed"))?;
        let tuple = if let Some(tuple) = value.downcast_ref::<PyTuple>() {
            tuple
        } else if let Some(list) = value.downcast_ref::<PyList>() {
            return Ok((Some(vm.new_tuple(list.borrow_vec().to_vec()).into()), None));
        } else {
            return Ok((None, None));
        };
        let args = tuple
            .get(0)
            .ok_or_else(|| SnapshotError::msg("__getnewargs_ex__ missing args"))?
            .clone();
        let kwargs = tuple
            .get(1)
            .ok_or_else(|| SnapshotError::msg("__getnewargs_ex__ missing kwargs"))?
            .clone();
        return Ok((Some(args), Some(kwargs)));
    }
    if let Some(getnewargs) = vm
        .get_attribute_opt(obj.clone(), "__getnewargs__")
        .map_err(|_| SnapshotError::msg("getnewargs lookup failed"))?
    {
        let value = getnewargs
            .call((), vm)
            .map_err(|_| SnapshotError::msg("__getnewargs__ failed"))?;
        if value.downcast_ref::<PyTuple>().is_some() {
            return Ok((Some(value), None));
        }
        if let Some(list) = value.downcast_ref::<PyList>() {
            return Ok((Some(vm.new_tuple(list.borrow_vec().to_vec()).into()), None));
        }
        return Ok((None, None));
    }
    Ok((None, None))
}

fn snapshot_key_bytes(vm: &VirtualMachine, obj: &PyObjectRef) -> Result<Vec<u8>, SnapshotError> {
    let mut encoder = CborWriter::new();
    encode_key(vm, obj, &mut encoder)?;
    Ok(encoder.into_bytes())
}

fn encode_key(vm: &VirtualMachine, obj: &PyObjectRef, encoder: &mut CborWriter) -> Result<(), SnapshotError> {
    const TAG_NONE: u64 = 0;
    const TAG_BOOL: u64 = 1;
    const TAG_INT: u64 = 2;
    const TAG_FLOAT: u64 = 3;
    const TAG_STR: u64 = 4;
    const TAG_BYTES: u64 = 5;
    const TAG_TUPLE: u64 = 6;
    const TAG_TYPE: u64 = 7;
    const TAG_MODULE: u64 = 8;
    const TAG_FUNCTION: u64 = 9;
    const TAG_BUILTIN_FUNCTION: u64 = 10;
    const TAG_CODE: u64 = 11;
    const TAG_FROZENSET: u64 = 12;
    const TAG_WEAKREF: u64 = 13;

    if vm.is_none(obj) {
        write_tagged_key(encoder, TAG_NONE, |enc| enc.write_null());
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.bool_type) {
        let value = obj.clone().is_true(vm).unwrap_or(false);
        write_tagged_key(encoder, TAG_BOOL, |enc| enc.write_bool(value));
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.int_type) {
        let value = obj
            .downcast_ref::<PyInt>()
            .ok_or_else(|| SnapshotError::msg("expected int key"))?;
        let text = value.as_bigint().to_string();
        write_tagged_key(encoder, TAG_INT, |enc| enc.write_text(&text));
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.float_type) {
        let value = obj
            .downcast_ref::<PyFloat>()
            .ok_or_else(|| SnapshotError::msg("expected float key"))?;
        let num = value.to_f64();
        write_tagged_key(encoder, TAG_FLOAT, |enc| enc.write_f64(num));
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.str_type) {
        let value = obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("expected str key"))?;
        let text = value.as_str();
        write_tagged_key(encoder, TAG_STR, |enc| enc.write_text(text));
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.bytes_type) {
        let value = obj
            .downcast_ref::<crate::builtins::PyBytes>()
            .ok_or_else(|| SnapshotError::msg("expected bytes key"))?;
        let bytes = value.as_bytes();
        write_tagged_key(encoder, TAG_BYTES, |enc| enc.write_bytes(bytes));
        return Ok(());
    }
    if let Some(tuple) = obj.downcast_ref::<PyTuple>() {
        encoder.write_array_len(2);
        encoder.write_uint(TAG_TUPLE);
        encoder.write_array_len(tuple.len());
        for item in tuple.iter() {
            encode_key(vm, item, encoder)?;
        }
        return Ok(());
    }
    if let Some(frozen) = obj.downcast_ref::<PyFrozenSet>() {
        let mut entries = Vec::new();
        for item in frozen.elements() {
            let mut item_writer = CborWriter::new();
            encode_key(vm, &item, &mut item_writer)?;
            entries.push(item_writer.into_bytes());
        }
        entries.sort_by(|a, b| cbor_key_cmp(a, b));
        encoder.write_array_len(2);
        encoder.write_uint(TAG_FROZENSET);
        encoder.write_array_len(entries.len());
        for item in entries {
            encoder.buf.extend_from_slice(&item);
        }
        return Ok(());
    }
    if let Some(weak) = obj.downcast_ref::<PyWeak>() {
        let Some(target) = weak.upgrade() else {
            return Err(SnapshotError::msg("unsupported dict/set key type: weakref (dead)"));
        };
        let mut target_writer = CborWriter::new();
        encode_key(vm, &target, &mut target_writer)?;
        encoder.write_array_len(2);
        encoder.write_uint(TAG_WEAKREF);
        encoder.buf.extend_from_slice(&target_writer.into_bytes());
        return Ok(());
    }
    if let Some(typ) = obj.downcast_ref::<PyType>() {
        let module = get_attr_str(vm, obj, "__module__")?
            .unwrap_or_else(|| "builtins".to_owned());
        let qualname = get_attr_str(vm, obj, "__qualname__")?
            .unwrap_or_else(|| typ.name().to_owned());
        encoder.write_array_len(2);
        encoder.write_uint(TAG_TYPE);
        encoder.write_array_len(2);
        encoder.write_text(&module);
        encoder.write_text(&qualname);
        return Ok(());
    }
    if obj.downcast_ref::<PyModule>().is_some() {
        let name = get_attr_str(vm, obj, "__name__")?.unwrap_or_default();
        write_tagged_key(encoder, TAG_MODULE, |enc| enc.write_text(&name));
        return Ok(());
    }
    if obj.downcast_ref::<PyFunction>().is_some() {
        let module = get_attr_str(vm, obj, "__module__")?.unwrap_or_default();
        let qualname = get_attr_str(vm, obj, "__qualname__")?
            .or_else(|| get_attr_str(vm, obj, "__name__").ok().flatten())
            .unwrap_or_default();
        encoder.write_array_len(2);
        encoder.write_uint(TAG_FUNCTION);
        encoder.write_array_len(2);
        encoder.write_text(&module);
        encoder.write_text(&qualname);
        return Ok(());
    }
    if obj.fast_isinstance(vm.ctx.types.builtin_function_or_method_type) {
        let module = get_attr_str(vm, obj, "__module__")?
            .unwrap_or_else(|| "builtins".to_owned());
        let qualname = get_attr_str(vm, obj, "__qualname__")?
            .or_else(|| get_attr_str(vm, obj, "__name__").ok().flatten())
            .unwrap_or_default();
        let self_obj = get_attr_opt(vm, obj, "__self__")?
            .and_then(|value| if vm.is_none(&value) { None } else { Some(value) });
        encoder.write_array_len(2);
        encoder.write_uint(TAG_BUILTIN_FUNCTION);
        if let Some(self_obj) = self_obj {
            encoder.write_array_len(3);
            encoder.write_text(&module);
            encoder.write_text(&qualname);
            let mut self_writer = CborWriter::new();
            encode_key(vm, &self_obj, &mut self_writer)?;
            encoder.buf.extend_from_slice(&self_writer.into_bytes());
        } else {
            encoder.write_array_len(2);
            encoder.write_text(&module);
            encoder.write_text(&qualname);
        }
        return Ok(());
    }
    if let Some(code) = obj.downcast_ref::<PyCode>() {
        let filename = code.code.source_path.as_str();
        let name = code.code.obj_name.as_str();
        let first_line = code.code.first_line_number.map_or(0, |n| n.get());
        encoder.write_array_len(2);
        encoder.write_uint(TAG_CODE);
        encoder.write_array_len(3);
        encoder.write_text(filename);
        encoder.write_text(name);
        encoder.write_uint(first_line as u64);
        return Ok(());
    }
    let type_name = obj.class().name();
    Err(SnapshotError::msg(format!(
        "unsupported dict/set key type: {type_name}"
    )))
}

fn write_tagged_key(encoder: &mut CborWriter, tag: u64, f: impl FnOnce(&mut CborWriter)) {
    encoder.write_array_len(2);
    encoder.write_uint(tag);
    f(encoder);
}

fn cbor_key_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

struct SnapshotReader<'a> {
    vm: &'a VirtualMachine,
    entries: &'a [ObjectEntry],
    root: ObjId,
    objects: Vec<Option<PyObjectRef>>,
    filled: Vec<bool>,
    /// Track which objects are currently being restored to detect cycles
    restoring: Vec<bool>,
}

impl<'a> SnapshotReader<'a> {
    fn new(vm: &'a VirtualMachine, entries: &'a [ObjectEntry], root: ObjId) -> Self {
        Self {
            vm,
            entries,
            root,
            objects: vec![None; entries.len()],
            filled: vec![false; entries.len()],
            restoring: vec![false; entries.len()],
        }
    }

    fn restore_all(mut self) -> Result<Vec<PyObjectRef>, SnapshotError> {
        for idx in 0..self.entries.len() {
            self.restore_entry(idx)?;
        }
        for idx in 0..self.entries.len() {
            self.fill_container(idx)?;
        }
        for idx in 0..self.entries.len() {
            self.apply_instance_state(idx)?;
        }
        Ok(self.objects.into_iter().map(|o| o.unwrap()).collect())
    }

    fn restore_entry(&mut self, idx: usize) -> Result<(), SnapshotError> {
        // Already restored
        if self.objects[idx].is_some() {
            return Ok(());
        }
        
        // Cycle detection: if we're already restoring this object, we have a cycle
        if self.restoring[idx] {
            let entry = &self.entries[idx];
            // For cycles, we'll create a placeholder and handle it later
            // This shouldn't happen with the two-phase serialization, but check anyway
            return Err(SnapshotError::msg(format!("cycle detected while restoring object {} (tag={:?})", idx, entry.tag)));
        }
        
        self.restoring[idx] = true;
        let entry = &self.entries[idx];
        let obj = match &entry.payload {
            ObjectPayload::None => self.vm.ctx.none(),
            ObjectPayload::Bool(value) => self.vm.ctx.new_bool(*value).into(),
            ObjectPayload::Int(value) => {
                let int = value
                    .parse::<malachite_bigint::BigInt>()
                    .map_err(|_| SnapshotError::msg("invalid int"))?;
                self.vm.ctx.new_int(int).into()
            }
            ObjectPayload::Float(value) => self.vm.ctx.new_float(*value).into(),
            ObjectPayload::Str(value) => self.vm.ctx.new_str(value.clone()).into(),
            ObjectPayload::Bytes(value) => self.vm.ctx.new_bytes(value.clone()).into(),
            ObjectPayload::List(_) => self.vm.ctx.new_list(Vec::new()).into(),
            ObjectPayload::Dict(_) => self.vm.ctx.new_dict().into(),
            ObjectPayload::Set(_) => PySet::default().into_ref(&self.vm.ctx).into(),
            ObjectPayload::FrozenSet(items) => {
                let values = items
                    .iter()
                    .map(|id| self.get_obj(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                let frozen = PyFrozenSet::from_iter(self.vm, values)
                    .map_err(|_| SnapshotError::msg("frozenset build failed"))?;
                frozen.into_ref(&self.vm.ctx).into()
            }
            ObjectPayload::Tuple(items) => {
                let values = items
                    .iter()
                    .map(|id| self.get_obj(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                self.vm.new_tuple(values).into()
            }
            ObjectPayload::Module { name, dict } => {
                let dict = self.get_obj(*dict)?;
                let dict = PyDictRef::try_from_object(self.vm, dict)
                    .map_err(|_| SnapshotError::msg("module dict invalid"))?;
                self.vm.new_module(name, dict.clone(), None).into()
            }
            ObjectPayload::BuiltinModule { name } => lookup_module(self.vm, name)?,
            ObjectPayload::BuiltinDict { name } => {
                let module = lookup_module(self.vm, name)?;
                let dict = module
                    .dict()
                    .ok_or_else(|| SnapshotError::msg("builtin module missing dict"))?;
                dict.into()
            }
            ObjectPayload::Function(payload) => {
                let code_obj = self.get_obj(payload.code)?;
                let code = code_obj
                    .downcast_ref::<PyCode>()
                    .ok_or_else(|| SnapshotError::msg("function code invalid"))?
                    .to_owned();
                let globals_obj = self.get_obj(payload.globals)?;
                let globals = self.resolve_globals(globals_obj, Some(payload.module))?;
                let mut func = PyFunction::new(code, globals.clone(), self.vm)
                    .map_err(|_| SnapshotError::msg("function create failed"))?;
                if let Some(defaults) = payload.defaults {
                    let obj = self.get_obj(defaults)?;
                    func
                        .set_function_attribute(crate::bytecode::MakeFunctionFlags::DEFAULTS, obj, self.vm)
                        .map_err(|_| SnapshotError::msg("defaults invalid"))?;
                }
                if let Some(kwdefaults) = payload.kwdefaults {
                    let obj = self.get_obj(kwdefaults)?;
                    if let Ok(dict) = PyDictRef::try_from_object(self.vm, obj.clone()) {
                        func
                            .set_function_attribute(
                                crate::bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS,
                                dict.into(),
                                self.vm,
                            )
                            .map_err(|_| SnapshotError::msg("kwdefaults invalid"))?;
                    } else if let Ok(dict) = mapping_to_dict(self.vm, &obj) {
                        func
                            .set_function_attribute(
                                crate::bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS,
                                dict.into(),
                                self.vm,
                            )
                            .map_err(|_| SnapshotError::msg("kwdefaults invalid"))?;
                    }
                }
                if let Some(closure_id) = payload.closure {
                    let obj = self.get_obj(closure_id)?;
                    func
                        .set_function_attribute(crate::bytecode::MakeFunctionFlags::CLOSURE, obj, self.vm)
                        .map_err(|_| SnapshotError::msg("closure invalid"))?;
                }
                let annotations_obj = self.get_obj(payload.annotations)?;
                let annotations_obj = match PyDictRef::try_from_object(self.vm, annotations_obj.clone()) {
                    Ok(dict) => dict.into(),
                    Err(_) => match mapping_to_dict(self.vm, &annotations_obj) {
                        Ok(dict) => dict.into(),
                        Err(_) => annotations_obj,
                    },
                };
                func
                    .set_function_attribute(crate::bytecode::MakeFunctionFlags::ANNOTATIONS, annotations_obj, self.vm)
                    .map_err(|_| SnapshotError::msg("annotations invalid"))?;
                let type_params_obj = self.get_obj(payload.type_params)?;
                func
                    .set_function_attribute(crate::bytecode::MakeFunctionFlags::TYPE_PARAMS, type_params_obj, self.vm)
                    .map_err(|_| SnapshotError::msg("type params invalid"))?;
                let func_ref = func.into_ref(&self.vm.ctx);
                let func_obj: PyObjectRef = func_ref.clone().into();
                let name = self.get_obj(payload.name)?;
                func_obj
                    .set_attr("__name__", name, self.vm)
                    .map_err(|_| SnapshotError::msg("name invalid"))?;
                let qualname = self.get_obj(payload.qualname)?;
                func_obj
                    .set_attr("__qualname__", qualname, self.vm)
                    .map_err(|_| SnapshotError::msg("qualname invalid"))?;
                let module = self.get_obj(payload.module)?;
                func_obj
                    .set_attr("__module__", module, self.vm)
                    .map_err(|_| SnapshotError::msg("module invalid"))?;
                let doc = self.get_obj(payload.doc)?;
                func_obj
                    .set_attr("__doc__", doc, self.vm)
                    .map_err(|_| SnapshotError::msg("doc invalid"))?;
                func_obj
            }
            ObjectPayload::BuiltinFunction(payload) => {
                if let Some(self_id) = payload.self_obj {
                    let target = self.get_obj(self_id)?;
                    let attr = self.vm.ctx.intern_str(payload.name.as_str());
                    target
                        .get_attr(attr, self.vm)
                        .map_err(|_| SnapshotError::msg("builtin method lookup failed"))?
                } else {
                    let module_name = payload.module.as_deref().unwrap_or("builtins");
                    
                    // Special case: maketrans is actually str.maketrans, not builtins.maketrans
                    if module_name == "builtins" && payload.name == "maketrans" {
                        let attr = self.vm.ctx.intern_str("maketrans");
                        self.vm.ctx.types.str_type.as_object().get_attr(attr, self.vm)
                            .map_err(|_| SnapshotError::msg("str.maketrans not found"))?
                    } else {
                        let module = lookup_module(self.vm, module_name)?;
                        let attr = self.vm.ctx.intern_str(payload.name.as_str());
                        module
                            .get_attr(attr, self.vm)
                            .map_err(|e| {
                                SnapshotError::msg(format!("builtin function lookup failed: {}.{}", module_name, payload.name))
                            })?
                    }
                }
            }
            ObjectPayload::Code(bytes) => {
                let code = deserialize_code_object(self.vm, bytes)?;
                let code_ref: crate::PyRef<PyCode> = self.vm.ctx.new_pyref(PyCode::new(code));
                code_ref.into()
            }
            ObjectPayload::Type(payload) => {
                // Phase 1: Create type with object as base and empty attributes to avoid cycles
                // Real bases and attributes will be set in fill_container phase
                let temp_bases = vec![self.vm.ctx.types.object_type.to_owned()];
                let empty_attrs = crate::builtins::type_::PyAttributes::default();
                let mut slots = crate::types::PyTypeSlots::heap_default();
                slots.flags = crate::types::PyTypeFlags::from_bits_truncate(payload.flags);
                slots.basicsize = payload.basicsize;
                slots.itemsize = payload.itemsize;
                slots.member_count = payload.member_count;
                let metatype = self.vm.ctx.types.type_type.to_owned();
                let typ = crate::builtins::type_::PyType::new_heap(
                    payload.name.as_str(),
                    temp_bases,
                    empty_attrs,
                    slots,
                    metatype,
                    &self.vm.ctx,
                )
                .map_err(|e| SnapshotError::msg(format!("type create failed: {e}")))?;
                let typ_obj: PyObjectRef = typ.clone().into();
                if payload.qualname != payload.name {
                    typ_obj
                        .set_attr("__qualname__", self.vm.ctx.new_str(payload.qualname.clone()), self.vm)
                        .map_err(|_| SnapshotError::msg("type qualname invalid"))?;
                }
                typ_obj
            }
            ObjectPayload::BuiltinType { module, name } => {
                // Some builtin types (iterators, views, generators, descriptors, wrappers, etc.) cannot be properly restored
                // Use the type class itself instead
                if name.ends_with("_iterator") 
                    || name.ends_with("iterator")
                    || name.ends_with("_descriptor")  // wrapper_descriptor, method_descriptor, etc.
                    || name.ends_with("-wrapper")  // method-wrapper, etc.
                    || name.starts_with("dict_")  // dict_keys, dict_values, dict_items
                    || name.contains("_wrapper")  // slot_wrapper, etc.
                    || name == "generator"
                    || name == "coroutine"
                    || name == "async_generator" {
                    self.vm.ctx.types.type_type.to_owned().into()
                } else {
                    // Handle module and type name aliases
                    let (actual_module, actual_name) = match (module.as_str(), name.as_str()) {
                        ("thread", "lock") => ("_thread", "LockType"),
                        ("builtins", "weakref") => ("weakref", "ref"),
                        ("builtins", "weakproxy") => ("weakref", "ProxyType"),
                        ("builtins", "code") => ("types", "CodeType"),
                        ("builtins", "EllipsisType") => ("types", "EllipsisType"),
                        ("builtins", "function") => ("types", "FunctionType"),
                        ("builtins", "mappingproxy") => ("types", "MappingProxyType"),
                        ("builtins", "cell") => ("types", "CellType"),
                        ("builtins", "method") => ("types", "MethodType"),
                        ("builtins", "builtin_function_or_method") => ("types", "BuiltinMethodType"),
                        ("builtins", "builtin_method") => ("types", "BuiltinMethodType"),
                        ("builtins", "module") => ("types", "ModuleType"),
                        ("builtins", "traceback") => ("types", "TracebackType"),
                        ("builtins", "frame") => ("types", "FrameType"),
                        ("builtins", "NoneType") => ("types", "NoneType"),
                        ("builtins", "NotImplementedType") => ("types", "NotImplementedType"),
                        _ => (module.as_str(), name.as_str()),
                    };
                    
                    let module_obj = lookup_module(self.vm, actual_module)?;
                    let attr = self.vm.ctx.intern_str(actual_name);
                    module_obj
                        .get_attr(attr, self.vm)
                        .map_err(|e| {
                            SnapshotError::msg(format!("builtin type not found: {}.{}", module, name))
                        })?
                }
            }
            ObjectPayload::Instance(payload) => {
                let typ_obj = self.get_obj(payload.typ)?;
                let typ = match typ_obj.clone().downcast::<PyType>() {
                    Ok(typ) => typ,
                    Err(obj) => obj.class().to_owned(),
                };
                let type_name = typ.name().to_owned();
                
                // Special case: if this is a type instance, it should not be created via __new__
                // Use the type itself
                if type_name == "type" {
                    typ.clone().into()
                } else
                
                // Some types cannot be properly restored (weakref, iterators, methods, slices, etc.)
                // Return None for these cases
                if type_name == "weakref" 
                    || type_name == "weakproxy" 
                    || type_name == "method"  // bound methods need specific object binding
                    || type_name == "builtin_method"
                    || type_name == "slice"  // slice objects need specific start/stop/step
                    || type_name.ends_with("_iterator")
                    || type_name.ends_with("iterator") {
                    self.vm.ctx.none()
                } else {
                let args_obj = payload
                    .new_args
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let kwargs_obj = payload
                    .new_kwargs
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let args_obj = args_obj.unwrap_or_else(|| self.vm.ctx.empty_tuple.clone().into());
                let args = if let Some(tuple) = args_obj.downcast_ref::<PyTuple>() {
                    tuple.to_owned()
                } else if let Some(list) = args_obj.downcast_ref::<PyList>() {
                    self.vm.new_tuple(list.borrow_vec().to_vec())
                } else {
                    self.vm.ctx.empty_tuple.clone()
                };
                if typ.is(self.vm.ctx.types.classmethod_type)
                    || typ.is(self.vm.ctx.types.staticmethod_type)
                {
                    let func = args
                        .get(0)
                        .cloned()
                        .unwrap_or_else(|| self.vm.ctx.none().into());
                    if typ.is(self.vm.ctx.types.classmethod_type) {
                        let obj: PyObjectRef = PyClassMethod::from(func)
                            .into_ref_with_type(self.vm, typ.clone())
                            .map_err(|_| SnapshotError::msg("classmethod create failed"))?
                            .into();
                        obj
                    } else {
                        let obj: PyObjectRef = PyStaticMethod::new(func)
                            .into_ref_with_type(self.vm, typ.clone())
                            .map_err(|_| SnapshotError::msg("staticmethod create failed"))?
                            .into();
                        obj
                    }
                } else {
                let new_func = self
                    .vm
                    .get_attribute_opt(typ.clone().into(), "__new__")
                    .map_err(|_| SnapshotError::msg("__new__ lookup failed"))?;
                let kwargs_obj = kwargs_obj.unwrap_or_else(|| self.vm.ctx.new_dict().into());
                let kwargs = if let Ok(dict) = PyDictRef::try_from_object(self.vm, kwargs_obj.clone()) {
                    dict
                } else if let Ok(dict) = mapping_to_dict(self.vm, &kwargs_obj) {
                    dict
                } else {
                    self.vm.ctx.new_dict()
                };
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(typ.clone().into());
                call_args.extend(args.iter().cloned());
                let kwargs = kwargs_from_dict(kwargs)?;
                let instance = if let Some(new_func) = new_func {
                    match new_func.call(crate::function::FuncArgs::new(call_args.clone(), kwargs.clone()), self.vm) {
                        Ok(value) => value,
                        Err(_) => {
                            self
                            .vm
                            .call_method(self.vm.ctx.types.object_type.as_object(), "__new__", (typ.clone(),))
                            .map_err(|_| {
                                SnapshotError::msg(format!("__new__ failed for {type_name}"))
                            })?
                        }
                    }
                } else {
                    self.vm
                        .call_method(self.vm.ctx.types.object_type.as_object(), "__new__", (typ.clone(),))
                        .map_err(|_| SnapshotError::msg(format!("__new__ missing for {type_name}")))?
                };
                instance
                }
                }
            }
            ObjectPayload::Cell(contents) => {
                let value = contents
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let cell = PyCell::new(value);
                let cell_ref: crate::PyRef<PyCell> = self.vm.ctx.new_pyref(cell);
                cell_ref.into()
            }
            ObjectPayload::Enumerate { iterator, count } => {
                // Restore enumerate object
                // Important: get_obj will trigger list_iterator restoration with __setstate__
                let iter_obj = self.get_obj(*iterator)
                    .map_err(|e| SnapshotError::msg(format!("enumerate: failed to get iterator {}: {:?}", iterator, e)))?;
                
                // Call enumerate(iter, start=count) to recreate
                // Note: iter_obj should already be at the correct position after get_obj
                let enumerate_fn = self.vm.builtins.get_attr("enumerate", self.vm)
                    .map_err(|e| SnapshotError::msg(format!("enumerate: builtin not found: {:?}", e)))?;
                let count_obj = self.vm.ctx.new_int(*count);
                
                // Create kwargs with "start" parameter
                use crate::function::{FuncArgs, KwArgs};
                use indexmap::IndexMap;
                let mut kwargs_map = IndexMap::new();
                kwargs_map.insert("start".to_string(), count_obj.into());
                let kwargs = KwArgs::new(kwargs_map);
                // Use iter_obj directly (don't clone, as it's already the restored iterator)
                let args = FuncArgs::new(vec![iter_obj], kwargs);
                
                self.vm.invoke(&enumerate_fn, args)
                    .map_err(|e| SnapshotError::msg(format!("enumerate(iterator={}, start={}) failed: {:?}", iterator, count, e)))?
            }
            ObjectPayload::Zip { iterators } => {
                // Restore zip object
                let iter_objs: Result<Vec<_>, _> = iterators.iter()
                    .map(|id| self.get_obj(*id))
                    .collect();
                let iter_objs = iter_objs?;
                let zip_fn = self.vm.builtins.get_attr("zip", self.vm)
                    .map_err(|_| SnapshotError::msg("zip not found"))?;
                self.vm.invoke(&zip_fn, iter_objs)
                    .map_err(|_| SnapshotError::msg("zip restore failed"))?
            }
            ObjectPayload::Map { function, iterator } => {
                // Restore map object
                let func_obj = self.get_obj(*function)?;
                let iter_obj = self.get_obj(*iterator)?;
                let map_fn = self.vm.builtins.get_attr("map", self.vm)
                    .map_err(|_| SnapshotError::msg("map not found"))?;
                self.vm.invoke(&map_fn, (func_obj, iter_obj))
                    .map_err(|_| SnapshotError::msg("map restore failed"))?
            }
            ObjectPayload::Filter { function, iterator } => {
                // Restore filter object
                let func_obj = self.get_obj(*function)?;
                let iter_obj = self.get_obj(*iterator)?;
                let filter_fn = self.vm.builtins.get_attr("filter", self.vm)
                    .map_err(|_| SnapshotError::msg("filter not found"))?;
                self.vm.invoke(&filter_fn, (func_obj, iter_obj))
                    .map_err(|_| SnapshotError::msg("filter restore failed"))?
            }
            ObjectPayload::ListIterator { list, position } => {
                // Restore list_iterator object
                // 1. Get the list object and fill it
                let list_obj = self.get_obj(*list)?;
                
                // IMPORTANT: fill_container must be called to populate the list's elements
                // Otherwise the list will be empty!
                let list_idx = *list as usize;
                self.fill_container(list_idx)?;
                
                // 2. Create a new iterator from the list
                let iter_fn = self.vm.builtins.get_attr("iter", self.vm)
                    .map_err(|_| SnapshotError::msg("iter not found"))?;
                let new_iter = self.vm.invoke(&iter_fn, (list_obj.clone(),))
                    .map_err(|e| SnapshotError::msg(format!("iter() failed: {:?}", e)))?;
                
                // 3. Advance the iterator to the saved position by calling __next__
                for _ in 0..*position {
                    match self.vm.call_method(&new_iter, "__next__", ()) {
                        Ok(_) => {
                            // Successfully advanced, continue
                        }
                        Err(e) => {
                            // Check if it's StopIteration (iterator exhausted early)
                            let class_name = e.class().name();
                            if &*class_name == "StopIteration" {
                                // Iterator exhausted before reaching target position, break
                                break;
                            } else {
                                // Other error, propagate
                                return Err(SnapshotError::msg(format!("list_iterator advance failed: {:?}", e)));
                            }
                        }
                    }
                }
                
                new_iter
            }
            ObjectPayload::Range { start, stop, step } => {
                // Restore range object by calling range(start, stop, step)
                let range_fn = self.vm.builtins.get_attr("range", self.vm)
                    .map_err(|_| SnapshotError::msg("range not found"))?;
                let start_obj = self.vm.ctx.new_int(*start);
                let stop_obj = self.vm.ctx.new_int(*stop);
                let step_obj = self.vm.ctx.new_int(*step);
                self.vm.invoke(&range_fn, (start_obj, stop_obj, step_obj))
                    .map_err(|e| SnapshotError::msg(format!("range({}, {}, {}) failed: {:?}", start, stop, step, e)))?
            }
            ObjectPayload::RangeIterator { range, position } => {
                // Restore range_iterator object
                // 1. Get the range object
                let range_obj = self.get_obj(*range)?;
                
                // 2. Create a new iterator from the range
                let iter_fn = self.vm.builtins.get_attr("iter", self.vm)
                    .map_err(|_| SnapshotError::msg("iter not found"))?;
                let new_iter = self.vm.invoke(&iter_fn, (range_obj.clone(),))
                    .map_err(|e| SnapshotError::msg(format!("iter(range) failed: {:?}", e)))?;
                
                // 3. Advance the iterator to the saved position by calling __next__
                for _ in 0..*position {
                    match self.vm.call_method(&new_iter, "__next__", ()) {
                        Ok(_) => {
                            // Successfully advanced, continue
                        }
                        Err(e) => {
                            // Check if it's StopIteration (iterator exhausted early)
                            let class_name = e.class().name();
                            if &*class_name == "StopIteration" {
                                // Iterator exhausted before reaching target position, break
                                break;
                            } else {
                                // Other error, propagate
                                return Err(SnapshotError::msg(format!("range_iterator advance failed: {:?}", e)));
                            }
                        }
                    }
                }
                
                new_iter
            }
        };
        self.objects[idx] = Some(obj);
        self.restoring[idx] = false;
        Ok(())
    }

    fn fill_container(&mut self, idx: usize) -> Result<(), SnapshotError> {
        if self.filled[idx] {
            return Ok(());
        }
        let entry = &self.entries[idx];
        let Some(obj) = self.objects[idx].clone() else {
            return Ok(());
        };
        match &entry.payload {
            ObjectPayload::List(items) => {
                let list = obj
                    .downcast_ref::<PyList>()
                    .ok_or_else(|| SnapshotError::msg("list fill type error"))?;
                let mut data = list.borrow_vec_mut();
                for id in items {
                    data.push(self.get_obj(*id)?);
                }
            }
            ObjectPayload::Dict(items) => {
                let dict = PyDictRef::try_from_object(self.vm, obj)
                    .map_err(|_| SnapshotError::msg("dict fill type error"))?;
                for (k, v) in items {
                    let key = self.get_obj(*k)?;
                    let value = self.get_obj(*v)?;
                    dict.set_item(&*key, value, self.vm)
                        .map_err(|_| SnapshotError::msg("dict fill failed"))?;
                }
            }
            ObjectPayload::Set(items) => {
                let set = obj
                    .downcast_ref::<PySet>()
                    .ok_or_else(|| SnapshotError::msg("set fill type error"))?;
                for id in items {
                    set.add(self.get_obj(*id)?, self.vm)
                        .map_err(|_| SnapshotError::msg("set add failed"))?;
                }
            }
            ObjectPayload::Type(payload) => {
                // Fill in the real bases and attributes for Type objects
                let typ = obj
                    .downcast_ref::<crate::builtins::type_::PyType>()
                    .ok_or_else(|| SnapshotError::msg("type fill type error"))?;
                
                // Fill bases
                if !payload.bases.is_empty() {
                    let mut bases = Vec::new();
                    for base_id in &payload.bases {
                        let base_obj = self.get_obj(*base_id)?;
                        let base_type = base_obj.downcast::<crate::builtins::type_::PyType>()
                            .map_err(|_| SnapshotError::msg("type base invalid"))?;
                        bases.push(base_type);
                    }
                    // Update the bases
                    *typ.bases.write() = bases;
                }
                
                // Fill attributes
                let attrs = build_type_attributes(self, payload.dict, idx as ObjId)?;
                for (key, value) in attrs.iter() {
                    typ.attributes.write().insert(key.clone(), value.clone());
                }
                
                // Apply deferred attributes
                apply_deferred_type_attrs(self, obj.clone(), payload.dict, idx as ObjId)?;
            }
            _ => {}
        }
        self.filled[idx] = true;
        Ok(())
    }

    fn apply_instance_state(&mut self, idx: usize) -> Result<(), SnapshotError> {
        let entry = &self.entries[idx];
        let ObjectPayload::Instance(payload) = &entry.payload else {
            return Ok(());
        };
        let Some(instance) = self.objects[idx].clone() else {
            return Ok(());
        };
        let Some(state_id) = payload.state else {
            return Ok(());
        };
        let state = self.get_obj(state_id)?;
        if let Some(setstate) = self
            .vm
            .get_attribute_opt(instance.clone(), "__setstate__")
            .map_err(|_| SnapshotError::msg("__setstate__ lookup failed"))?
        {
            setstate
                .call((state,), self.vm)
                .map_err(|_| SnapshotError::msg("__setstate__ failed"))?;
            return Ok(());
        }
        if let Some(dict) = instance.dict() {
            // state can be None for some objects
            if self.vm.is_none(&state) {
                return Ok(());
            }
            let state_dict = PyDictRef::try_from_object(self.vm, state.clone())
                .map_err(|_| {
                    SnapshotError::msg("state must be dict")
                })?;
            for (key, value) in &state_dict {
                dict.set_item(&*key, value, self.vm)
                    .map_err(|_| SnapshotError::msg("state set failed"))?;
            }
        }
        Ok(())
    }

    fn get_obj(&mut self, id: ObjId) -> Result<PyObjectRef, SnapshotError> {
        let idx = id as usize;
        if self.objects.get(idx).and_then(|o| o.as_ref()).is_none() {
            self.restore_entry(idx)?;
        }
        Ok(self.objects[idx].clone().unwrap())
    }

    fn resolve_globals(
        &mut self,
        globals_obj: PyObjectRef,
        module_id: Option<ObjId>,
    ) -> Result<PyDictRef, SnapshotError> {
        if let Ok(dict) = PyDictRef::try_from_object(self.vm, globals_obj.clone()) {
            return Ok(dict);
        }
        if let Some(dict) = globals_obj.dict() {
            return Ok(dict);
        }
        if let Some(module_id) = module_id {
            let module_obj = self.get_obj(module_id)?;
            if let Ok(dict) = PyDictRef::try_from_object(self.vm, module_obj.clone()) {
                return Ok(dict);
            }
            if let Some(dict) = module_obj.dict() {
                return Ok(dict);
            }
            if let Some(name) = module_obj
                .downcast_ref::<PyStr>()
                .map(|s| s.as_str().to_owned())
            {
                if let Ok(module) = lookup_module(self.vm, &name) {
                    if let Some(dict) = module.dict() {
                        return Ok(dict);
                    }
                }
            }
        }
        if let Ok(dict) = mapping_to_dict(self.vm, &globals_obj) {
            return Ok(dict);
        }
        let root_obj = self.get_obj(self.root)?;
        if let Ok(dict) = PyDictRef::try_from_object(self.vm, root_obj.clone()) {
            return Ok(dict);
        }
        if let Some(dict) = root_obj.dict() {
            return Ok(dict);
        }
        Err(SnapshotError::msg(format!(
            "function globals invalid: {}",
            globals_obj.class().name()
        )))
    }
}

fn lookup_module(vm: &VirtualMachine, name: &str) -> Result<PyObjectRef, SnapshotError> {
    if name == "builtins" {
        return Ok(vm.builtins.clone().into());
    }
    if name == "sys" {
        return Ok(vm.sys_module.clone().into());
    }
    
    // Handle module name aliases (Python 2 -> Python 3)
    let actual_name = match name {
        "thread" => "_thread",
        "_os" => "posix",  // _os is typically mapped to posix or nt
        _ => name,
    };
    
    // Try to get from sys.modules first
    let sys_modules = vm.sys_module
        .get_attr("modules", vm)
        .map_err(|_| SnapshotError::msg("sys.modules unavailable"))?;
    
    if let Ok(module) = sys_modules.get_item(actual_name, vm) {
        return Ok(module);
    }
    
    // If not found, try to import it
    let import_func = vm.builtins
        .get_attr("__import__", vm)
        .map_err(|_| SnapshotError::msg("__import__ not found"))?;
    
    match import_func.call((actual_name,), vm) {
        Ok(module) => {
            Ok(module)
        }
        Err(e) => {
            Err(SnapshotError::msg(format!("failed to import module: {name}")))
        }
    }
}

fn build_type_attributes(
    reader: &mut SnapshotReader<'_>,
    dict_id: ObjId,
    type_id: ObjId,
) -> Result<crate::builtins::type_::PyAttributes, SnapshotError> {
    if dict_id == type_id {
        return Ok(crate::builtins::type_::PyAttributes::default());
    }
    let entry = reader
        .entries
        .get(dict_id as usize)
        .ok_or_else(|| SnapshotError::msg("type dict missing"))?;
    let items = match &entry.payload {
        ObjectPayload::Dict(items) => items.clone(),
        ObjectPayload::BuiltinDict { name } => {
            let module = lookup_module(reader.vm, name)?;
            let dict = module
                .dict()
                .ok_or_else(|| SnapshotError::msg("builtin module missing dict"))?;
            return build_type_attributes_from_dict(reader, dict, type_id);
        }
        ObjectPayload::Module { dict, .. } => {
            let dict_obj = reader.get_obj(*dict)?;
            let dict = PyDictRef::try_from_object(reader.vm, dict_obj)
                .map_err(|_| SnapshotError::msg("module dict invalid"))?;
            return build_type_attributes_from_dict(reader, dict, type_id);
        }
        _ => {
            let dict_obj = reader.get_obj(dict_id)?;
            if let Ok(dict) = PyDictRef::try_from_object(reader.vm, dict_obj.clone()) {
                return build_type_attributes_from_dict(reader, dict, type_id);
            }
            if let Ok(dict) = mapping_to_dict(reader.vm, &dict_obj) {
                return build_type_attributes_from_dict(reader, dict, type_id);
            }
            return Ok(crate::builtins::type_::PyAttributes::default());
        }
    };
    let mut attrs = crate::builtins::type_::PyAttributes::default();
    for (key_id, val_id) in items {
        if key_id == type_id || val_id == type_id {
            continue;
        }
        let key_obj = reader.get_obj(key_id)?;
        let key = key_obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("type dict key must be str"))?;
        let value = reader.get_obj(val_id)?;
        let interned = reader.vm.ctx.intern_str(key.as_str());
        attrs.insert(interned, value);
    }
    Ok(attrs)
}

fn build_type_attributes_from_dict(
    reader: &mut SnapshotReader<'_>,
    dict: PyDictRef,
    type_id: ObjId,
) -> Result<crate::builtins::type_::PyAttributes, SnapshotError> {
    let mut attrs = crate::builtins::type_::PyAttributes::default();
    for (key, value) in &dict {
        let _ = type_id;
        let key = key
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("type dict key must be str"))?;
        let interned = reader.vm.ctx.intern_str(key.as_str());
        attrs.insert(interned, value);
    }
    Ok(attrs)
}

fn apply_deferred_type_attrs(
    reader: &mut SnapshotReader<'_>,
    typ_obj: PyObjectRef,
    dict_id: ObjId,
    type_id: ObjId,
) -> Result<(), SnapshotError> {
    let entry = reader
        .entries
        .get(dict_id as usize)
        .ok_or_else(|| SnapshotError::msg("type dict missing"))?;
    let items = match &entry.payload {
        ObjectPayload::Dict(items) => items.clone(),
        _ => return Ok(()),
    };
    for (key_id, val_id) in items {
        if key_id != type_id && val_id != type_id {
            continue;
        }
        let key_obj = reader.get_obj(key_id)?;
        let key = key_obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("type dict key must be str"))?;
        let value = reader.get_obj(val_id)?;
        let key_interned = reader.vm.ctx.intern_str(key.as_str());
        typ_obj
            .set_attr(key_interned, value, reader.vm)
            .map_err(|_| SnapshotError::msg("type attribute set failed"))?;
    }
    Ok(())
}

fn kwargs_from_dict(dict: PyDictRef) -> Result<crate::function::KwArgs, SnapshotError> {
    let mut map = indexmap::IndexMap::new();
    for (key, value) in &dict {
        let key = key
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("kwargs key must be str"))?;
        map.insert(key.as_str().to_owned(), value);
    }
    Ok(crate::function::KwArgs::new(map))
}

fn mapping_to_dict(vm: &VirtualMachine, mapping: &PyObjectRef) -> Result<PyDictRef, SnapshotError> {
    let items = vm
        .call_method(mapping.as_object(), "items", ())
        .map_err(|_| SnapshotError::msg("globals items() failed"))?;
    let iter = items
        .get_iter(vm)
        .map_err(|_| SnapshotError::msg("globals items() not iterable"))?;
    let dict = vm.ctx.new_dict();
    loop {
        let next = iter
            .next(vm)
            .map_err(|_| SnapshotError::msg("globals items() iteration failed"))?;
        let PyIterReturn::Return(item) = next else {
            break;
        };
        let (key, value) = if let Some(pair) = item.downcast_ref::<PyTuple>() {
            if pair.len() != 2 {
                return Err(SnapshotError::msg("globals item must be (key, value)"));
            }
            (
                pair.get(0).unwrap().clone(),
                pair.get(1).unwrap().clone(),
            )
        } else if let Some(pair) = item.downcast_ref::<PyList>() {
            if pair.borrow_vec().len() != 2 {
                return Err(SnapshotError::msg("globals item must be [key, value]"));
            }
            let values = pair.borrow_vec();
            (values[0].clone(), values[1].clone())
        } else {
            return Err(SnapshotError::msg("globals item must be tuple/list"));
        };
        dict.set_item(&*key, value, vm)
            .map_err(|_| SnapshotError::msg("globals item set failed"))?;
    }
    Ok(dict)
}

#[derive(Debug, Clone)]
struct CborWriter {
    buf: Vec<u8>,
}

impl CborWriter {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn write_uint(&mut self, value: u64) {
        write_uint_major(&mut self.buf, 0, value);
    }

    fn write_bytes(&mut self, value: &[u8]) {
        write_uint_major(&mut self.buf, 2, value.len() as u64);
        self.buf.extend_from_slice(value);
    }

    fn write_text(&mut self, value: &str) {
        write_uint_major(&mut self.buf, 3, value.len() as u64);
        self.buf.extend_from_slice(value.as_bytes());
    }

    fn write_array_len(&mut self, len: usize) {
        write_uint_major(&mut self.buf, 4, len as u64);
    }

    fn write_map_len(&mut self, len: usize) {
        write_uint_major(&mut self.buf, 5, len as u64);
    }

    fn write_bool(&mut self, value: bool) {
        self.buf.push(if value { 0xf5 } else { 0xf4 });
    }

    fn write_null(&mut self) {
        self.buf.push(0xf6);
    }

    fn write_f64(&mut self, value: f64) {
        self.buf.push(0xfb);
        self.buf.extend_from_slice(&value.to_be_bytes());
    }
}

fn write_uint_major(buf: &mut Vec<u8>, major: u8, value: u64) {
    if value < 24 {
        buf.push((major << 5) | value as u8);
    } else if value <= u8::MAX as u64 {
        buf.push((major << 5) | 24);
        buf.push(value as u8);
    } else if value <= u16::MAX as u64 {
        buf.push((major << 5) | 25);
        buf.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u32::MAX as u64 {
        buf.push((major << 5) | 26);
        buf.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        buf.push((major << 5) | 27);
        buf.extend_from_slice(&value.to_be_bytes());
    }
}

#[derive(Debug)]
struct CborReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> CborReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read_u8(&mut self) -> Result<u8, SnapshotError> {
        let b = *self.data.get(self.pos).ok_or_else(|| SnapshotError::msg("cbor eof"))?;
        self.pos += 1;
        Ok(b)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.pos + len;
        let slice = self.data.get(self.pos..end).ok_or_else(|| SnapshotError::msg("cbor eof"))?;
        self.pos = end;
        Ok(slice)
    }

    fn read_uint(&mut self, info: u8) -> Result<u64, SnapshotError> {
        match info {
            0..=23 => Ok(info as u64),
            24 => Ok(self.read_u8()? as u64),
            25 => Ok(u16::from_be_bytes(self.read_exact(2)?.try_into().unwrap()) as u64),
            26 => Ok(u32::from_be_bytes(self.read_exact(4)?.try_into().unwrap()) as u64),
            27 => Ok(u64::from_be_bytes(self.read_exact(8)?.try_into().unwrap())),
            _ => Err(SnapshotError::msg("unsupported uint")),
        }
    }

    fn read_value(&mut self) -> Result<CborValue, SnapshotError> {
        let head = self.read_u8()?;
        let major = head >> 5;
        let info = head & 0x1f;
        match major {
            0 => Ok(CborValue::Uint(self.read_uint(info)?)),
            1 => Ok(CborValue::Nint(self.read_uint(info)?)),
            2 => {
                let len = self.read_uint(info)? as usize;
                let bytes = self.read_exact(len)?.to_vec();
                Ok(CborValue::Bytes(bytes))
            }
            3 => {
                let len = self.read_uint(info)? as usize;
                let bytes = self.read_exact(len)?.to_vec();
                let text = String::from_utf8(bytes).map_err(|_| SnapshotError::msg("utf8 error"))?;
                Ok(CborValue::Text(text))
            }
            4 => {
                let len = self.read_uint(info)? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(self.read_value()?);
                }
                Ok(CborValue::Array(items))
            }
            5 => {
                let len = self.read_uint(info)? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    let key = self.read_value()?;
                    let val = self.read_value()?;
                    items.push((key, val));
                }
                Ok(CborValue::Map(items))
            }
            7 => match info {
                20 => Ok(CborValue::Bool(false)),
                21 => Ok(CborValue::Bool(true)),
                22 => Ok(CborValue::Null),
                27 => {
                    let bytes = self.read_exact(8)?;
                    Ok(CborValue::Float(f64::from_be_bytes(bytes.try_into().unwrap())))
                }
                _ => Err(SnapshotError::msg("unsupported simple")),
            },
            _ => Err(SnapshotError::msg("unsupported major")),
        }
    }
}

#[derive(Debug, Clone)]
enum CborValue {
    Uint(u64),
    Nint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(Vec<(CborValue, CborValue)>),
    Bool(bool),
    Null,
    Float(f64),
}

fn encode_checkpoint_state(state: &CheckpointState) -> Vec<u8> {
    let mut writer = CborWriter::new();
    let mut fields = Vec::new();
    fields.push(("version", CborValue::Uint(state.version as u64)));
    fields.push(("source_path", CborValue::Text(state.source_path.clone())));
    
    // Encode frames array
    let frames_array = state.frames.iter().map(|frame_state| {
        let stack_array = frame_state.stack.iter()
            .map(|obj_id| CborValue::Uint(*obj_id as u64))
            .collect::<Vec<_>>();
        
        // Encode blocks array
        let blocks_array = frame_state.blocks.iter().map(|block_state| {
            encode_block_state(block_state)
        }).collect::<Vec<_>>();
        
        CborValue::Map(vec![
            (CborValue::Text("code".to_owned()), CborValue::Bytes(frame_state.code.clone())),
            (CborValue::Text("lasti".to_owned()), CborValue::Uint(frame_state.lasti as u64)),
            (CborValue::Text("locals".to_owned()), CborValue::Uint(frame_state.locals as u64)),
            (CborValue::Text("stack".to_owned()), CborValue::Array(stack_array)),
            (CborValue::Text("blocks".to_owned()), CborValue::Array(blocks_array)),
        ])
    }).collect::<Vec<_>>();
    fields.push(("frames", CborValue::Array(frames_array)));
    
    fields.push(("root", CborValue::Uint(state.root as u64)));
    let objects = state
        .objects
        .iter()
        .map(encode_object_entry)
        .collect::<Vec<_>>();
    fields.push(("objects", CborValue::Array(objects)));
    write_cbor_map(&mut writer, fields);
    writer.into_bytes()
}

fn decode_checkpoint_state(data: &[u8]) -> Result<CheckpointState, SnapshotError> {
    let mut reader = CborReader::new(data);
    let value = reader.read_value()?;
    let map = match value {
        CborValue::Map(map) => map,
        _ => return Err(SnapshotError::msg("checkpoint is not map")),
    };
    let mut version = None;
    let mut source_path = None;
    let mut frames_data = None;
    // Old format fields (for backward compatibility)
    let mut lasti = None;
    let mut code = None;
    let mut root = None;
    let mut objects = None;
    for (key, val) in map {
        let key = match key {
            CborValue::Text(text) => text,
            _ => return Err(SnapshotError::msg("invalid map key")),
        };
        match key.as_str() {
            "version" => version = Some(expect_uint(val)? as u32),
            "source_path" => source_path = Some(expect_text(val)?),
            "frames" => {
                let arr = expect_array(val)?;
                let mut frame_states = Vec::new();
                for frame_val in arr {
                    let frame_map = expect_map(frame_val)?;
                    let mut f_code = None;
                    let mut f_lasti = None;
                    let mut f_locals = None;
                    let mut f_stack = None;
                    let mut f_blocks = None;
                    for (k, v) in frame_map {
                        let k = expect_text(k)?;
                        match k.as_str() {
                            "code" => f_code = Some(expect_bytes(v)?),
                            "lasti" => f_lasti = Some(expect_uint(v)? as u32),
                            "locals" => f_locals = Some(expect_uint(v)? as ObjId),
                            "stack" => {
                                let arr = expect_array(v)?;
                                let mut stack_ids = Vec::new();
                                for item in arr {
                                    stack_ids.push(expect_uint(item)? as ObjId);
                                }
                                f_stack = Some(stack_ids);
                            }
                            "blocks" => {
                                let arr = expect_array(v)?;
                                let mut blocks = Vec::new();
                                for block_val in arr {
                                    blocks.push(decode_block_state(block_val)?);
                                }
                                f_blocks = Some(blocks);
                            }
                            _ => {}
                        }
                    }
                    frame_states.push(FrameState {
                        code: f_code.ok_or_else(|| SnapshotError::msg("missing frame code"))?,
                        lasti: f_lasti.ok_or_else(|| SnapshotError::msg("missing frame lasti"))?,
                        locals: f_locals.ok_or_else(|| SnapshotError::msg("missing frame locals"))?,
                        stack: f_stack.unwrap_or_else(Vec::new),  // For backward compatibility
                        blocks: f_blocks.unwrap_or_else(Vec::new),  // For backward compatibility
                    });
                }
                frames_data = Some(frame_states);
            }
            // Old format fields
            "lasti" => lasti = Some(expect_uint(val)? as u32),
            "code" => code = Some(expect_bytes(val)?),
            "root" => root = Some(expect_uint(val)? as ObjId),
            "objects" => {
                let arr = expect_array(val)?;
                let mut entries = Vec::new();
                for item in arr {
                    entries.push(decode_object_entry(item)?);
                }
                objects = Some(entries);
            }
            _ => {}
        }
    }
    
    let root = root.ok_or_else(|| SnapshotError::msg("missing root"))?;
    
    // Handle backward compatibility: if 'frames' field doesn't exist, convert old format
    let frames = if let Some(frames_data) = frames_data {
        frames_data
    } else {
        // Old format: single frame
        let lasti = lasti.ok_or_else(|| SnapshotError::msg("missing lasti"))?;
        let code = code.ok_or_else(|| SnapshotError::msg("missing code"))?;
        vec![FrameState {
            code,
            lasti,
            locals: root,  // Old format: locals == globals for module-level
            stack: Vec::new(),  // Old format: assume empty stack
            blocks: Vec::new(),  // Old format: assume empty blocks
        }]
    };
    
    Ok(CheckpointState {
        version: version.ok_or_else(|| SnapshotError::msg("missing version"))?,
        source_path: source_path.ok_or_else(|| SnapshotError::msg("missing source_path"))?,
        frames,
        root,
        objects: objects.ok_or_else(|| SnapshotError::msg("missing objects"))?,
    })
}

fn encode_object_entry(entry: &ObjectEntry) -> CborValue {
    let payload = match &entry.payload {
        ObjectPayload::None => CborValue::Null,
        ObjectPayload::Bool(value) => CborValue::Bool(*value),
        ObjectPayload::Int(value) => CborValue::Text(value.clone()),
        ObjectPayload::Float(value) => CborValue::Float(*value),
        ObjectPayload::Str(value) => CborValue::Text(value.clone()),
        ObjectPayload::Bytes(value) => CborValue::Bytes(value.clone()),
        ObjectPayload::List(items) => CborValue::Array(items.iter().map(|id| CborValue::Uint(*id as u64)).collect()),
        ObjectPayload::Tuple(items) => CborValue::Array(items.iter().map(|id| CborValue::Uint(*id as u64)).collect()),
        ObjectPayload::Dict(items) => CborValue::Array(items.iter().map(|(k, v)| {
            CborValue::Array(vec![CborValue::Uint(*k as u64), CborValue::Uint(*v as u64)])
        }).collect()),
        ObjectPayload::Set(items) => CborValue::Array(items.iter().map(|id| CborValue::Uint(*id as u64)).collect()),
        ObjectPayload::FrozenSet(items) => CborValue::Array(items.iter().map(|id| CborValue::Uint(*id as u64)).collect()),
        ObjectPayload::Module { name, dict } => CborValue::Map(vec![
            (CborValue::Text("name".to_owned()), CborValue::Text(name.clone())),
            (CborValue::Text("dict".to_owned()), CborValue::Uint(*dict as u64)),
        ]),
        ObjectPayload::BuiltinModule { name } => CborValue::Map(vec![
            (CborValue::Text("name".to_owned()), CborValue::Text(name.clone())),
        ]),
        ObjectPayload::BuiltinDict { name } => CborValue::Map(vec![
            (CborValue::Text("name".to_owned()), CborValue::Text(name.clone())),
        ]),
        ObjectPayload::Function(func) => CborValue::Map(vec![
            (CborValue::Text("code".to_owned()), CborValue::Uint(func.code as u64)),
            (CborValue::Text("globals".to_owned()), CborValue::Uint(func.globals as u64)),
            (CborValue::Text("defaults".to_owned()), opt_id(func.defaults)),
            (CborValue::Text("kwdefaults".to_owned()), opt_id(func.kwdefaults)),
            (CborValue::Text("closure".to_owned()), opt_id(func.closure)),
            (CborValue::Text("name".to_owned()), CborValue::Uint(func.name as u64)),
            (CborValue::Text("qualname".to_owned()), CborValue::Uint(func.qualname as u64)),
            (CborValue::Text("annotations".to_owned()), CborValue::Uint(func.annotations as u64)),
            (CborValue::Text("module".to_owned()), CborValue::Uint(func.module as u64)),
            (CborValue::Text("doc".to_owned()), CborValue::Uint(func.doc as u64)),
            (CborValue::Text("type_params".to_owned()), CborValue::Uint(func.type_params as u64)),
        ]),
        ObjectPayload::BuiltinFunction(func) => CborValue::Map(vec![
            (CborValue::Text("name".to_owned()), CborValue::Text(func.name.clone())),
            (
                CborValue::Text("module".to_owned()),
                func.module
                    .as_ref()
                    .map(|m| CborValue::Text(m.clone()))
                    .unwrap_or(CborValue::Null),
            ),
            (CborValue::Text("self".to_owned()), opt_id(func.self_obj)),
        ]),
        ObjectPayload::Code(bytes) => CborValue::Bytes(bytes.clone()),
        ObjectPayload::Type(typ) => CborValue::Map(vec![
            (CborValue::Text("name".to_owned()), CborValue::Text(typ.name.clone())),
            (CborValue::Text("qualname".to_owned()), CborValue::Text(typ.qualname.clone())),
            (CborValue::Text("bases".to_owned()), CborValue::Array(typ.bases.iter().map(|id| CborValue::Uint(*id as u64)).collect())),
            (CborValue::Text("dict".to_owned()), CborValue::Uint(typ.dict as u64)),
            (CborValue::Text("flags".to_owned()), CborValue::Uint(typ.flags)),
            (CborValue::Text("basicsize".to_owned()), CborValue::Uint(typ.basicsize as u64)),
            (CborValue::Text("itemsize".to_owned()), CborValue::Uint(typ.itemsize as u64)),
            (CborValue::Text("member_count".to_owned()), CborValue::Uint(typ.member_count as u64)),
        ]),
        ObjectPayload::BuiltinType { module, name } => CborValue::Map(vec![
            (CborValue::Text("module".to_owned()), CborValue::Text(module.clone())),
            (CborValue::Text("name".to_owned()), CborValue::Text(name.clone())),
        ]),
        ObjectPayload::Instance(inst) => CborValue::Map(vec![
            (CborValue::Text("type".to_owned()), CborValue::Uint(inst.typ as u64)),
            (CborValue::Text("state".to_owned()), opt_id(inst.state)),
            (CborValue::Text("new_args".to_owned()), opt_id(inst.new_args)),
            (CborValue::Text("new_kwargs".to_owned()), opt_id(inst.new_kwargs)),
        ]),
        ObjectPayload::Cell(value) => opt_id(*value),
        ObjectPayload::Enumerate { iterator, count } => CborValue::Map(vec![
            (CborValue::Text("iterator".to_owned()), CborValue::Uint(*iterator as u64)),
            (CborValue::Text("count".to_owned()), if *count >= 0 {
                CborValue::Uint(*count as u64)
            } else {
                CborValue::Nint((-*count - 1) as u64)
            }),
        ]),
        ObjectPayload::Zip { iterators } => CborValue::Array(
            iterators.iter().map(|id| CborValue::Uint(*id as u64)).collect()
        ),
        ObjectPayload::Map { function, iterator } => CborValue::Map(vec![
            (CborValue::Text("function".to_owned()), CborValue::Uint(*function as u64)),
            (CborValue::Text("iterator".to_owned()), CborValue::Uint(*iterator as u64)),
        ]),
        ObjectPayload::Filter { function, iterator } => CborValue::Map(vec![
            (CborValue::Text("function".to_owned()), CborValue::Uint(*function as u64)),
            (CborValue::Text("iterator".to_owned()), CborValue::Uint(*iterator as u64)),
        ]),
        ObjectPayload::ListIterator { list, position } => CborValue::Map(vec![
            (CborValue::Text("list".to_owned()), CborValue::Uint(*list as u64)),
            (CborValue::Text("position".to_owned()), CborValue::Uint(*position as u64)),
        ]),
        ObjectPayload::RangeIterator { range, position } => CborValue::Map(vec![
            (CborValue::Text("range".to_owned()), CborValue::Uint(*range as u64)),
            (CborValue::Text("position".to_owned()), CborValue::Uint(*position as u64)),
        ]),
        ObjectPayload::Range { start, stop, step } => CborValue::Map(vec![
            (CborValue::Text("start".to_owned()), CborValue::Text(start.to_string())),
            (CborValue::Text("stop".to_owned()), CborValue::Text(stop.to_string())),
            (CborValue::Text("step".to_owned()), CborValue::Text(step.to_string())),
        ]),
    };
    CborValue::Array(vec![CborValue::Uint(entry.tag as u64), payload])
}

fn decode_object_entry(value: CborValue) -> Result<ObjectEntry, SnapshotError> {
    let arr = expect_array(value)?;
    if arr.len() != 2 {
        return Err(SnapshotError::msg("invalid object entry"));
    }
    let tag = expect_uint(arr[0].clone())? as u8;
    let tag = match tag {
        0 => ObjTag::None,
        1 => ObjTag::Bool,
        2 => ObjTag::Int,
        3 => ObjTag::Float,
        4 => ObjTag::Str,
        5 => ObjTag::Bytes,
        6 => ObjTag::List,
        7 => ObjTag::Tuple,
        8 => ObjTag::Dict,
        9 => ObjTag::Set,
        10 => ObjTag::FrozenSet,
        11 => ObjTag::Module,
        12 => ObjTag::Function,
        13 => ObjTag::Code,
        14 => ObjTag::Type,
        15 => ObjTag::BuiltinType,
        16 => ObjTag::Instance,
        17 => ObjTag::Cell,
        18 => ObjTag::BuiltinModule,
        19 => ObjTag::BuiltinDict,
        20 => ObjTag::BuiltinFunction,
        21 => ObjTag::Enumerate,
        22 => ObjTag::Zip,
        23 => ObjTag::Map,
        24 => ObjTag::Filter,
        25 => ObjTag::ListIterator,
        26 => ObjTag::RangeIterator,
        27 => ObjTag::Range,
        _ => return Err(SnapshotError::msg("unknown tag")),
    };
    let payload = decode_payload(tag, arr[1].clone())?;
    Ok(ObjectEntry { tag, payload })
}

fn decode_payload(tag: ObjTag, value: CborValue) -> Result<ObjectPayload, SnapshotError> {
    match tag {
        ObjTag::None => Ok(ObjectPayload::None),
        ObjTag::Bool => Ok(ObjectPayload::Bool(expect_bool(value)?)),
        ObjTag::Int => Ok(ObjectPayload::Int(expect_text(value)?)),
        ObjTag::Float => Ok(ObjectPayload::Float(expect_float(value)?)),
        ObjTag::Str => Ok(ObjectPayload::Str(expect_text(value)?)),
        ObjTag::Bytes => Ok(ObjectPayload::Bytes(expect_bytes(value)?)),
        ObjTag::List => Ok(ObjectPayload::List(expect_id_list(value)?)),
        ObjTag::Tuple => Ok(ObjectPayload::Tuple(expect_id_list(value)?)),
        ObjTag::Dict => {
            let arr = expect_array(value)?;
            let mut items = Vec::new();
            for item in arr {
                let pair = expect_array(item)?;
                if pair.len() != 2 {
                    return Err(SnapshotError::msg("dict entry invalid"));
                }
                items.push((expect_uint(pair[0].clone())? as ObjId, expect_uint(pair[1].clone())? as ObjId));
            }
            Ok(ObjectPayload::Dict(items))
        }
        ObjTag::Set => Ok(ObjectPayload::Set(expect_id_list(value)?)),
        ObjTag::FrozenSet => Ok(ObjectPayload::FrozenSet(expect_id_list(value)?)),
        ObjTag::Module => {
            let map = expect_map(value)?;
            let name = expect_text(map_get(&map, "name")?)?;
            let dict = expect_uint(map_get(&map, "dict")?)? as ObjId;
            Ok(ObjectPayload::Module { name, dict })
        }
        ObjTag::BuiltinModule => {
            let map = expect_map(value)?;
            let name = expect_text(map_get(&map, "name")?)?;
            Ok(ObjectPayload::BuiltinModule { name })
        }
        ObjTag::BuiltinDict => {
            let map = expect_map(value)?;
            let name = expect_text(map_get(&map, "name")?)?;
            Ok(ObjectPayload::BuiltinDict { name })
        }
        ObjTag::Function => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Function(FunctionPayload {
                code: expect_uint(map_get(&map, "code")?)? as ObjId,
                globals: expect_uint(map_get(&map, "globals")?)? as ObjId,
                defaults: opt_id_decode(&map_get(&map, "defaults")?),
                kwdefaults: opt_id_decode(&map_get(&map, "kwdefaults")?),
                closure: opt_id_decode(&map_get(&map, "closure")?),
                name: expect_uint(map_get(&map, "name")?)? as ObjId,
                qualname: expect_uint(map_get(&map, "qualname")?)? as ObjId,
                annotations: expect_uint(map_get(&map, "annotations")?)? as ObjId,
                module: expect_uint(map_get(&map, "module")?)? as ObjId,
                doc: expect_uint(map_get(&map, "doc")?)? as ObjId,
                type_params: expect_uint(map_get(&map, "type_params")?)? as ObjId,
            }))
        }
        ObjTag::BuiltinFunction => {
            let map = expect_map(value)?;
            let name = expect_text(map_get(&map, "name")?)?;
            let module = match map_get(&map, "module")? {
                CborValue::Null => None,
                CborValue::Text(text) => Some(text.clone()),
                _ => return Err(SnapshotError::msg("builtin function module invalid")),
            };
            let self_obj = opt_id_decode(&map_get(&map, "self")?);
            Ok(ObjectPayload::BuiltinFunction(BuiltinFunctionPayload {
                name,
                module,
                self_obj,
            }))
        }
        ObjTag::Code => Ok(ObjectPayload::Code(expect_bytes(value)?)),
        ObjTag::Type => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Type(TypePayload {
                name: expect_text(map_get(&map, "name")?)?,
                qualname: expect_text(map_get(&map, "qualname")?)?,
                bases: expect_id_list(map_get(&map, "bases")?)?,
                dict: expect_uint(map_get(&map, "dict")?)? as ObjId,
                flags: expect_uint(map_get(&map, "flags")?)?,
                basicsize: expect_uint(map_get(&map, "basicsize")?)? as usize,
                itemsize: expect_uint(map_get(&map, "itemsize")?)? as usize,
                member_count: expect_uint(map_get(&map, "member_count")?)? as usize,
            }))
        }
        ObjTag::BuiltinType => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::BuiltinType {
                module: expect_text(map_get(&map, "module")?)?,
                name: expect_text(map_get(&map, "name")?)?,
            })
        }
        ObjTag::Instance => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Instance(InstancePayload {
                typ: expect_uint(map_get(&map, "type")?)? as ObjId,
                state: opt_id_decode(&map_get(&map, "state")?),
                new_args: opt_id_decode(&map_get(&map, "new_args")?),
                new_kwargs: opt_id_decode(&map_get(&map, "new_kwargs")?),
            }))
        }
        ObjTag::Cell => Ok(ObjectPayload::Cell(opt_id_decode(&value))),
        ObjTag::Enumerate => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Enumerate {
                iterator: expect_uint(map_get(&map, "iterator")?)? as ObjId,
                count: expect_int(map_get(&map, "count")?)?,
            })
        }
        ObjTag::Zip => {
            let arr = expect_array(value)?;
            let iterators = arr.iter()
                .map(|v| expect_uint(v.clone()).map(|id| id as ObjId))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ObjectPayload::Zip { iterators })
        }
        ObjTag::Map => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Map {
                function: expect_uint(map_get(&map, "function")?)? as ObjId,
                iterator: expect_uint(map_get(&map, "iterator")?)? as ObjId,
            })
        }
        ObjTag::Filter => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::Filter {
                function: expect_uint(map_get(&map, "function")?)? as ObjId,
                iterator: expect_uint(map_get(&map, "iterator")?)? as ObjId,
            })
        }
        ObjTag::ListIterator => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::ListIterator {
                list: expect_uint(map_get(&map, "list")?)? as ObjId,
                position: expect_uint(map_get(&map, "position")?)? as usize,
            })
        }
        ObjTag::RangeIterator => {
            let map = expect_map(value)?;
            Ok(ObjectPayload::RangeIterator {
                range: expect_uint(map_get(&map, "range")?)? as ObjId,
                position: expect_uint(map_get(&map, "position")?)? as usize,
            })
        }
        ObjTag::Range => {
            let map = expect_map(value)?;
            let start_str = expect_text(map_get(&map, "start")?)?;
            let stop_str = expect_text(map_get(&map, "stop")?)?;
            let step_str = expect_text(map_get(&map, "step")?)?;
            Ok(ObjectPayload::Range {
                start: start_str.parse::<i64>().unwrap_or(0),
                stop: stop_str.parse::<i64>().unwrap_or(0),
                step: step_str.parse::<i64>().unwrap_or(1),
            })
        }
    }
}

fn opt_id(value: Option<ObjId>) -> CborValue {
    match value {
        Some(id) => CborValue::Uint(id as u64),
        None => CborValue::Null,
    }
}

fn opt_id_decode(value: &CborValue) -> Option<ObjId> {
    match value {
        CborValue::Null => None,
        CborValue::Uint(id) => Some(*id as ObjId),
        _ => None,
    }
}

fn write_cbor_map(writer: &mut CborWriter, fields: Vec<(&str, CborValue)>) {
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(fields.len());
    for (key, value) in fields {
        let mut key_writer = CborWriter::new();
        key_writer.write_text(key);
        let key_bytes = key_writer.into_bytes();
        let value_bytes = encode_cbor_value(value);
        entries.push((key_bytes, value_bytes));
    }
    entries.sort_by(|(a, _), (b, _)| cbor_key_cmp(a, b));
    writer.write_map_len(entries.len());
    for (key, value) in entries {
        writer.buf.extend_from_slice(&key);
        writer.buf.extend_from_slice(&value);
    }
}

fn encode_cbor_value(value: CborValue) -> Vec<u8> {
    let mut writer = CborWriter::new();
    write_cbor_value(&mut writer, value);
    writer.into_bytes()
}

fn write_cbor_value(writer: &mut CborWriter, value: CborValue) {
    match value {
        CborValue::Uint(value) => writer.write_uint(value),
        CborValue::Nint(value) => write_uint_major(&mut writer.buf, 1, value),
        CborValue::Bytes(value) => writer.write_bytes(&value),
        CborValue::Text(value) => writer.write_text(&value),
        CborValue::Array(items) => {
            writer.write_array_len(items.len());
            for item in items {
                write_cbor_value(writer, item);
            }
        }
        CborValue::Map(items) => {
            let mut fields = Vec::with_capacity(items.len());
            for (k, v) in items {
                let mut key_writer = CborWriter::new();
                write_cbor_value(&mut key_writer, k);
                let key_bytes = key_writer.into_bytes();
                let value_bytes = encode_cbor_value(v);
                fields.push((key_bytes, value_bytes));
            }
            fields.sort_by(|(a, _), (b, _)| cbor_key_cmp(a, b));
            writer.write_map_len(fields.len());
            for (key, value) in fields {
                writer.buf.extend_from_slice(&key);
                writer.buf.extend_from_slice(&value);
            }
        }
        CborValue::Bool(value) => writer.write_bool(value),
        CborValue::Null => writer.write_null(),
        CborValue::Float(value) => writer.write_f64(value),
    }
}

fn expect_uint(value: CborValue) -> Result<u64, SnapshotError> {
    match value {
        CborValue::Uint(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected uint")),
    }
}

fn expect_int(value: CborValue) -> Result<i64, SnapshotError> {
    match value {
        CborValue::Uint(v) => Ok(v as i64),
        CborValue::Nint(v) => Ok(-(v as i64) - 1),
        _ => Err(SnapshotError::msg("expected int")),
    }
}

fn expect_text(value: CborValue) -> Result<String, SnapshotError> {
    match value {
        CborValue::Text(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected text")),
    }
}

fn expect_bytes(value: CborValue) -> Result<Vec<u8>, SnapshotError> {
    match value {
        CborValue::Bytes(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected bytes")),
    }
}

fn expect_array(value: CborValue) -> Result<Vec<CborValue>, SnapshotError> {
    match value {
        CborValue::Array(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected array")),
    }
}

fn expect_map(value: CborValue) -> Result<Vec<(CborValue, CborValue)>, SnapshotError> {
    match value {
        CborValue::Map(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected map")),
    }
}

fn expect_bool(value: CborValue) -> Result<bool, SnapshotError> {
    match value {
        CborValue::Bool(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected bool")),
    }
}

fn expect_float(value: CborValue) -> Result<f64, SnapshotError> {
    match value {
        CborValue::Float(v) => Ok(v),
        _ => Err(SnapshotError::msg("expected float")),
    }
}

fn expect_id_list(value: CborValue) -> Result<Vec<ObjId>, SnapshotError> {
    let arr = expect_array(value)?;
    arr.into_iter()
        .map(|v| Ok(expect_uint(v)? as ObjId))
        .collect()
}

fn map_get(map: &[(CborValue, CborValue)], key: &str) -> Result<CborValue, SnapshotError> {
    for (k, v) in map {
        if let CborValue::Text(text) = k {
            if text == key {
                return Ok(v.clone());
            }
        }
    }
    Err(SnapshotError::msg("missing map key"))
}

/// Encode a BlockState to CBOR
fn encode_block_state(block_state: &BlockState) -> CborValue {
    let mut map = vec![
        (CborValue::Text("level".to_owned()), CborValue::Uint(block_state.level as u64)),
    ];
    
    let typ_value = match &block_state.typ {
        BlockTypeState::Loop => {
            CborValue::Text("Loop".to_owned())
        }
        BlockTypeState::TryExcept { handler } => {
            CborValue::Map(vec![
                (CborValue::Text("type".to_owned()), CborValue::Text("TryExcept".to_owned())),
                (CborValue::Text("handler".to_owned()), CborValue::Uint(*handler as u64)),
            ])
        }
        BlockTypeState::Finally { handler } => {
            CborValue::Map(vec![
                (CborValue::Text("type".to_owned()), CborValue::Text("Finally".to_owned())),
                (CborValue::Text("handler".to_owned()), CborValue::Uint(*handler as u64)),
            ])
        }
        BlockTypeState::FinallyHandler { reason, prev_exc } => {
            let mut inner_map = vec![
                (CborValue::Text("type".to_owned()), CborValue::Text("FinallyHandler".to_owned())),
            ];
            if let Some(reason_state) = reason {
                inner_map.push((CborValue::Text("reason".to_owned()), encode_unwind_reason(reason_state)));
            }
            if let Some(exc_id) = prev_exc {
                inner_map.push((CborValue::Text("prev_exc".to_owned()), CborValue::Uint(*exc_id as u64)));
            }
            CborValue::Map(inner_map)
        }
        BlockTypeState::ExceptHandler { prev_exc } => {
            let mut inner_map = vec![
                (CborValue::Text("type".to_owned()), CborValue::Text("ExceptHandler".to_owned())),
            ];
            if let Some(exc_id) = prev_exc {
                inner_map.push((CborValue::Text("prev_exc".to_owned()), CborValue::Uint(*exc_id as u64)));
            }
            CborValue::Map(inner_map)
        }
    };
    
    map.push((CborValue::Text("typ".to_owned()), typ_value));
    CborValue::Map(map)
}

/// Encode an UnwindReasonState to CBOR
fn encode_unwind_reason(reason: &UnwindReasonState) -> CborValue {
    match reason {
        UnwindReasonState::Returning { value } => {
            CborValue::Map(vec![
                (CborValue::Text("kind".to_owned()), CborValue::Text("Returning".to_owned())),
                (CborValue::Text("value".to_owned()), CborValue::Uint(*value as u64)),
            ])
        }
        UnwindReasonState::Raising { exception } => {
            CborValue::Map(vec![
                (CborValue::Text("kind".to_owned()), CborValue::Text("Raising".to_owned())),
                (CborValue::Text("exception".to_owned()), CborValue::Uint(*exception as u64)),
            ])
        }
        UnwindReasonState::Break { target } => {
            CborValue::Map(vec![
                (CborValue::Text("kind".to_owned()), CborValue::Text("Break".to_owned())),
                (CborValue::Text("target".to_owned()), CborValue::Uint(*target as u64)),
            ])
        }
        UnwindReasonState::Continue { target } => {
            CborValue::Map(vec![
                (CborValue::Text("kind".to_owned()), CborValue::Text("Continue".to_owned())),
                (CborValue::Text("target".to_owned()), CborValue::Uint(*target as u64)),
            ])
        }
    }
}

/// Decode a BlockState from CBOR
fn decode_block_state(value: CborValue) -> Result<BlockState, SnapshotError> {
    let map = expect_map(value)?;
    let level = expect_uint(map_get(&map, "level")?)? as usize;
    let typ_val = map_get(&map, "typ")?;
    
    let typ = match typ_val {
        CborValue::Text(text) => {
            if text == "Loop" {
                BlockTypeState::Loop
            } else {
                return Err(SnapshotError::msg(format!("unknown block type: {}", text)));
            }
        }
        CborValue::Map(inner_map) => {
            let type_name = expect_text(map_get(&inner_map, "type")?)?;
            match type_name.as_str() {
                "TryExcept" => {
                    let handler = expect_uint(map_get(&inner_map, "handler")?)? as u32;
                    BlockTypeState::TryExcept { handler }
                }
                "Finally" => {
                    let handler = expect_uint(map_get(&inner_map, "handler")?)? as u32;
                    BlockTypeState::Finally { handler }
                }
                "FinallyHandler" => {
                    let reason = if let Ok(reason_val) = map_get(&inner_map, "reason") {
                        Some(decode_unwind_reason(reason_val)?)
                    } else {
                        None
                    };
                    let prev_exc = if let Ok(exc_val) = map_get(&inner_map, "prev_exc") {
                        Some(expect_uint(exc_val)? as ObjId)
                    } else {
                        None
                    };
                    BlockTypeState::FinallyHandler { reason, prev_exc }
                }
                "ExceptHandler" => {
                    let prev_exc = if let Ok(exc_val) = map_get(&inner_map, "prev_exc") {
                        Some(expect_uint(exc_val)? as ObjId)
                    } else {
                        None
                    };
                    BlockTypeState::ExceptHandler { prev_exc }
                }
                _ => {
                    return Err(SnapshotError::msg(format!("unknown block type: {}", type_name)));
                }
            }
        }
        _ => {
            return Err(SnapshotError::msg("invalid block type format"));
        }
    };
    
    Ok(BlockState { typ, level })
}

/// Decode an UnwindReasonState from CBOR
fn decode_unwind_reason(value: CborValue) -> Result<UnwindReasonState, SnapshotError> {
    let map = expect_map(value)?;
    let kind = expect_text(map_get(&map, "kind")?)?;
    
    match kind.as_str() {
        "Returning" => {
            let value_id = expect_uint(map_get(&map, "value")?)? as ObjId;
            Ok(UnwindReasonState::Returning { value: value_id })
        }
        "Raising" => {
            let exc_id = expect_uint(map_get(&map, "exception")?)? as ObjId;
            Ok(UnwindReasonState::Raising { exception: exc_id })
        }
        "Break" => {
            let target = expect_uint(map_get(&map, "target")?)? as u32;
            Ok(UnwindReasonState::Break { target })
        }
        "Continue" => {
            let target = expect_uint(map_get(&map, "target")?)? as u32;
            Ok(UnwindReasonState::Continue { target })
        }
        _ => {
            Err(SnapshotError::msg(format!("unknown unwind reason: {}", kind)))
        }
    }
}

// ============================================================================
// Block Stack Conversion Functions
// ============================================================================

/// Convert frame Block to serializable BlockState
fn convert_block_to_state(
    block: &crate::frame::Block,
    writer: &SnapshotWriter<'_>,
) -> PyResult<BlockState> {
    use crate::frame::{BlockType, UnwindReason};
    
    let typ_state = match &block.typ {
        BlockType::Loop => BlockTypeState::Loop,
        
        BlockType::TryExcept { handler } => {
            BlockTypeState::TryExcept {
                handler: handler.0,
            }
        }
        
        BlockType::Finally { handler } => {
            BlockTypeState::Finally {
                handler: handler.0,
            }
        }
        
        BlockType::FinallyHandler { reason, prev_exc } => {
            let reason_state = reason.as_ref()
                .map(|r| {
                    let value_id = match r {
                        UnwindReason::Returning { value } => {
                            writer.get_id(value).map(|id| UnwindReasonState::Returning { value: id })
                        }
                        UnwindReason::Raising { exception } => {
                            let exc_obj = exception.as_object().to_owned();
                            writer.get_id(&exc_obj).map(|id| UnwindReasonState::Raising { exception: id })
                        }
                        UnwindReason::Break { target } => {
                            Ok(UnwindReasonState::Break { target: target.0 })
                        }
                        UnwindReason::Continue { target } => {
                            Ok(UnwindReasonState::Continue { target: target.0 })
                        }
                    };
                    value_id.map_err(|e| writer.vm.new_value_error(format!("Failed to serialize unwind reason: {e:?}")))
                })
                .transpose()?;
            let prev_exc_id = prev_exc.as_ref()
                .map(|exc| {
                    let exc_obj = exc.as_object().to_owned();
                    writer.get_id(&exc_obj).map_err(|e| writer.vm.new_value_error(format!("Failed to get exception ID: {e:?}")))
                })
                .transpose()?;
            
            BlockTypeState::FinallyHandler {
                reason: reason_state,
                prev_exc: prev_exc_id,
            }
        }
        
        BlockType::ExceptHandler { prev_exc } => {
            let prev_exc_id = prev_exc.as_ref()
                .map(|exc| {
                    let exc_obj = exc.as_object().to_owned();
                    writer.get_id(&exc_obj).map_err(|e| writer.vm.new_value_error(format!("Failed to get exception ID: {e:?}")))
                })
                .transpose()?;
            
            BlockTypeState::ExceptHandler {
                prev_exc: prev_exc_id,
            }
        }
    };
    
    Ok(BlockState {
        typ: typ_state,
        level: block.level,
    })
}

/// Convert serializable BlockState to frame Block
pub(super) fn convert_block_state_to_block(
    block_state: &BlockState,
    objects: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult<crate::frame::Block> {
    use crate::frame::{Block, BlockType, UnwindReason};
    use crate::convert::TryFromObject;
    
    let typ = match &block_state.typ {
        BlockTypeState::Loop => BlockType::Loop,
        
        BlockTypeState::TryExcept { handler } => {
            BlockType::TryExcept {
                handler: bytecode::Label(*handler),
            }
        }
        
        BlockTypeState::Finally { handler } => {
            BlockType::Finally {
                handler: bytecode::Label(*handler),
            }
        }
        
        BlockTypeState::FinallyHandler { reason, prev_exc } => {
            let reason_opt = reason.as_ref()
                .map(|r| {
                    match r {
                        UnwindReasonState::Returning { value } => {
                            let value_obj = objects.get(*value as usize)
                                .cloned()
                                .ok_or_else(|| vm.new_runtime_error(format!("return value {} not found", value)))?;
                            Ok(UnwindReason::Returning { value: value_obj })
                        }
                        UnwindReasonState::Raising { exception } => {
                            let exc_obj = objects.get(*exception as usize)
                                .cloned()
                                .ok_or_else(|| vm.new_runtime_error(format!("exception {} not found", exception)))?;
                            let exc = crate::builtins::PyBaseExceptionRef::try_from_object(vm, exc_obj)?;
                            Ok(UnwindReason::Raising { exception: exc })
                        }
                        UnwindReasonState::Break { target } => {
                            Ok(UnwindReason::Break { target: bytecode::Label(*target) })
                        }
                        UnwindReasonState::Continue { target } => {
                            Ok(UnwindReason::Continue { target: bytecode::Label(*target) })
                        }
                    }
                })
                .transpose()?;
            let prev_exc_opt = prev_exc.as_ref()
                .map(|exc_id| {
                    let exc_obj = objects.get(*exc_id as usize)
                        .cloned()
                        .ok_or_else(|| vm.new_runtime_error(format!("exception {} not found", exc_id)))?;
                    crate::builtins::PyBaseExceptionRef::try_from_object(vm, exc_obj)
                })
                .transpose()?;
            
            BlockType::FinallyHandler {
                reason: reason_opt,
                prev_exc: prev_exc_opt,
            }
        }
        
        BlockTypeState::ExceptHandler { prev_exc } => {
            let prev_exc_opt = prev_exc.as_ref()
                .map(|exc_id| {
                    let exc_obj = objects.get(*exc_id as usize)
                        .cloned()
                        .ok_or_else(|| vm.new_runtime_error(format!("exception {} not found", exc_id)))?;
                    crate::builtins::PyBaseExceptionRef::try_from_object(vm, exc_obj)
                })
                .transpose()?;
            
            BlockType::ExceptHandler {
                prev_exc: prev_exc_opt,
            }
        }
    };
    
    Ok(Block {
        typ,
        level: block_state.level,
    })
}
