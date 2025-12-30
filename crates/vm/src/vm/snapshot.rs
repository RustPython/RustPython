use crate::{
    AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    builtins::{
        PyDictRef, PyFloat, PyInt, PyList, PyModule, PyStr, PyTuple,
        code::{PyCode, CodeObject, PyObjBag},
        dict::PyDict,
        function::{PyCell, PyFunction},
        set::{PyFrozenSet, PySet},
        type_::PyType,
    },
    convert::TryFromObject,
};
use rustpython_compiler_core::marshal;
use std::collections::HashMap;

pub(crate) type ObjId = u32;

const SNAPSHOT_VERSION: u32 = 3;

#[derive(Debug)]
pub(crate) struct CheckpointState {
    pub version: u32,
    pub source_path: String,
    pub lasti: u32,
    pub code: Vec<u8>,
    pub root: ObjId,
    pub objects: Vec<ObjectEntry>,
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
    let state = CheckpointState {
        version: SNAPSHOT_VERSION,
        source_path: source_path.to_owned(),
        lasti,
        code: code_bytes,
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
    let reader = SnapshotReader::new(vm, &state.objects);
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
}

impl<'a> SnapshotWriter<'a> {
    fn new(vm: &'a VirtualMachine) -> Self {
        Self {
            vm,
            ids: HashMap::new(),
            objects: Vec::new(),
        }
    }

