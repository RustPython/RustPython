//! This package contains the python basic/builtin types
//! 7 common PyRef type aliases are exposed - [`PyBytesRef`], [`PyDictRef`], [`PyIntRef`], [`PyListRef`], [`PyStrRef`], [`PyTypeRef`], [`PyTupleRef`]
//! Do not add more PyRef type aliases. They will be rare enough to use directly `PyRef<T>`.

pub(crate) mod asyncgenerator;
pub use asyncgenerator::PyAsyncGen;
pub(crate) mod builtin_func;
pub(crate) mod bytearray;
pub use bytearray::PyByteArray;
pub(crate) mod bytes;
pub use bytes::{PyBytes, PyBytesRef};
pub(crate) mod classmethod;
pub use classmethod::PyClassMethod;
pub(crate) mod code;
pub use code::PyCode;
pub(crate) mod complex;
pub use complex::PyComplex;
pub(crate) mod coroutine;
pub use coroutine::PyCoroutine;
pub(crate) mod dict;
pub use dict::{PyDict, PyDictRef};
pub(crate) mod enumerate;
pub use enumerate::PyEnumerate;
pub(crate) mod filter;
pub use filter::PyFilter;
pub(crate) mod float;
pub use float::PyFloat;
pub(crate) mod frame;
pub(crate) mod function;
pub use function::{PyBoundMethod, PyFunction};
pub(crate) mod generator;
pub use generator::PyGenerator;
pub(crate) mod genericalias;
pub use genericalias::PyGenericAlias;
pub(crate) mod getset;
pub use getset::PyGetSet;
pub(crate) mod int;
pub use int::{PyInt, PyIntRef};
pub(crate) mod interpolation;
pub use interpolation::PyInterpolation;
pub(crate) mod iter;
pub use iter::*;
pub(crate) mod list;
pub use list::{PyList, PyListRef};
pub(crate) mod map;
pub use map::PyMap;
pub(crate) mod mappingproxy;
pub use mappingproxy::PyMappingProxy;
pub(crate) mod memory;
pub use memory::PyMemoryView;
pub(crate) mod module;
pub use module::{PyModule, PyModuleDef};
pub(crate) mod namespace;
pub use namespace::PyNamespace;
pub(crate) mod object;
pub use object::PyBaseObject;
pub(crate) mod property;
pub use property::PyProperty;
#[path = "bool.rs"]
pub(crate) mod bool_;
pub use bool_::PyBool;
#[path = "str.rs"]
pub(crate) mod pystr;
pub use pystr::{PyStr, PyStrInterned, PyStrRef, PyUtf8Str, PyUtf8StrRef};
#[path = "super.rs"]
pub(crate) mod super_;
pub use super_::PySuper;
#[path = "type.rs"]
pub(crate) mod type_;
pub use type_::{PyType, PyTypeRef};
pub(crate) mod range;
pub use range::PyRange;
pub(crate) mod set;
pub use set::{PyFrozenSet, PySet};
pub(crate) mod singletons;
pub use singletons::{PyNone, PyNotImplemented};
pub(crate) mod slice;
pub use slice::{PyEllipsis, PySlice};
pub(crate) mod staticmethod;
pub use staticmethod::PyStaticMethod;
pub(crate) mod template;
pub use template::{PyTemplate, PyTemplateIter};
pub(crate) mod traceback;
pub use traceback::PyTraceback;
pub(crate) mod tuple;
pub use tuple::{PyTuple, PyTupleRef};
pub(crate) mod weakproxy;
pub use weakproxy::PyWeakProxy;
pub(crate) mod weakref;
pub use weakref::PyWeak;
pub(crate) mod zip;
pub use zip::PyZip;
#[path = "union.rs"]
pub(crate) mod union_;
pub use union_::PyUnion;
pub(crate) mod descriptor;

pub use float::try_to_bigint as try_f64_to_bigint;
pub use int::try_to_float as try_bigint_to_f64;

pub use crate::exceptions::types::*;
