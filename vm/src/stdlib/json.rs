use std::fmt;

use serde;
use serde::de::{DeserializeSeed, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde_json;

use super::super::obj::{objbool, objdict, objfloat, objint, objlist, objstr, objtuple, objtype};
use super::super::pyobject::{
    DictProtocol, PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::VirtualMachine;

// We need to have a VM available to serialise a PyObject based on its subclass, so we implement
// PyObject serialisation via a proxy object which holds a reference to a VM
struct PyObjectSerializer<'s> {
    pyobject: &'s PyObjectRef,
    ctx: &'s PyContext,
}

impl<'s> PyObjectSerializer<'s> {
    fn clone_with_object(&self, pyobject: &'s PyObjectRef) -> PyObjectSerializer {
        PyObjectSerializer {
            pyobject,
            ctx: self.ctx,
        }
    }
}

impl<'s> serde::Serialize for PyObjectSerializer<'s> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let serialize_seq_elements =
            |serializer: S, elements: Vec<PyObjectRef>| -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(elements.len()))?;
                for e in elements {
                    seq.serialize_element(&self.clone_with_object(&e))?;
                }
                seq.end()
            };
        if objtype::isinstance(self.pyobject, self.ctx.str_type()) {
            serializer.serialize_str(&objstr::get_value(&self.pyobject))
        } else if objtype::isinstance(self.pyobject, self.ctx.float_type()) {
            serializer.serialize_f64(objfloat::get_value(self.pyobject))
        } else if objtype::isinstance(self.pyobject, self.ctx.bool_type()) {
            serializer.serialize_bool(objbool::get_value(self.pyobject))
        } else if objtype::isinstance(self.pyobject, self.ctx.int_type()) {
            serializer.serialize_i32(objint::get_value(self.pyobject))
        } else if objtype::isinstance(self.pyobject, self.ctx.list_type()) {
            let elements = objlist::get_elements(self.pyobject);
            serialize_seq_elements(serializer, elements)
        } else if objtype::isinstance(self.pyobject, self.ctx.tuple_type()) {
            let elements = objtuple::get_elements(self.pyobject);
            serialize_seq_elements(serializer, elements)
        } else if objtype::isinstance(self.pyobject, self.ctx.dict_type()) {
            let elements = objdict::get_elements(self.pyobject);
            let mut map = serializer.serialize_map(Some(elements.len()))?;
            for (key, e) in elements {
                map.serialize_entry(&key, &self.clone_with_object(&e))?;
            }
            map.end()
        } else if let PyObjectKind::None = self.pyobject.borrow().kind {
            serializer.serialize_none()
        } else {
            unimplemented!(
                "Object of type '{:?}' is not serializable",
                self.pyobject.typ()
            );
        }
    }
}

// This object is used as the seed for deserialization so we have access to the PyContext for type
// creation
#[derive(Clone)]
struct PyObjectDeserializer<'c> {
    ctx: &'c PyContext,
}

impl<'de> serde::de::DeserializeSeed<'de> for PyObjectDeserializer<'de> {
    type Value = PyObjectRef;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        impl<'de> Visitor<'de> for PyObjectDeserializer<'de> {
            type Value = PyObjectRef;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a type that can deserialise in Python")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(self.ctx.new_str(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(self.ctx.new_str(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // The JSON deserialiser always uses the i64/u64 deserialisers, so we only need to
                // implement those for now
                use std::i32;
                if value >= i32::MIN as i64 && value <= i32::MAX as i64 {
                    Ok(self.ctx.new_int(value as i32))
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
                    Ok(self.ctx.new_int(value as i32))
                } else {
                    Err(E::custom(format!("u64 out of range: {}", value)))
                }
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(self.ctx.new_float(value))
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(self.ctx.new_bool(value))
            }

            fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut seq = Vec::with_capacity(access.size_hint().unwrap_or(0));
                while let Some(value) = access.next_element_seed(self.clone())? {
                    seq.push(value);
                }
                Ok(self.ctx.new_list(seq))
            }

            fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
            where
                M: serde::de::MapAccess<'de>,
            {
                let dict = self.ctx.new_dict();
                // TODO: Given keys must be strings, we can probably do something more efficient
                // than wrapping the given object up and then unwrapping it to determine whether or
                // not it is a string
                while let Some((key_obj, value)) =
                    access.next_entry_seed(self.clone(), self.clone())?
                {
                    let key = match key_obj.borrow().kind {
                        PyObjectKind::String { ref value } => value.clone(),
                        _ => unimplemented!("map keys must be strings"),
                    };
                    dict.set_item(&key, value);
                }
                Ok(dict)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(self.ctx.none.clone())
            }
        }

        deserializer.deserialize_any(self.clone())
    }
}

fn dumps(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: Implement non-trivial serialisation case
    arg_check!(vm, args, required = [(obj, None)]);
    // TODO: Raise an exception for serialisation errors
    let serializer = PyObjectSerializer {
        pyobject: obj,
        ctx: &vm.ctx,
    };
    let string = serde_json::to_string(&serializer).unwrap();
    Ok(vm.context().new_str(string))
}

fn loads(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: Implement non-trivial deserialisation case
    arg_check!(vm, args, required = [(string, Some(vm.ctx.str_type()))]);
    // TODO: Raise an exception for deserialisation errors
    let de = PyObjectDeserializer { ctx: &vm.ctx };
    // TODO: Support deserializing string sub-classes
    Ok(de
        .deserialize(&mut serde_json::Deserializer::from_str(&objstr::get_value(
            &string,
        ))).unwrap())
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let json_mod = ctx.new_module(&"json".to_string(), ctx.new_scope(None));
    json_mod.set_item("dumps", ctx.new_rustfunc(dumps));
    json_mod.set_item("loads", ctx.new_rustfunc(loads));
    json_mod
}
