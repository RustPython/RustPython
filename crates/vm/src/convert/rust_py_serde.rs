use core::{error::Error, fmt};

use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};

use crate::{
    PyObjectRef, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyDictRef},
};

// TODO: Add a shortcut implementation for `py_serde::PyObjectSerializer`.
/// Rust -> Python serializer.
///
/// # Panics
///
/// Panics on unit (`()`) values.
pub struct RustPySerDe<'a> {
    vm: &'a VirtualMachine,
    conf: RustPySerDeConf,
}

/// Configuration of Rust -> Python serializer.
#[derive(Eq, PartialEq, Debug, Clone)]
pub struct RustPySerDeConf {
    /// How to serialize lists.
    pub lists: RustPySerDeSeqKind,

    /// How to serialize tuples.
    pub tuples: RustPySerDeSeqKind,

    /// How to serialize tuple structures.
    pub tuple_structs: RustPySerDeSeqKind,

    /// How to serialize tuple variants of enums.
    pub tuple_variants: RustPySerDeSeqKind,
}

/// How to serialize sequences into Python types.
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
pub enum RustPySerDeSeqKind {
    /// Serialize sequences as Python tuples.
    AsTuple,

    /// Serialize sequences as Python lists.
    AsList,
}

impl<'a> Serializer for &'a RustPySerDe<'a> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    type SerializeSeq = RustToPySeqSerializer<'a>;
    type SerializeTuple = RustToPySeqSerializer<'a>;
    type SerializeTupleStruct = RustToPySeqSerializer<'a>;
    type SerializeTupleVariant = RustToPyTupleVariantSerializer<'a>;
    type SerializeMap = RustToPyMapSerializer<'a>;
    type SerializeStruct = RustToPyMapSerializer<'a>;
    type SerializeStructVariant = RustToPyStructVariantSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_bool(v).into())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_i128(self, v: i128) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_u128(self, v: u128) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_int(v).into())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_float(v.into()).into())
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_float(v).into())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_str(v).into())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_str(v).into())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_bytes(v.to_vec()).into())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.none())
    }

    fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        unimplemented!("BUG: Unit value cannot be serialized into a Python object")
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        unimplemented!("BUG: Unit struct value cannot be serialized into a Python object")
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(self.vm.ctx.new_str(variant).into())
    }

    fn serialize_newtype_struct<T>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error>
    where
        T: ?Sized + Serialize,
    {
        let dict = self.vm.ctx.new_dict();
        dict.set_item(variant, value.serialize(self)?, self.vm)?;
        Ok(dict.into())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        let vec = if let Some(capacity) = len {
            Vec::with_capacity(capacity)
        } else {
            Vec::new()
        };
        Ok(RustToPySeqSerializer { ser: self, vec })
    }

    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(RustToPySeqSerializer {
            ser: self,
            vec: Vec::with_capacity(len),
        })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(RustToPySeqSerializer {
            ser: self,
            vec: Vec::with_capacity(len),
        })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(RustToPyTupleVariantSerializer {
            ser: self,
            vec: Vec::with_capacity(len),
            variant,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(RustToPyMapSerializer {
            ser: self,
            dict: self.vm.ctx.new_dict(),
            key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(RustToPyMapSerializer {
            ser: self,
            dict: self.vm.ctx.new_dict(),
            key: None,
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(RustToPyStructVariantSerializer {
            ser: self,
            dict: self.vm.ctx.new_dict(),
            variant,
        })
    }
}

impl<'a> RustPySerDe<'a> {
    pub(crate) fn new(vm: &'a VirtualMachine, conf: RustPySerDeConf) -> Self {
        Self { vm, conf }
    }
}

impl Default for RustPySerDeConf {
    fn default() -> Self {
        Self {
            lists: RustPySerDeSeqKind::AsList,
            tuples: RustPySerDeSeqKind::AsTuple,
            tuple_structs: RustPySerDeSeqKind::AsTuple,
            tuple_variants: RustPySerDeSeqKind::AsTuple,
        }
    }
}

impl RustPySerDeConf {
    #[must_use]
    pub fn lists_as_tuples(mut self) -> Self {
        self.lists = RustPySerDeSeqKind::AsTuple;
        self
    }

    #[must_use]
    pub fn tuples_as_lists(mut self) -> Self {
        self.tuples = RustPySerDeSeqKind::AsList;
        self
    }

    #[must_use]
    pub fn tuple_variants_as_lists(mut self) -> Self {
        self.tuple_variants = RustPySerDeSeqKind::AsList;
        self
    }

    #[must_use]
    pub fn tuple_structs_as_lists(mut self) -> Self {
        self.tuple_structs = RustPySerDeSeqKind::AsList;
        self
    }
}

pub struct RustToPySeqSerializer<'a> {
    ser: &'a RustPySerDe<'a>,
    vec: Vec<PyObjectRef>,
}

impl SerializeSeq for RustToPySeqSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.vec.push(value.serialize(self.ser)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self.ser.conf.lists {
            RustPySerDeSeqKind::AsList => Ok(self.ser.vm.ctx.new_list(self.vec).into()),
            RustPySerDeSeqKind::AsTuple => Ok(self.ser.vm.ctx.new_tuple(self.vec).into()),
        }
    }
}

impl SerializeTuple for RustToPySeqSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.vec.push(value.serialize(self.ser)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self.ser.conf.tuples {
            RustPySerDeSeqKind::AsList => Ok(self.ser.vm.ctx.new_list(self.vec).into()),
            RustPySerDeSeqKind::AsTuple => Ok(self.ser.vm.ctx.new_tuple(self.vec).into()),
        }
    }
}

impl SerializeTupleStruct for RustToPySeqSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.vec.push(value.serialize(self.ser)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        match self.ser.conf.tuple_structs {
            RustPySerDeSeqKind::AsList => Ok(self.ser.vm.ctx.new_list(self.vec).into()),
            RustPySerDeSeqKind::AsTuple => Ok(self.ser.vm.ctx.new_tuple(self.vec).into()),
        }
    }
}

