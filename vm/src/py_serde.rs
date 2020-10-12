use num_traits::cast::ToPrimitive;
use num_traits::sign::Signed;
use serde::de::{DeserializeSeed, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq};

use crate::builtins::{dict::PyDictRef, float, int, list::PyList, pybool, pystr, tuple::PyTuple};
use crate::pyobject::{BorrowValue, ItemProtocol, PyObjectRef, TypeProtocol};
use crate::VirtualMachine;

#[inline]
pub fn serialize<S>(
    vm: &VirtualMachine,
    pyobject: &PyObjectRef,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    PyObjectSerializer { vm, pyobject }.serialize(serializer)
}

#[inline]
pub fn deserialize<'de, D>(
    vm: &'de VirtualMachine,
    deserializer: D,
) -> Result<<PyObjectDeserializer as DeserializeSeed>::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    PyObjectDeserializer { vm }.deserialize(deserializer)
}

// We need to have a VM available to serialise a PyObject based on its subclass, so we implement
// PyObject serialisation via a proxy object which holds a reference to a VM
pub struct PyObjectSerializer<'s> {
    pyobject: &'s PyObjectRef,
    vm: &'s VirtualMachine,
}

impl<'s> PyObjectSerializer<'s> {
    pub fn new(vm: &'s VirtualMachine, pyobject: &'s PyObjectRef) -> Self {
        PyObjectSerializer { vm, pyobject }
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
            |serializer: S, elements: &[PyObjectRef]| -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(Some(elements.len()))?;
                for e in elements.iter() {
                    seq.serialize_element(&self.clone_with_object(e))?;
                }
                seq.end()
            };
        if self.pyobject.isinstance(&self.vm.ctx.types.str_type) {
            serializer.serialize_str(pystr::borrow_value(&self.pyobject))
        } else if self.pyobject.isinstance(&self.vm.ctx.types.float_type) {
            serializer.serialize_f64(float::get_value(self.pyobject))
        } else if self.pyobject.isinstance(&self.vm.ctx.types.bool_type) {
            serializer.serialize_bool(pybool::get_value(self.pyobject))
        } else if self.pyobject.isinstance(&self.vm.ctx.types.int_type) {
            let v = int::get_value(self.pyobject);
            let int_too_large = || serde::ser::Error::custom("int too large to serialize");
            // TODO: serialize BigInt when it does not fit into i64
            // BigInt implements serialization to a tuple of sign and a list of u32s,
            // eg. -1 is [-1, [1]], 0 is [0, []], 12345678900000654321 is [1, [2710766577,2874452364]]
            // CPython serializes big ints as long decimal integer literals
            if v.is_positive() {
                serializer.serialize_u64(v.to_u64().ok_or_else(int_too_large)?)
            } else {
                serializer.serialize_i64(v.to_i64().ok_or_else(int_too_large)?)
            }
        } else if let Some(list) = self.pyobject.payload_if_subclass::<PyList>(self.vm) {
            serialize_seq_elements(serializer, &list.borrow_value())
        } else if let Some(tuple) = self.pyobject.payload_if_subclass::<PyTuple>(self.vm) {
            serialize_seq_elements(serializer, tuple.borrow_value())
        } else if self.pyobject.isinstance(&self.vm.ctx.types.dict_type) {
            let dict: PyDictRef = self.pyobject.clone().downcast().unwrap();
            let pairs: Vec<_> = dict.into_iter().collect();
            let mut map = serializer.serialize_map(Some(pairs.len()))?;
            for (key, e) in pairs.iter() {
                map.serialize_entry(&self.clone_with_object(key), &self.clone_with_object(&e))?;
            }
            map.end()
        } else if self.vm.is_none(&self.pyobject) {
            serializer.serialize_none()
        } else {
            Err(serde::ser::Error::custom(format!(
                "Object of type '{}' is not serializable",
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

impl<'de> DeserializeSeed<'de> for PyObjectDeserializer<'de> {
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

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a type that can deserialise in Python")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_bool(value))
    }

    // Other signed integers delegate to this method by default, it’s the only one needed
    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_int(value))
    }

    // Other unsigned integers delegate to this method by default, it’s the only one needed
    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_int(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_float(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // Owned value needed anyway, delegate to visit_string
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.new_str(value))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(self.vm.ctx.none())
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
        // Although JSON keys must be strings, implementation accepts any keys
        // and can be reused by other deserializers without such limit
        while let Some((key_obj, value)) = access.next_entry_seed(self.clone(), self.clone())? {
            dict.set_item(key_obj, value, self.vm).unwrap();
        }
        Ok(dict.into_object())
    }
}
