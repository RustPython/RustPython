use std::fmt;

use serde;
use serde::de::Visitor;
use serde::ser::{SerializeMap, SerializeSeq};

use crate::obj::{
    objbool,
    objdict::PyDictRef,
    objfloat, objint, objsequence,
    objstr::{self, PyString},
    objtype,
};
use crate::pyobject::{IdProtocol, ItemProtocol, PyObjectRef, TypeProtocol};
use crate::VirtualMachine;
use num_traits::cast::ToPrimitive;

// We need to have a VM available to serialise a PyObject based on its subclass, so we implement
// PyObject serialisation via a proxy object which holds a reference to a VM
pub struct PyObjectSerializer<'s> {
    pyobject: &'s PyObjectRef,
    vm: &'s VirtualMachine,
}

impl<'s> PyObjectSerializer<'s> {
    pub fn new(vm: &'s VirtualMachine, pyobject: &'s PyObjectRef) -> Self {
        PyObjectSerializer { pyobject, vm }
    }

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
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.list_type()) {
            let elements = objsequence::get_elements_list(self.pyobject);
            serialize_seq_elements(serializer, &elements)
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.tuple_type()) {
            let elements = objsequence::get_elements_tuple(self.pyobject);
            serialize_seq_elements(serializer, &elements)
        } else if objtype::isinstance(self.pyobject, &self.vm.ctx.dict_type()) {
            let dict: PyDictRef = self.pyobject.clone().downcast().unwrap();
            let pairs: Vec<_> = dict.into_iter().collect();
            let mut map = serializer.serialize_map(Some(pairs.len()))?;
            for (key, e) in pairs.iter() {
                map.serialize_entry(&self.clone_with_object(key), &self.clone_with_object(&e))?;
            }
            map.end()
        } else if self.pyobject.is(&self.vm.get_none()) {
            serializer.serialize_none()
        } else {
            Err(serde::ser::Error::custom(format!(
                "Object of type '{:?}' is not serializable",
                self.pyobject.class()
            )))
        }
    }
}

// This object is used as the seed for deserialization so we have access to the PyContext for type
// creation
#[derive(Clone)]
pub struct PyObjectDeserializer<'c> {
    vm: &'c VirtualMachine,
}

impl<'c> PyObjectDeserializer<'c> {
    pub fn new(vm: &'c VirtualMachine) -> Self {
        PyObjectDeserializer { vm }
    }
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
            let key: String = match key_obj.payload::<PyString>() {
                Some(PyString { ref value }) => value.clone(),
                _ => unimplemented!("map keys must be strings"),
            };
            dict.set_item(&key, value, self.vm).unwrap();
        }
        Ok(dict.into_object())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.get_none())
    }
}