pub struct RustToPyTupleVariantSerializer<'a> {
    ser: &'a RustPySerDe<'a>,
    vec: Vec<PyObjectRef>,
    variant: &'a str,
}

impl SerializeTupleVariant for RustToPyTupleVariantSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_field<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.vec.push(value.serialize(self.ser)?);
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let obj = match self.ser.conf.tuple_variants {
            RustPySerDeSeqKind::AsList => self.ser.vm.ctx.new_list(self.vec).into(),
            RustPySerDeSeqKind::AsTuple => self.ser.vm.ctx.new_tuple(self.vec).into(),
        };
        let dict = self.ser.vm.ctx.new_dict();
        dict.set_item(self.variant, obj, self.ser.vm)?;
        Ok(dict.into())
    }
}

pub struct RustToPyMapSerializer<'a> {
    ser: &'a RustPySerDe<'a>,
    dict: PyDictRef,
    key: Option<PyObjectRef>,
}

impl SerializeMap for RustToPyMapSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_key<T>(&mut self, key: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        assert!(self.key.is_none(), "BUG: Double key serialization");
        self.key = Some(key.serialize(self.ser)?);
        Ok(())
    }

    fn serialize_value<T>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        let key = self.key.take().expect("BUG: Value without a key");
        self.dict
            .set_item(&*key, value.serialize(self.ser)?, self.ser.vm)?;
        Ok(())
    }

    fn serialize_entry<K, V>(&mut self, key: &K, value: &V) -> Result<(), Self::Error>
    where
        K: ?Sized + Serialize,
        V: ?Sized + Serialize,
    {
        self.dict.set_item(
            &*key.serialize(self.ser)?,
            value.serialize(self.ser)?,
            self.ser.vm,
        )?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.dict.into())
    }
}

impl SerializeStruct for RustToPyMapSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.dict
            .set_item(key, value.serialize(self.ser)?, self.ser.vm)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(self.dict.into())
    }
}

pub struct RustToPyStructVariantSerializer<'a> {
    ser: &'a RustPySerDe<'a>,
    dict: PyDictRef,
    variant: &'a str,
}

