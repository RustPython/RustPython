use std::fmt;

use serde;
use serde::de::{DeserializeSeed, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde_json;

use super::super::obj::{objbool, objdict, objfloat, objint, objsequence, objstr, objtype};
use super::super::pyobject::{
    create_type, DictProtocol, PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef, PyResult,
    TypeProtocol,
};
use super::super::VirtualMachine;
use num_traits::cast::ToPrimitive;

// We need to have a VM available to serialise a PyObject based on its subclass, so we implement
// PyObject serialisation via a proxy object which holds a reference to a VM
struct PyObjectSerializer<'s> {
    pyobject: &'s PyObjectRef,
    vm: &'s VirtualMachine,
}

impl<'s> PyObjectSerializer<'s> {
    fn clone_with_object(&self, pyobject: &'s PyObjectRef) -> PyObjectSerializer {
        PyObjectSerializer {
            pyobject,
            vm: self.vm,
        }
    }
}

impl<'s> serde::Serialize for PyObjectSerializer<'s> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let serialize_seq_elements =
            |serializer: S, elements: &Vec<PyObjectRef>| -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(elements.len()))?;
                for e in elements.iter() {
                    seq.serialize_element(&self.clone_with_object(e))?;
                }
                seq.end()
            };
        if objtype::isinstance(self.pyobject, &self.vm.ctx.str_type()) {
            serializer.serialize_str(&objstr::get_value(&self.pyobject))
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.float_type()) {
            serializer.serialize_f64(objfloat::get_value(self.pyobject))
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.bool_type()) {
            serializer.serialize_bool(objbool::get_value(self.pyobject))
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.int_type()) {
            let v = objint::get_value(self.pyobject);
            serializer.serialize_i64(v.to_i64().unwrap())
        // Although this may seem nice, it does not give the right result:
        // v.serialize(serializer)
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.list_type())
            || objtype::isinstance(self.pyobject, &self.vm.ctx.tuple_type())
        {
            let elements = objsequence::get_elements(self.pyobject);
            serialize_seq_elements(serializer, &elements)
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.dict_type()) {
            let pairs = objdict::get_elements(self.pyobject);
            let mut map = serializer.serialize_map(Some(pairs.len()))?;
            for (key, e) in pairs.iter() {
                map.serialize_entry(&key, &self.clone_with_object(&e.1))?;
            }
            map.end()
        } else if let PyObjectPayload::None = self.pyobject.borrow().payload {
            serializer.serialize_none()
        } else {
            Err(serde::ser::Error::custom(format!(
                "Object of type '{:?}' is not serializable",
                self.pyobject.typ()
            )))
        }
    }
}

// This object is used as the seed for deserialization so we have access to the PyContext for type
// creation
#[derive(Clone)]
struct PyObjectDeserializer<'c> {
    vm: &'c VirtualMachine,
}

impl<'de> serde::de::DeserializeSeed<'de> for PyObjectDeserializer<'de> {
    type Value = PyObjectRef;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self.clone())
    }
}

impl<'de> Visitor<'de> for PyObjectDeserializer<'de> {
    type Value = PyObjectRef;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a type that can deserialise in Python")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_str(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_str(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // The JSON deserializer always uses the i64/u64 deserializers, so we only need to
        // implement those for now
        Ok(self.vm.ctx.new_int(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // The JSON deserializer always uses the i64/u64 deserializers, so we only need to
        // implement those for now
        Ok(self.vm.ctx.new_int(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_float(value))
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_bool(value))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut seq = Vec::with_capacity(access.size_hint().unwrap_or(0));
        while let Some(value) = access.next_element_seed(self.clone())? {
            seq.push(value);
        }
        Ok(self.vm.ctx.new_list(seq))
    }

    fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
    where
        M: serde::de::MapAccess<'de>,
    {
        let dict = self.vm.ctx.new_dict();
        // TODO: Given keys must be strings, we can probably do something more efficient
        // than wrapping the given object up and then unwrapping it to determine whether or
        // not it is a string
        while let Some((key_obj, value)) = access.next_entry_seed(self.clone(), self.clone())? {
            let key = match key_obj.borrow().payload {
                PyObjectPayload::String { ref value } => value.clone(),
                _ => unimplemented!("map keys must be strings"),
            };
            self.vm.ctx.set_item(&dict, &key, value);
        }
        Ok(dict)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.none.clone())
    }
}

/// Implement json.dumps
fn json_dumps(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: Implement non-trivial serialisation case
    arg_check!(vm, args, required = [(obj, None)]);
    let res = {
        let serializer = PyObjectSerializer { pyobject: obj, vm };
        serde_json::to_string(&serializer)
    };
    let string = res.map_err(|err| vm.new_type_error(format!("{}", err)))?;
    Ok(vm.context().new_str(string))
}

/// Implement json.loads
fn json_loads(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO: Implement non-trivial deserialization case
    arg_check!(vm, args, required = [(string, Some(vm.ctx.str_type()))]);
    let res = {
        let de = PyObjectDeserializer { vm };
        // TODO: Support deserializing string sub-classes
        de.deserialize(&mut serde_json::Deserializer::from_str(&objstr::get_value(
            &string,
        )))
    };

    res.map_err(|err| {
        let json_decode_error = vm
            .sys_module
            .get_item("modules")
            .unwrap()
            .get_item("json")
            .unwrap()
            .get_item("JSONDecodeError")
            .unwrap();
        let exc = vm.new_exception(json_decode_error, format!("{}", err));
        vm.ctx.set_attr(&exc, "lineno", vm.ctx.new_int(err.line()));
        vm.ctx.set_attr(&exc, "colno", vm.ctx.new_int(err.column()));
        exc
    })
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let json_mod = ctx.new_module("json", ctx.new_scope(None));

    ctx.set_attr(&json_mod, "dumps", ctx.new_rustfunc(json_dumps));
    ctx.set_attr(&json_mod, "loads", ctx.new_rustfunc(json_loads));

    // TODO: Make this a proper type with a constructor
    let json_decode_error = create_type(
        "JSONDecodeError",
        &ctx.type_type,
        &ctx.exceptions.exception_type,
        &ctx.dict_type,
    );
    ctx.set_attr(&json_mod, "JSONDecodeError", json_decode_error);

    json_mod
}
