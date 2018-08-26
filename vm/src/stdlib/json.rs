use std::collections::HashMap;
use std::fmt;

use serde;
use serde::de::Visitor;
use serde::ser::{SerializeMap, SerializeSeq};
use serde_json;

use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObject, PyObjectKind, PyObjectRef, PyResult,
};
use super::super::VirtualMachine;

impl serde::Serialize for PyObjectKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            PyObjectKind::String { value } => serializer.serialize_str(value),
            PyObjectKind::Integer { value } => serializer.serialize_i32(*value),
            PyObjectKind::Float { value } => serializer.serialize_f64(*value),
            PyObjectKind::Boolean { value } => serializer.serialize_bool(*value),
            PyObjectKind::List { elements } | PyObjectKind::Tuple { elements } => {
                let mut seq = serializer.serialize_seq(Some(elements.len()))?;
                for e in elements {
                    match e.borrow().kind {
                        ref kind => seq.serialize_element(kind)?,
                    }
                }
                seq.end()
            }
            PyObjectKind::Dict { elements } => {
                let mut map = serializer.serialize_map(Some(elements.len()))?;
                for (key, e) in elements {
                    map.serialize_entry(key, &e.borrow().kind)?;
                }
                map.end()
            }
            PyObjectKind::None => serializer.serialize_none(),
            kind => unimplemented!("Object of type '{:?}' is not serializable", kind),
        }
    }
}

struct PyObjectKindVisitor;

impl<'de> Visitor<'de> for PyObjectKindVisitor {
    type Value = PyObjectKind;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a type that can deserialise in Python")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PyObjectKind::String {
            value: value.to_string(),
        })
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PyObjectKind::String { value })
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // The JSON deserialiser always uses the i64/u64 deserialisers, so we only need to
        // implement those for now
        use std::i32;
        if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
            Ok(PyObjectKind::Integer {
                value: value as i32,
            })
        } else {
            Err(E::custom(format!("i64 out of range: {}", value)))
        }
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // The JSON deserialiser always uses the i64/u64 deserialisers, so we only need to
        // implement those for now
        use std::i32;
        if value <= i32::MAX as u64 {
            Ok(PyObjectKind::Integer {
                value: value as i32,
            })
        } else {
            Err(E::custom(format!("u64 out of range: {}", value)))
        }
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PyObjectKind::Float { value })
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PyObjectKind::Boolean { value })
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut seq = Vec::with_capacity(access.size_hint().unwrap_or(0));
        while let Some(value) = access.next_element()? {
            seq.push(
                PyObject {
                    kind: value,
                    typ: None, // TODO: Determine the effect this None will have
                }.into_ref(),
            );
        }
        Ok(PyObjectKind::List { elements: seq })
    }

    fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        let mut map = HashMap::with_capacity(access.size_hint().unwrap_or(0));

        while let Some((key, value)) = access.next_entry()? {
            map.insert(
                key,
                PyObject {
                    kind: value,
                    typ: None, // TODO: Determine the effect this None will have
                }.into_ref(),
            );
        }

        Ok(PyObjectKind::Dict { elements: map })
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PyObjectKind::None)
    }
}

impl<'de> serde::Deserialize<'de> for PyObjectKind {
    fn deserialize<D>(deserializer: D) -> Result<PyObjectKind, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(PyObjectKindVisitor)
    }
}

fn dumps(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        // TODO: Raise an exception for wrong number of args
        // TODO: Implement non-trivial serialisation case
        unimplemented!("json.dumps only supports the trivial 1-arg case");
    };
    // TODO: Raise an exception for serialisation errors
    let string = serde_json::to_string(&args.args[0].borrow().kind).unwrap();
    Ok(vm.context().new_str(string))
}

fn loads(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 1 {
        // TODO: Raise an exception for wrong number of args
        // TODO: Implement non-trivial serialisation case
        unimplemented!("json.loads only supports the trivial 1-arg case");
    };
    // TODO: Raise an exception for deserialisation errors
    let kind: PyObjectKind = match args.args[0].borrow().kind {
        PyObjectKind::String { ref value } => serde_json::from_str(&value).unwrap(),
        _ => unimplemented!("json.loads only handles strings"),
    };
    Ok(PyObject::new(kind, vm.get_type()))
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let json_mod = ctx.new_module(&"json".to_string(), ctx.new_scope(None));
    json_mod.set_item("dumps", ctx.new_rustfunc(dumps));
    json_mod.set_item("loads", ctx.new_rustfunc(loads));
    json_mod
}