impl SerializeStructVariant for RustToPyStructVariantSerializer<'_> {
    type Ok = PyObjectRef;
    type Error = RustPySerDeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<(), Self::Error>
    where
        T: ?Sized + Serialize,
    {
        self.dict
            .set_item(key, value.serialize(self.ser)?, self.ser.vm)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        let dict = self.ser.vm.ctx.new_dict();
        dict.set_item(self.variant, self.dict.into(), self.ser.vm)?;
        Ok(dict.into())
    }
}

pub enum RustPySerDeError {
    Py(PyBaseExceptionRef),
    SerDe(String),
}

impl Error for RustPySerDeError {}

impl fmt::Debug for RustPySerDeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for RustPySerDeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Py(_) => f.write_str("RustPySerDeError::Py(...)"),
            Self::SerDe(_) => f.write_str("RustPySerDeError::SerDe(...)"),
        }
    }
}

impl serde::ser::Error for RustPySerDeError {
    fn custom<T>(msg: T) -> Self
    where
        T: fmt::Display,
    {
        Self::SerDe(format!("Rust <-> Python serde: {msg}"))
    }
}

impl From<PyBaseExceptionRef> for RustPySerDeError {
    fn from(value: PyBaseExceptionRef) -> Self {
        Self::Py(value)
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use serde::Serialize;

    use crate::{Interpreter, convert::RustPySerDeConf, py_serde::PyObjectSerializer};

    fn interpreter() -> Interpreter {
        Interpreter::without_stdlib(Default::default())
    }

    #[derive(Serialize)]
    struct TestStruct {
        val_bool: bool,
        val_u8: u8,
        val_i8: i8,
        val_u16: u16,
        val_i16: i16,
        val_u32: u32,
        val_i32: i32,
        val_u64: u64,
        val_i64: i64,
        val_u128: u128,
        val_i128: i128,
        val_usize: usize,
        val_isize: isize,
        val_f32: f32,
        val_f64: f64,
        val_char: char,
        val_str: &'static str,
        #[serde(with = "serde_bytes")]
        val_bytes: &'static [u8],
        val_none: Option<i32>,
        val_some: Option<&'static str>,
        val_list: Vec<i32>,
        val_tuple: (&'static str, i32),
        val_map: BTreeMap<&'static str, i32>,
        val_struct: TestSubStruct,
    }

    #[derive(Serialize)]
    struct TestSubStruct {
        a: TestSubEnum,
        b: TestSubEnum,
        c: TestSubEnum,
        d: TestSubEnum,
    }

    #[derive(Serialize)]
    enum TestSubEnum {
        Foo,
        Bar(bool),
        Baz(u32, &'static str),
        Qux { aaa: String, bbb: i32 },
    }

    #[test]
    fn serialize() {
        let val = TestStruct {
            val_bool: true,
            val_u8: u8::MAX,
            val_i8: i8::MIN,
            val_u16: u16::MAX,
            val_i16: i16::MIN,
            val_u32: u32::MAX,
            val_i32: i32::MIN,
            val_u64: u64::MAX,
            val_i64: i64::MIN,
            val_u128: u128::MAX,
            val_i128: i128::MIN,
            val_usize: usize::MAX,
            val_isize: isize::MIN,
            val_f32: 234.25,
            val_f64: 34342.3125,
            val_char: 'x',
            val_str: "hello",
            val_bytes: b"byte string",
            val_none: None,
            val_some: Some("some"),
            val_list: vec![1, 2, 3],
            val_tuple: ("tuple", 4),
            val_map: BTreeMap::from([("one", 1), ("two", 2)]),
            val_struct: TestSubStruct {
                a: TestSubEnum::Foo,
                b: TestSubEnum::Bar(false),
                c: TestSubEnum::Baz(357652, "test test one two three"),
                d: TestSubEnum::Qux {
                    aaa: "hello world!".to_string(),
                    bbb: -3,
                },
            },
        };

        interpreter().enter(|vm| {
            let val = vm.unwrap_pyresult(vm.with_serde(|serde| val.serialize(serde)));

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                from sys import maxsize\n\
                \n\
                assert len(val) == 24\n\
                assert val['val_bool']\n\
                assert val['val_u8'] == 255\n\
                assert val['val_i8'] == -128\n\
                assert val['val_u16'] == 65535\n\
                assert val['val_i16'] == -32768\n\
                assert val['val_u32'] == 4294967295\n\
                assert val['val_i32'] == -2147483648\n\
                assert val['val_u64'] == 18446744073709551615\n\
                assert val['val_i64'] == -9223372036854775808\n\
                assert val['val_u128'] == 340282366920938463463374607431768211455\n\
                assert val['val_i128'] == -170141183460469231731687303715884105728\n\
                assert val['val_usize'] == maxsize * 2 + 1\n\
                assert val['val_isize'] == -maxsize - 1\n\
                assert val['val_f32'] == 234.25\n\
                assert val['val_f64'] == 34342.3125\n\
                assert val['val_char'] == 'x'\n\
                assert val['val_str'] == 'hello'\n\
                assert isinstance(val['val_str'], str)\n\
                assert val['val_bytes'] == b'byte string'\n\
                assert isinstance(val['val_bytes'], bytes)\n\
                assert val['val_none'] is None\n\
                assert val['val_some'] == 'some'\n\
                assert val['val_list'] == [1, 2, 3]\n\
                assert isinstance(val['val_list'], list)\n\
                assert val['val_tuple'] == ('tuple', 4)\n\
                assert isinstance(val['val_tuple'], tuple)\n\
                assert val['val_map'] == {'one': 1, 'two': 2}\n\
                assert isinstance(val['val_map'], dict)\n\
                \n\
                val = val['val_struct']\n\
                assert len(val) == 4\n\
                assert isinstance(val, dict)\n\
                \n\
                assert val['a'] == 'Foo'\n\
                assert val['b'] == {'Bar': False}\n\
                assert val['c'] == {'Baz': (357652, 'test test one two three')}\n\
                assert val['d'] == {'Qux': {'aaa': 'hello world!', 'bbb': -3}}\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }

    #[test]
    fn serialize_lists_as_tuples() {
        interpreter().enter(|vm| {
            let val = vm.unwrap_pyresult(
                vm.with_serde_conf(RustPySerDeConf::default().lists_as_tuples(), |serde| {
                    vec![1, 2, 3].serialize(serde)
                }),
            );

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                assert val == (1, 2, 3)\n\
                assert isinstance(val, tuple)\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }

    #[test]
    fn serialize_tuples_as_lists() {
        interpreter().enter(|vm| {
            let val = vm.unwrap_pyresult(
                vm.with_serde_conf(RustPySerDeConf::default().tuples_as_lists(), |serde| {
                    (1, 2, 3).serialize(serde)
                }),
            );

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                assert val == [1, 2, 3]\n\
                assert isinstance(val, list)\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }

    #[derive(Serialize)]
    struct TestTupleStruct(u8, u8, u8);

    #[test]
    fn serialize_tuple_structs_as_lists() {
        interpreter().enter(|vm| {
            let val = vm.unwrap_pyresult(vm.with_serde_conf(
                RustPySerDeConf::default().tuple_structs_as_lists(),
                |serde| TestTupleStruct(3, 2, 1).serialize(serde),
            ));

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                assert val == [3, 2, 1]\n\
                assert isinstance(val, list)\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }

    #[derive(Serialize)]
    enum TupleVariant {
        Variant(u8, u8, u8),
    }

    #[test]
    fn serialize_tuple_variants_as_lists() {
        interpreter().enter(|vm| {
            let val = vm.unwrap_pyresult(vm.with_serde_conf(
                RustPySerDeConf::default().tuple_variants_as_lists(),
                |serde| TupleVariant::Variant(11, 22, 33).serialize(serde),
            ));

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                assert val == {'Variant': [11, 22, 33]}\n\
                assert isinstance(val['Variant'], list)\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }

    #[test]
    fn serialize_py_object() {
        interpreter().enter(|vm| {
            let obj = vm.ctx.new_str("test");
            let val = vm.unwrap_pyresult(
                vm.with_serde(|serde| PyObjectSerializer::new(vm, &obj.into()).serialize(serde)),
            );

            let scope = vm.new_scope_with_builtins();
            vm.unwrap_pyresult(scope.globals.set_item("val", val, vm));

            let script = "\
                assert val == 'test'\n\
                assert isinstance(val, str)\n\
            ";
            let _ = vm.unwrap_pyresult(vm.run_block_expr(scope, script));
        });
    }
}