    fn serialize_obj(&mut self, obj: &PyObjectRef) -> Result<ObjId, SnapshotError> {
        let ptr = obj.as_object().as_raw() as usize;
        if let Some(id) = self.ids.get(&ptr) {
            return Ok(*id);
        }

        let tag = classify_obj(self.vm, obj)?;
        let id = self.objects.len() as ObjId;
        self.ids.insert(ptr, id);
        let payload = self.build_payload(tag, obj)?;
        self.objects.push(ObjectEntry { tag, payload });
        Ok(id)
    }

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
                    .map(|item| self.serialize_obj(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ObjectPayload::List(items))
            }
            ObjTag::Tuple => {
                let tuple = obj.downcast_ref::<PyTuple>().ok_or_else(|| SnapshotError::msg("expected tuple"))?;
                let items = tuple
                    .iter()
                    .map(|item| self.serialize_obj(item))
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
                    let key_id = self.serialize_obj(&key)?;
                    let value_id = self.serialize_obj(&value)?;
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
                    let key_id = self.serialize_obj(&key)?;
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
                    let key_id = self.serialize_obj(&key)?;
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
                let dict_id = self.serialize_obj(&dict.into())?;
                Ok(ObjectPayload::Module { name, dict: dict_id })
            }
            ObjTag::Function => {
                obj.downcast_ref::<PyFunction>()
                    .ok_or_else(|| SnapshotError::msg("expected function"))?;
                let code_obj = get_attr(self.vm, obj, "__code__")?;
                let code = self.serialize_obj(&code_obj)?;
                let globals_obj = get_attr(self.vm, obj, "__globals__")?;
                let globals = self.serialize_obj(&globals_obj)?;
                let defaults_obj = get_attr(self.vm, obj, "__defaults__")?;
                let defaults = if self.vm.is_none(&defaults_obj) {
                    None
                } else {
                    Some(self.serialize_obj(&defaults_obj)?)
                };
                let kwdefaults_obj = get_attr(self.vm, obj, "__kwdefaults__")?;
                let kwdefaults = if self.vm.is_none(&kwdefaults_obj) {
                    None
                } else {
                    Some(self.serialize_obj(&kwdefaults_obj)?)
                };
                let closure_obj = get_attr(self.vm, obj, "__closure__")?;
                let closure = if self.vm.is_none(&closure_obj) {
                    None
                } else {
                    Some(self.serialize_obj(&closure_obj)?)
                };
                let name = self.serialize_obj(&get_attr(self.vm, obj, "__name__")?)?;
                let qualname = self.serialize_obj(&get_attr(self.vm, obj, "__qualname__")?)?;
                let annotations = self.serialize_obj(&get_attr(self.vm, obj, "__annotations__")?)?;
                let module = self.serialize_obj(&get_attr(self.vm, obj, "__module__")?)?;
                let doc = self.serialize_obj(&get_attr(self.vm, obj, "__doc__")?)?;
                let type_params_obj = get_attr_opt(self.vm, obj, "__type_params__")?
                    .unwrap_or_else(|| self.vm.ctx.empty_tuple.clone().into());
                let type_params = self.serialize_obj(&type_params_obj)?;
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
                let bases = typ
                    .bases
                    .read()
                    .iter()
                    .map(|base| self.serialize_obj(&base.to_owned().into()))
                    .collect::<Result<Vec<_>, _>>()?;
                let dict = self.vm.ctx.new_dict();
                for (key, value) in typ.attributes.read().iter() {
                    if should_skip_type_attr(self.vm, value) {
                        continue;
                    }
                    dict.set_item(key.as_str(), value.clone(), self.vm)
                        .map_err(|_| SnapshotError::msg("type dict build failed"))?;
                }
                let attrs_dict_id = self.serialize_obj(&dict.into())?;
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
                    dict: attrs_dict_id,
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
                let typ_id = self.serialize_obj(&typ.to_owned().into())?;
                let (new_args, new_kwargs) = get_newargs(self.vm, obj)?;
                let new_args_id = new_args.map(|o| self.serialize_obj(&o)).transpose()?;
                let new_kwargs_id = new_kwargs.map(|o| self.serialize_obj(&o)).transpose()?;
                let state = get_state(self.vm, obj)?;
                let state_id = state.map(|o| self.serialize_obj(&o)).transpose()?;
                Ok(ObjectPayload::Instance(InstancePayload {
                    typ: typ_id,
                    state: state_id,
                    new_args: new_args_id,
                    new_kwargs: new_kwargs_id,
                }))
            }
            ObjTag::Cell => {
                let cell = obj.downcast_ref::<PyCell>().ok_or_else(|| SnapshotError::msg("expected cell"))?;
                let contents = cell.get().map(|o| self.serialize_obj(&o)).transpose()?;
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
                    .map(|value| self.serialize_obj(&value))
                    .transpose()?;
                Ok(ObjectPayload::BuiltinFunction(BuiltinFunctionPayload {
                    name,
                    module,
                    self_obj,
                }))
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
    if let Some(getstate) = vm.get_attribute_opt(obj.clone(), "__getstate__").map_err(|_| SnapshotError::msg("getstate lookup failed"))? {
        let value = getstate
            .call((), vm)
            .map_err(|_| SnapshotError::msg("__getstate__ failed"))?;
        return Ok(Some(value));
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
    if let Some(getnewargs_ex) = vm
        .get_attribute_opt(obj.clone(), "__getnewargs_ex__")
        .map_err(|_| SnapshotError::msg("getnewargs_ex lookup failed"))?
    {
        let value = getnewargs_ex
            .call((), vm)
            .map_err(|_| SnapshotError::msg("__getnewargs_ex__ failed"))?;
        let tuple = value
            .downcast_ref::<PyTuple>()
            .ok_or_else(|| SnapshotError::msg("__getnewargs_ex__ must return (args, kwargs)"))?;
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
        return Ok((Some(value), None));
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
    objects: Vec<Option<PyObjectRef>>,
    filled: Vec<bool>,
}

impl<'a> SnapshotReader<'a> {
    fn new(vm: &'a VirtualMachine, entries: &'a [ObjectEntry]) -> Self {
        Self {
            vm,
            entries,
            objects: vec![None; entries.len()],
            filled: vec![false; entries.len()],
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
        if self.objects[idx].is_some() {
            return Ok(());
        }
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
                let globals = PyDictRef::try_from_object(self.vm, globals_obj)
                    .map_err(|_| SnapshotError::msg("function globals invalid"))?;
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
                    func
                        .set_function_attribute(crate::bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS, obj, self.vm)
                        .map_err(|_| SnapshotError::msg("kwdefaults invalid"))?;
                }
                if let Some(closure_id) = payload.closure {
                    let obj = self.get_obj(closure_id)?;
                    func
                        .set_function_attribute(crate::bytecode::MakeFunctionFlags::CLOSURE, obj, self.vm)
                        .map_err(|_| SnapshotError::msg("closure invalid"))?;
                }
                let annotations_obj = self.get_obj(payload.annotations)?;
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
                    let module = lookup_module(self.vm, module_name)?;
                    let attr = self.vm.ctx.intern_str(payload.name.as_str());
                    module
                        .get_attr(attr, self.vm)
                        .map_err(|_| SnapshotError::msg("builtin function lookup failed"))?
                }
            }
            ObjectPayload::Code(bytes) => {
                let code = deserialize_code_object(self.vm, bytes)?;
                let code_ref: crate::PyRef<PyCode> = self.vm.ctx.new_pyref(PyCode::new(code));
                code_ref.into()
            }
            ObjectPayload::Type(payload) => {
                let mut bases = payload
                    .bases
                    .iter()
                    .map(|id| {
                        let obj = self.get_obj(*id)?;
                        obj.downcast::<PyType>()
                            .map_err(|_| SnapshotError::msg("type base invalid"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if bases.is_empty() {
                    bases.push(self.vm.ctx.types.object_type.to_owned());
                }
                let attrs = build_type_attributes(self, payload.dict, idx as ObjId)?;
                let mut slots = crate::types::PyTypeSlots::heap_default();
                slots.flags = crate::types::PyTypeFlags::from_bits_truncate(payload.flags);
                slots.basicsize = payload.basicsize;
                slots.itemsize = payload.itemsize;
                slots.member_count = payload.member_count;
                let metatype = self.vm.ctx.types.type_type.to_owned();
                let typ = crate::builtins::type_::PyType::new_heap(
                    payload.name.as_str(),
                    bases,
                    attrs,
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
                apply_deferred_type_attrs(self, typ_obj.clone(), payload.dict, idx as ObjId)?;
                typ_obj
            }
            ObjectPayload::BuiltinType { module, name } => {
                let module_obj = if module == "builtins" {
                    self.vm.builtins.clone().into()
                } else {
                    self.vm
                        .sys_module
                        .get_attr("modules", self.vm)
                        .map_err(|_| SnapshotError::msg("sys.modules unavailable"))?
                        .get_item(module.as_str(), self.vm)
                        .map_err(|_| SnapshotError::msg("module not found"))?
                };
                let attr = self.vm.ctx.intern_str(name.as_str());
                let ty = module_obj
                    .get_attr(attr, self.vm)
                    .map_err(|_| SnapshotError::msg("builtin type not found"))?;
                ty
            }
            ObjectPayload::Instance(payload) => {
                let typ_obj = self.get_obj(payload.typ)?;
                let typ = typ_obj
                    .downcast::<PyType>()
                    .map_err(|_| SnapshotError::msg("instance type invalid"))?;
                let args_obj = payload
                    .new_args
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let kwargs_obj = payload
                    .new_kwargs
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let new_func = self
                    .vm
                    .get_attribute_opt(typ.clone().into(), "__new__")
                    .map_err(|_| SnapshotError::msg("__new__ lookup failed"))?
                    .ok_or_else(|| SnapshotError::msg("__new__ missing"))?;
                let args_obj = args_obj.unwrap_or_else(|| self.vm.ctx.empty_tuple.clone().into());
                let kwargs_obj = kwargs_obj.unwrap_or_else(|| self.vm.ctx.new_dict().into());
                let args = args_obj
                    .downcast_ref::<PyTuple>()
                    .ok_or_else(|| SnapshotError::msg("new args must be tuple"))?;
                let kwargs = PyDictRef::try_from_object(self.vm, kwargs_obj)
                    .map_err(|_| SnapshotError::msg("new kwargs must be dict"))?;
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(typ.clone().into());
                call_args.extend(args.iter().cloned());
                let kwargs = kwargs_from_dict(kwargs)?;
                let instance = new_func
                    .call(crate::function::FuncArgs::new(call_args, kwargs), self.vm)
                    .map_err(|_| SnapshotError::msg("__new__ failed"))?;
                instance
            }
            ObjectPayload::Cell(contents) => {
                let value = contents
                    .map(|id| self.get_obj(id))
                    .transpose()?;
                let cell = PyCell::new(value);
                let cell_ref: crate::PyRef<PyCell> = self.vm.ctx.new_pyref(cell);
                cell_ref.into()
            }
        };
        self.objects[idx] = Some(obj);
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
            let state_dict = PyDictRef::try_from_object(self.vm, state)
                .map_err(|_| SnapshotError::msg("state must be dict"))?;
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
}

fn lookup_module(vm: &VirtualMachine, name: &str) -> Result<PyObjectRef, SnapshotError> {
    if name == "builtins" {
        return Ok(vm.builtins.clone().into());
    }
    if name == "sys" {
        return Ok(vm.sys_module.clone().into());
    }
    vm.sys_module
        .get_attr("modules", vm)
        .map_err(|_| SnapshotError::msg("sys.modules unavailable"))?
        .get_item(name, vm)
        .map_err(|_| SnapshotError::msg("module not found"))
}

fn build_type_attributes(
    reader: &mut SnapshotReader<'_>,
    dict_id: ObjId,
    type_id: ObjId,
) -> Result<crate::builtins::type_::PyAttributes, SnapshotError> {
    let entry = reader
        .entries
        .get(dict_id as usize)
        .ok_or_else(|| SnapshotError::msg("type dict missing"))?;
    let ObjectPayload::Dict(items) = &entry.payload else {
        return Err(SnapshotError::msg("type dict payload invalid"));
    };
    let mut attrs = crate::builtins::type_::PyAttributes::default();
    for (key_id, val_id) in items {
        if *key_id == type_id || *val_id == type_id {
            continue;
        }
        let key_obj = reader.get_obj(*key_id)?;
        let key = key_obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("type dict key must be str"))?;
        let value = reader.get_obj(*val_id)?;
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
    let ObjectPayload::Dict(items) = &entry.payload else {
        return Ok(());
    };
    for (key_id, val_id) in items {
        if *key_id != type_id && *val_id != type_id {
            continue;
        }
        let key_obj = reader.get_obj(*key_id)?;
        let key = key_obj
            .downcast_ref::<PyStr>()
            .ok_or_else(|| SnapshotError::msg("type dict key must be str"))?;
        let value = reader.get_obj(*val_id)?;
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
    fields.push(("lasti", CborValue::Uint(state.lasti as u64)));
    fields.push(("code", CborValue::Bytes(state.code.clone())));
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
    Ok(CheckpointState {
        version: version.ok_or_else(|| SnapshotError::msg("missing version"))?,
        source_path: source_path.ok_or_else(|| SnapshotError::msg("missing source_path"))?,
        lasti: lasti.ok_or_else(|| SnapshotError::msg("missing lasti"))?,
        code: code.ok_or_else(|| SnapshotError::msg("missing code"))?,
        root: root.ok_or_else(|| SnapshotError::msg("missing root"))?,
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
