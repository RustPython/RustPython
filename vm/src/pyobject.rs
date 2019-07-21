use std::any::Any;
use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::ops::Deref;
use std::ptr;
use std::rc::Rc;

use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::{One, Zero};

use crate::bytecode;
use crate::exceptions;
use crate::frame::Scope;
use crate::function::{IntoPyNativeFunc, PyFuncArgs};
use crate::obj::objbool;
use crate::obj::objbuiltinfunc::PyBuiltinFunction;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objclassmethod::{self, PyClassMethod};
use crate::obj::objcode;
use crate::obj::objcode::PyCodeRef;
use crate::obj::objcomplex::{self, PyComplex};
use crate::obj::objdict::{self, PyDict, PyDictRef};
use crate::obj::objellipsis;
use crate::obj::objenumerate;
use crate::obj::objfilter;
use crate::obj::objfloat::{self, PyFloat};
use crate::obj::objframe;
use crate::obj::objfunction::{self, PyFunction, PyMethod};
use crate::obj::objgenerator;
use crate::obj::objint::{self, PyInt, PyIntRef};
use crate::obj::objiter;
use crate::obj::objlist::{self, PyList};
use crate::obj::objmap;
use crate::obj::objmappingproxy;
use crate::obj::objmemory;
use crate::obj::objmodule::{self, PyModule};
use crate::obj::objnamespace::{self, PyNamespace};
use crate::obj::objnone::{self, PyNone, PyNoneRef};
use crate::obj::objobject;
use crate::obj::objproperty;
use crate::obj::objproperty::PropertyBuilder;
use crate::obj::objrange;
use crate::obj::objset::{self, PySet};
use crate::obj::objslice;
use crate::obj::objstaticmethod;
use crate::obj::objstr;
use crate::obj::objsuper;
use crate::obj::objtuple::{self, PyTuple, PyTupleRef};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::obj::objweakproxy;
use crate::obj::objweakref;
use crate::obj::objzip;
use crate::vm::VirtualMachine;
use indexmap::IndexMap;

/* Python objects and references.

Okay, so each python object itself is an class itself (PyObject). Each
python object can have several references to it (PyObjectRef). These
references are Rc (reference counting) rust smart pointers. So when
all references are destroyed, the object itself also can be cleaned up.
Basically reference counting, but then done by rust.

*/

/*
 * Good reference: https://github.com/ProgVal/pythonvm-rust/blob/master/src/objects/mod.rs
 */

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
pub type PyObjectRef = Rc<PyObject<dyn PyObjectPayload>>;

/// Use this type for function which return a python object or and exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyObjectRef>; // A valid value, or an exception

/// For attributes we do not use a dict, but a hashmap. This is probably
/// faster, unordered, and only supports strings as keys.
/// TODO: class attributes should maintain insertion order (use IndexMap here)
pub type PyAttributes = HashMap<String, PyObjectRef>;

impl fmt::Display for PyObject<dyn PyObjectPayload> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(PyClass { ref name, .. }) = self.payload::<PyClass>() {
            let type_name = self.class().name.clone();
            // We don't have access to a vm, so just assume that if its parent's name
            // is type, it's a type
            if type_name == "type" {
                return write!(f, "type object '{}'", name);
            } else {
                return write!(f, "'{}' object", type_name);
            }
        }

        if let Some(PyModule { ref name, .. }) = self.payload::<PyModule>() {
            return write!(f, "module '{}'", name);
        }
        write!(f, "'{}' object", self.class().name)
    }
}

#[derive(Debug)]
pub struct PyContext {
    pub bytes_type: PyClassRef,
    pub bytesiterator_type: PyClassRef,
    pub bytearray_type: PyClassRef,
    pub bytearrayiterator_type: PyClassRef,
    pub bool_type: PyClassRef,
    pub classmethod_type: PyClassRef,
    pub code_type: PyClassRef,
    pub dict_type: PyClassRef,
    pub ellipsis_type: PyClassRef,
    pub enumerate_type: PyClassRef,
    pub filter_type: PyClassRef,
    pub float_type: PyClassRef,
    pub frame_type: PyClassRef,
    pub frozenset_type: PyClassRef,
    pub generator_type: PyClassRef,
    pub int_type: PyClassRef,
    pub iter_type: PyClassRef,
    pub complex_type: PyClassRef,
    pub true_value: PyIntRef,
    pub false_value: PyIntRef,
    pub list_type: PyClassRef,
    pub listiterator_type: PyClassRef,
    pub listreverseiterator_type: PyClassRef,
    pub dictkeyiterator_type: PyClassRef,
    pub dictvalueiterator_type: PyClassRef,
    pub dictitemiterator_type: PyClassRef,
    pub dictkeys_type: PyClassRef,
    pub dictvalues_type: PyClassRef,
    pub dictitems_type: PyClassRef,
    pub map_type: PyClassRef,
    pub memoryview_type: PyClassRef,
    pub none: PyNoneRef,
    pub ellipsis: PyEllipsisRef,
    pub not_implemented: PyNotImplementedRef,
    pub tuple_type: PyClassRef,
    pub tupleiterator_type: PyClassRef,
    pub set_type: PyClassRef,
    pub staticmethod_type: PyClassRef,
    pub super_type: PyClassRef,
    pub str_type: PyClassRef,
    pub range_type: PyClassRef,
    pub rangeiterator_type: PyClassRef,
    pub slice_type: PyClassRef,
    pub type_type: PyClassRef,
    pub zip_type: PyClassRef,
    pub function_type: PyClassRef,
    pub builtin_function_or_method_type: PyClassRef,
    pub property_type: PyClassRef,
    pub readonly_property_type: PyClassRef,
    pub module_type: PyClassRef,
    pub namespace_type: PyClassRef,
    pub bound_method_type: PyClassRef,
    pub weakref_type: PyClassRef,
    pub weakproxy_type: PyClassRef,
    pub mappingproxy_type: PyClassRef,
    pub object: PyClassRef,
    pub exceptions: exceptions::ExceptionZoo,
}

pub fn create_type(name: &str, type_type: &PyClassRef, base: &PyClassRef) -> PyClassRef {
    let dict = PyAttributes::new();
    objtype::new(type_type.clone(), name, vec![base.clone()], dict).unwrap()
}

pub type PyNotImplementedRef = PyRef<PyNotImplemented>;

#[derive(Debug)]
pub struct PyNotImplemented;

impl PyValue for PyNotImplemented {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.not_implemented().class()
    }
}

pub type PyEllipsisRef = PyRef<PyEllipsis>;

#[derive(Debug)]
pub struct PyEllipsis;

impl PyValue for PyEllipsis {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.ellipsis_type.clone()
    }
}

fn init_type_hierarchy() -> (PyClassRef, PyClassRef) {
    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = unsafe {
        let object_type = PyObject {
            typ: mem::uninitialized(), // !
            dict: None,
            payload: PyClass {
                name: String::from("object"),
                mro: vec![],
                subclasses: RefCell::new(vec![]),
                attributes: RefCell::new(PyAttributes::new()),
            },
        }
        .into_ref();

        let type_type = PyObject {
            typ: mem::uninitialized(), // !
            dict: None,
            payload: PyClass {
                name: String::from("type"),
                mro: vec![object_type.clone().downcast().unwrap()],
                subclasses: RefCell::new(vec![]),
                attributes: RefCell::new(PyAttributes::new()),
            },
        }
        .into_ref();

        let object_type_ptr = PyObjectRef::into_raw(object_type.clone()) as *mut PyObject<PyClass>;
        let type_type_ptr = PyObjectRef::into_raw(type_type.clone()) as *mut PyObject<PyClass>;

        let type_type: PyClassRef = type_type.downcast().unwrap();
        let object_type: PyClassRef = object_type.downcast().unwrap();

        ptr::write(&mut (*object_type_ptr).typ, type_type.clone());
        ptr::write(&mut (*type_type_ptr).typ, type_type.clone());

        (type_type, object_type)
    };

    object_type
        .subclasses
        .borrow_mut()
        .push(objweakref::PyWeak::downgrade(&type_type.as_object()));

    (type_type, object_type)
}

// Basic objects:
impl PyContext {
    pub fn new() -> Self {
        flame_guard!("init PyContext");
        let (type_type, object_type) = init_type_hierarchy();

        let dict_type = create_type("dict", &type_type, &object_type);
        let module_type = create_type("module", &type_type, &object_type);
        let namespace_type = create_type("SimpleNamespace", &type_type, &object_type);
        let classmethod_type = create_type("classmethod", &type_type, &object_type);
        let staticmethod_type = create_type("staticmethod", &type_type, &object_type);
        let function_type = create_type("function", &type_type, &object_type);
        let builtin_function_or_method_type =
            create_type("builtin_function_or_method", &type_type, &object_type);
        let property_type = create_type("property", &type_type, &object_type);
        let readonly_property_type = create_type("readonly_property", &type_type, &object_type);
        let super_type = create_type("super", &type_type, &object_type);
        let weakref_type = create_type("ref", &type_type, &object_type);
        let weakproxy_type = create_type("weakproxy", &type_type, &object_type);
        let generator_type = create_type("generator", &type_type, &object_type);
        let bound_method_type = create_type("method", &type_type, &object_type);
        let str_type = create_type("str", &type_type, &object_type);
        let list_type = create_type("list", &type_type, &object_type);
        let listiterator_type = create_type("list_iterator", &type_type, &object_type);
        let listreverseiterator_type =
            create_type("list_reverseiterator", &type_type, &object_type);
        let dictkeys_type = create_type("dict_keys", &type_type, &object_type);
        let dictvalues_type = create_type("dict_values", &type_type, &object_type);
        let dictitems_type = create_type("dict_items", &type_type, &object_type);
        let dictkeyiterator_type = create_type("dict_keyiterator", &type_type, &object_type);
        let dictvalueiterator_type = create_type("dict_valueiterator", &type_type, &object_type);
        let dictitemiterator_type = create_type("dict_itemiterator", &type_type, &object_type);
        let set_type = create_type("set", &type_type, &object_type);
        let frozenset_type = create_type("frozenset", &type_type, &object_type);
        let int_type = create_type("int", &type_type, &object_type);
        let float_type = create_type("float", &type_type, &object_type);
        let frame_type = create_type("frame", &type_type, &object_type);
        let complex_type = create_type("complex", &type_type, &object_type);
        let bytes_type = create_type("bytes", &type_type, &object_type);
        let bytesiterator_type = create_type("bytes_iterator", &type_type, &object_type);
        let bytearray_type = create_type("bytearray", &type_type, &object_type);
        let bytearrayiterator_type = create_type("bytearray_iterator", &type_type, &object_type);
        let tuple_type = create_type("tuple", &type_type, &object_type);
        let tupleiterator_type = create_type("tuple_iterator", &type_type, &object_type);
        let iter_type = create_type("iter", &type_type, &object_type);
        let enumerate_type = create_type("enumerate", &type_type, &object_type);
        let filter_type = create_type("filter", &type_type, &object_type);
        let map_type = create_type("map", &type_type, &object_type);
        let zip_type = create_type("zip", &type_type, &object_type);
        let bool_type = create_type("bool", &type_type, &int_type);
        let memoryview_type = create_type("memoryview", &type_type, &object_type);
        let code_type = create_type("code", &type_type, &object_type);
        let range_type = create_type("range", &type_type, &object_type);
        let rangeiterator_type = create_type("range_iterator", &type_type, &object_type);
        let slice_type = create_type("slice", &type_type, &object_type);
        let mappingproxy_type = create_type("mappingproxy", &type_type, &object_type);
        let exceptions = exceptions::ExceptionZoo::new(&type_type, &object_type);

        fn create_object<T: PyObjectPayload>(payload: T, cls: &PyClassRef) -> PyRef<T> {
            PyRef {
                obj: PyObject::new(payload, cls.clone(), None),
                _payload: PhantomData,
            }
        }

        let none_type = create_type("NoneType", &type_type, &object_type);
        let none = create_object(PyNone, &none_type);

        let ellipsis_type = create_type("EllipsisType", &type_type, &object_type);
        let ellipsis = create_object(PyEllipsis, &ellipsis_type);

        let not_implemented_type = create_type("NotImplementedType", &type_type, &object_type);
        let not_implemented = create_object(PyNotImplemented, &not_implemented_type);

        let true_value = create_object(PyInt::new(BigInt::one()), &bool_type);
        let false_value = create_object(PyInt::new(BigInt::zero()), &bool_type);
        let context = PyContext {
            bool_type,
            memoryview_type,
            bytearray_type,
            bytearrayiterator_type,
            bytes_type,
            bytesiterator_type,
            code_type,
            complex_type,
            classmethod_type,
            int_type,
            float_type,
            frame_type,
            staticmethod_type,
            list_type,
            listiterator_type,
            listreverseiterator_type,
            dictkeys_type,
            dictvalues_type,
            dictitems_type,
            dictkeyiterator_type,
            dictvalueiterator_type,
            dictitemiterator_type,
            set_type,
            frozenset_type,
            true_value,
            false_value,
            tuple_type,
            tupleiterator_type,
            iter_type,
            ellipsis_type,
            enumerate_type,
            filter_type,
            map_type,
            zip_type,
            dict_type,
            none,
            ellipsis,
            not_implemented,
            str_type,
            range_type,
            rangeiterator_type,
            slice_type,
            object: object_type,
            function_type,
            builtin_function_or_method_type,
            super_type,
            mappingproxy_type,
            property_type,
            readonly_property_type,
            generator_type,
            module_type,
            namespace_type,
            bound_method_type,
            weakref_type,
            weakproxy_type,
            type_type,
            exceptions,
        };
        objtype::init(&context);
        objlist::init(&context);
        objset::init(&context);
        objtuple::init(&context);
        objobject::init(&context);
        objdict::init(&context);
        objfunction::init(&context);
        objstaticmethod::init(&context);
        objclassmethod::init(&context);
        objgenerator::init(&context);
        objint::init(&context);
        objfloat::init(&context);
        objcomplex::init(&context);
        objbytes::init(&context);
        objbytearray::init(&context);
        objproperty::init(&context);
        objmemory::init(&context);
        objstr::init(&context);
        objrange::init(&context);
        objslice::init(&context);
        objsuper::init(&context);
        objtuple::init(&context);
        objiter::init(&context);
        objellipsis::init(&context);
        objenumerate::init(&context);
        objfilter::init(&context);
        objmap::init(&context);
        objzip::init(&context);
        objbool::init(&context);
        objcode::init(&context);
        objframe::init(&context);
        objweakref::init(&context);
        objweakproxy::init(&context);
        objnone::init(&context);
        objmodule::init(&context);
        objnamespace::init(&context);
        objmappingproxy::init(&context);
        exceptions::init(&context);
        context
    }

    pub fn bytearray_type(&self) -> PyClassRef {
        self.bytearray_type.clone()
    }

    pub fn bytearrayiterator_type(&self) -> PyClassRef {
        self.bytearrayiterator_type.clone()
    }

    pub fn bytes_type(&self) -> PyClassRef {
        self.bytes_type.clone()
    }

    pub fn bytesiterator_type(&self) -> PyClassRef {
        self.bytesiterator_type.clone()
    }

    pub fn code_type(&self) -> PyClassRef {
        self.code_type.clone()
    }

    pub fn complex_type(&self) -> PyClassRef {
        self.complex_type.clone()
    }

    pub fn dict_type(&self) -> PyClassRef {
        self.dict_type.clone()
    }

    pub fn float_type(&self) -> PyClassRef {
        self.float_type.clone()
    }

    pub fn frame_type(&self) -> PyClassRef {
        self.frame_type.clone()
    }

    pub fn int_type(&self) -> PyClassRef {
        self.int_type.clone()
    }

    pub fn list_type(&self) -> PyClassRef {
        self.list_type.clone()
    }

    pub fn listiterator_type(&self) -> PyClassRef {
        self.listiterator_type.clone()
    }

    pub fn listreverseiterator_type(&self) -> PyClassRef {
        self.listreverseiterator_type.clone()
    }

    pub fn module_type(&self) -> PyClassRef {
        self.module_type.clone()
    }

    pub fn namespace_type(&self) -> PyClassRef {
        self.namespace_type.clone()
    }

    pub fn set_type(&self) -> PyClassRef {
        self.set_type.clone()
    }

    pub fn range_type(&self) -> PyClassRef {
        self.range_type.clone()
    }

    pub fn rangeiterator_type(&self) -> PyClassRef {
        self.rangeiterator_type.clone()
    }

    pub fn slice_type(&self) -> PyClassRef {
        self.slice_type.clone()
    }

    pub fn frozenset_type(&self) -> PyClassRef {
        self.frozenset_type.clone()
    }

    pub fn bool_type(&self) -> PyClassRef {
        self.bool_type.clone()
    }

    pub fn memoryview_type(&self) -> PyClassRef {
        self.memoryview_type.clone()
    }

    pub fn tuple_type(&self) -> PyClassRef {
        self.tuple_type.clone()
    }

    pub fn tupleiterator_type(&self) -> PyClassRef {
        self.tupleiterator_type.clone()
    }

    pub fn iter_type(&self) -> PyClassRef {
        self.iter_type.clone()
    }

    pub fn enumerate_type(&self) -> PyClassRef {
        self.enumerate_type.clone()
    }

    pub fn filter_type(&self) -> PyClassRef {
        self.filter_type.clone()
    }

    pub fn map_type(&self) -> PyClassRef {
        self.map_type.clone()
    }

    pub fn zip_type(&self) -> PyClassRef {
        self.zip_type.clone()
    }

    pub fn str_type(&self) -> PyClassRef {
        self.str_type.clone()
    }

    pub fn super_type(&self) -> PyClassRef {
        self.super_type.clone()
    }

    pub fn function_type(&self) -> PyClassRef {
        self.function_type.clone()
    }

    pub fn builtin_function_or_method_type(&self) -> PyClassRef {
        self.builtin_function_or_method_type.clone()
    }

    pub fn property_type(&self) -> PyClassRef {
        self.property_type.clone()
    }

    pub fn readonly_property_type(&self) -> PyClassRef {
        self.readonly_property_type.clone()
    }

    pub fn classmethod_type(&self) -> PyClassRef {
        self.classmethod_type.clone()
    }

    pub fn staticmethod_type(&self) -> PyClassRef {
        self.staticmethod_type.clone()
    }

    pub fn generator_type(&self) -> PyClassRef {
        self.generator_type.clone()
    }

    pub fn bound_method_type(&self) -> PyClassRef {
        self.bound_method_type.clone()
    }

    pub fn weakref_type(&self) -> PyClassRef {
        self.weakref_type.clone()
    }

    pub fn weakproxy_type(&self) -> PyClassRef {
        self.weakproxy_type.clone()
    }

    pub fn type_type(&self) -> PyClassRef {
        self.type_type.clone()
    }

    pub fn none(&self) -> PyObjectRef {
        self.none.clone().into_object()
    }

    pub fn ellipsis(&self) -> PyObjectRef {
        self.ellipsis.clone().into_object()
    }

    pub fn not_implemented(&self) -> PyObjectRef {
        self.not_implemented.clone().into_object()
    }

    pub fn object(&self) -> PyClassRef {
        self.object.clone()
    }

    pub fn new_int<T: Into<BigInt>>(&self, i: T) -> PyObjectRef {
        PyObject::new(PyInt::new(i), self.int_type(), None)
    }

    pub fn new_float(&self, value: f64) -> PyObjectRef {
        PyObject::new(PyFloat::from(value), self.float_type(), None)
    }

    pub fn new_complex(&self, value: Complex64) -> PyObjectRef {
        PyObject::new(PyComplex::from(value), self.complex_type(), None)
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(objstr::PyString { value: s }, self.str_type(), None)
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(objbytes::PyBytes::new(data), self.bytes_type(), None)
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            objbytearray::PyByteArray::new(data),
            self.bytearray_type(),
            None,
        )
    }

    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        if b {
            self.true_value.clone().into_object()
        } else {
            self.false_value.clone().into_object()
        }
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyTuple::from(elements), self.tuple_type(), None)
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyList::from(elements), self.list_type(), None)
    }

    pub fn new_set(&self) -> PyObjectRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the objset code itself.
        PyObject::new(PySet::default(), self.set_type(), None)
    }

    pub fn new_dict(&self) -> PyDictRef {
        PyObject::new(PyDict::default(), self.dict_type(), None)
            .downcast()
            .unwrap()
    }

    pub fn new_class(&self, name: &str, base: PyClassRef) -> PyClassRef {
        objtype::new(self.type_type(), name, vec![base], PyAttributes::new()).unwrap()
    }

    pub fn new_module(&self, name: &str, dict: PyDictRef) -> PyObjectRef {
        PyObject::new(
            PyModule {
                name: name.to_string(),
            },
            self.module_type.clone(),
            Some(dict),
        )
    }

    pub fn new_namespace(&self) -> PyObjectRef {
        PyObject::new(PyNamespace, self.namespace_type(), Some(self.new_dict()))
    }

    pub fn new_rustfunc<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyBuiltinFunction::new(f.into_func()),
            self.builtin_function_or_method_type(),
            None,
        )
    }

    pub fn new_classmethod<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyClassMethod {
                callable: self.new_rustfunc(f),
            },
            self.classmethod_type(),
            None,
        )
    }

    pub fn new_property<F, I, V>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<I, V>,
    {
        PropertyBuilder::new(self).add_getter(f).create()
    }

    pub fn new_code_object(&self, code: bytecode::CodeObject) -> PyCodeRef {
        PyObject::new(objcode::PyCode::new(code), self.code_type(), None)
            .downcast()
            .unwrap()
    }

    pub fn new_function(
        &self,
        code_obj: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> PyObjectRef {
        PyObject::new(
            PyFunction::new(code_obj, scope, defaults, kw_only_defaults),
            self.function_type(),
            Some(self.new_dict()),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyMethod::new(object, function),
            self.bound_method_type(),
            None,
        )
    }

    pub fn new_instance(&self, class: PyClassRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject {
            typ: class,
            dict,
            payload: objobject::PyInstance,
        }
        .into_ref()
    }

    pub fn unwrap_constant(&self, value: &bytecode::Constant) -> PyObjectRef {
        match *value {
            bytecode::Constant::Integer { ref value } => self.new_int(value.clone()),
            bytecode::Constant::Float { ref value } => self.new_float(*value),
            bytecode::Constant::Complex { ref value } => self.new_complex(*value),
            bytecode::Constant::String { ref value } => self.new_str(value.clone()),
            bytecode::Constant::Bytes { ref value } => self.new_bytes(value.clone()),
            bytecode::Constant::Boolean { ref value } => self.new_bool(value.clone()),
            bytecode::Constant::Code { ref code } => {
                self.new_code_object(*code.clone()).into_object()
            }
            bytecode::Constant::Tuple { ref elements } => {
                let elements = elements
                    .iter()
                    .map(|value| self.unwrap_constant(value))
                    .collect();
                self.new_tuple(elements)
            }
            bytecode::Constant::None => self.none(),
            bytecode::Constant::Ellipsis => self.ellipsis(),
        }
    }
}

impl Default for PyContext {
    fn default() -> Self {
        PyContext::new()
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
pub struct PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    pub typ: PyClassRef,
    pub dict: Option<PyDictRef>, // __dict__ member
    pub payload: T,
}

impl PyObject<dyn PyObjectPayload> {
    /// Attempt to downcast this reference to a subclass.
    ///
    /// If the downcast fails, the original ref is returned in as `Err` so
    /// another downcast can be attempted without unnecessary cloning.
    ///
    /// Note: The returned `Result` is _not_ a `PyResult`, even though the
    ///       types are compatible.
    pub fn downcast<T: PyObjectPayload>(self: Rc<Self>) -> Result<PyRef<T>, PyObjectRef> {
        if self.payload_is::<T>() {
            Ok({
                PyRef {
                    obj: self,
                    _payload: PhantomData,
                }
            })
        } else {
            Err(self)
        }
    }
}

/// A reference to a Python object.
///
/// Note that a `PyRef<T>` can only deref to a shared / immutable reference.
/// It is the payload type's responsibility to handle (possibly concurrent)
/// mutability with locks or concurrent data structures if required.
///
/// A `PyRef<T>` can be directly returned from a built-in function to handle
/// situations (such as when implementing in-place methods such as `__iadd__`)
/// where a reference to the same object must be returned.
#[derive(Debug)]
pub struct PyRef<T> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<T>,
}

impl<T> Clone for PyRef<T> {
    fn clone(&self) -> Self {
        Self {
            obj: self.obj.clone(),
            _payload: PhantomData,
        }
    }
}

impl<T: PyValue> PyRef<T> {
    pub fn as_object(&self) -> &PyObjectRef {
        &self.obj
    }

    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }

    pub fn typ(&self) -> PyClassRef {
        PyRef {
            obj: self.obj.class().into_object(),
            _payload: PhantomData,
        }
    }
}

impl<T> Deref for PyRef<T>
where
    T: PyValue,
{
    type Target = T;

    fn deref(&self) -> &T {
        self.obj.payload().expect("unexpected payload for type")
    }
}

impl<T> TryFromObject for PyRef<T>
where
    T: PyValue,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if objtype::isinstance(&obj, &T::class(vm)) {
            Ok(PyRef {
                obj,
                _payload: PhantomData,
            })
        } else {
            let class = T::class(vm);
            let expected_type = vm.to_pystr(&class)?;
            let actual_type = vm.to_pystr(&obj.class())?;
            Err(vm.new_type_error(format!(
                "Expected type {}, not {}",
                expected_type, actual_type,
            )))
        }
    }
}

impl<T> IntoPyObject for PyRef<T> {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self.obj)
    }
}

impl<'a, T: PyValue> From<&'a PyRef<T>> for &'a PyObjectRef {
    fn from(obj: &'a PyRef<T>) -> Self {
        obj.as_object()
    }
}

impl<T: PyValue> From<PyRef<T>> for PyObjectRef {
    fn from(obj: PyRef<T>) -> Self {
        obj.into_object()
    }
}

impl<T: fmt::Display> fmt::Display for PyRef<T>
where
    T: PyValue + fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value: &T = self.obj.payload().expect("unexpected payload for type");
        fmt::Display::fmt(value, f)
    }
}

#[derive(Clone)]
pub struct PyCallable {
    obj: PyObjectRef,
}

impl PyCallable {
    #[inline]
    pub fn invoke(&self, args: impl Into<PyFuncArgs>, vm: &VirtualMachine) -> PyResult {
        vm.invoke(self.obj.clone(), args)
    }

    #[inline]
    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }
}

impl TryFromObject for PyCallable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.is_callable(&obj) {
            Ok(PyCallable { obj })
        } else {
            Err(vm.new_type_error(format!("'{}' object is not callable", obj.class().name)))
        }
    }
}

impl IntoPyObject for PyCallable {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self.into_object())
    }
}

pub trait IdProtocol {
    fn get_id(&self) -> usize;
    fn is<T>(&self, other: &T) -> bool
    where
        T: IdProtocol,
    {
        self.get_id() == other.get_id()
    }
}

#[derive(Debug)]
enum Never {}

impl PyValue for Never {
    fn class(_vm: &VirtualMachine) -> PyClassRef {
        unreachable!()
    }
}

impl<T: ?Sized + PyObjectPayload> IdProtocol for PyObject<T> {
    fn get_id(&self) -> usize {
        self as *const _ as *const PyObject<Never> as usize
    }
}

impl<T: ?Sized + IdProtocol> IdProtocol for Rc<T> {
    fn get_id(&self) -> usize {
        (**self).get_id()
    }
}

impl<T: PyObjectPayload> IdProtocol for PyRef<T> {
    fn get_id(&self) -> usize {
        self.obj.get_id()
    }
}

pub trait TypeProtocol {
    fn class(&self) -> PyClassRef;
}

impl TypeProtocol for PyObjectRef {
    fn class(&self) -> PyClassRef {
        (**self).class()
    }
}

impl<T> TypeProtocol for PyObject<T>
where
    T: ?Sized + PyObjectPayload,
{
    fn class(&self) -> PyClassRef {
        self.typ.clone()
    }
}

impl<T> TypeProtocol for PyRef<T> {
    fn class(&self) -> PyClassRef {
        self.obj.typ.clone()
    }
}

pub trait ItemProtocol {
    fn get_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult;
    fn set_item<T: IntoPyObject>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult;
    fn del_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult;
    fn get_item_option<T: IntoPyObject>(
        &self,
        key: T,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        match self.get_item(key, vm) {
            Ok(value) => Ok(Some(value)),
            Err(exc) => {
                if objtype::isinstance(&exc, &vm.ctx.exceptions.key_error) {
                    Ok(None)
                } else {
                    Err(exc)
                }
            }
        }
    }
}

impl ItemProtocol for PyObjectRef {
    fn get_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__getitem__", key.into_pyobject(vm)?)
    }

    fn set_item<T: IntoPyObject>(
        &self,
        key: T,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        vm.call_method(self, "__setitem__", vec![key.into_pyobject(vm)?, value])
    }

    fn del_item<T: IntoPyObject>(&self, key: T, vm: &VirtualMachine) -> PyResult {
        vm.call_method(self, "__delitem__", key.into_pyobject(vm)?)
    }
}

pub trait BufferProtocol {
    fn readonly(&self) -> bool;
}

impl BufferProtocol for PyObjectRef {
    fn readonly(&self) -> bool {
        match self.class().name.as_str() {
            "bytes" => false,
            "bytearray" | "memoryview" => true,
            _ => panic!("Bytes-Like type expected not {:?}", self),
        }
    }
}

impl fmt::Debug for PyObject<dyn PyObjectPayload> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObj {:?}]", &self.payload)
    }
}

/// An iterable Python object.
///
/// `PyIterable` implements `FromArgs` so that a built-in function can accept
/// an object that is required to conform to the Python iterator protocol.
///
/// PyIterable can optionally perform type checking and conversions on iterated
/// objects using a generic type parameter that implements `TryFromObject`.
pub struct PyIterable<T = PyObjectRef> {
    method: PyObjectRef,
    _item: std::marker::PhantomData<T>,
}

impl<T> PyIterable<T> {
    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterator<'a, T>> {
        let iter_obj = vm.invoke(
            self.method.clone(),
            PyFuncArgs {
                args: vec![],
                kwargs: IndexMap::new(),
            },
        )?;

        Ok(PyIterator {
            vm,
            obj: iter_obj,
            _item: std::marker::PhantomData,
        })
    }
}

pub struct PyIterator<'a, T> {
    vm: &'a VirtualMachine,
    obj: PyObjectRef,
    _item: std::marker::PhantomData<T>,
}

impl<'a, T> Iterator for PyIterator<'a, T>
where
    T: TryFromObject,
{
    type Item = PyResult<T>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.vm.call_method(&self.obj, "__next__", vec![]) {
            Ok(value) => Some(T::try_from_object(self.vm, value)),
            Err(err) => {
                if objtype::isinstance(&err, &self.vm.ctx.exceptions.stop_iteration) {
                    None
                } else {
                    Some(Err(err))
                }
            }
        }
    }
}

impl<T> TryFromObject for PyIterable<T>
where
    T: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if let Some(method_or_err) = vm.get_method(obj.clone(), "__iter__") {
            let method = method_or_err?;
            Ok(PyIterable {
                method,
                _item: std::marker::PhantomData,
            })
        } else {
            vm.get_method_or_type_error(obj.clone(), "__getitem__", || {
                format!("'{}' object is not iterable", obj.class().name)
            })?;
            Self::try_from_object(
                vm,
                objiter::PySequenceIterator {
                    position: Cell::new(0),
                    obj: obj.clone(),
                    reversed: false,
                }
                .into_ref(vm)
                .into_object(),
            )
        }
    }
}

impl TryFromObject for PyObjectRef {
    fn try_from_object(_vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(obj)
    }
}

impl<T: TryFromObject> TryFromObject for Option<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.get_none().is(&obj) {
            Ok(None)
        } else {
            T::try_from_object(vm, obj).map(Some)
        }
    }
}

/// Allows coercion of a types into PyRefs, so that we can write functions that can take
/// refs, pyobject refs or basic types.
pub trait TryIntoRef<T> {
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<T>>;
}

impl<T> TryIntoRef<T> for PyRef<T> {
    fn try_into_ref(self, _vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        Ok(self)
    }
}

impl<T> TryIntoRef<T> for PyObjectRef
where
    T: PyValue,
{
    fn try_into_ref(self, vm: &VirtualMachine) -> PyResult<PyRef<T>> {
        TryFromObject::try_from_object(vm, self)
    }
}

/// Implemented by any type that can be created from a Python object.
///
/// Any type that implements `TryFromObject` is automatically `FromArgs`, and
/// so can be accepted as a argument to a built-in function.
pub trait TryFromObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
}

/// Implemented by any type that can be returned from a built-in Python function.
///
/// `IntoPyObject` has a blanket implementation for any built-in object payload,
/// and should be implemented by many primitive Rust types, allowing a built-in
/// function to simply return a `bool` or a `usize` for example.
pub trait IntoPyObject {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult;
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, _vm: &VirtualMachine) -> PyResult {
        Ok(self)
    }
}

impl<T> IntoPyObject for PyResult<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        self.and_then(|res| T::into_pyobject(res, vm))
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyValue + Sized,
{
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(PyObject::new(self, T::class(vm), None))
    }
}

impl<T> PyObject<T>
where
    T: Sized + PyObjectPayload,
{
    #[allow(clippy::new_ret_no_self)]
    pub fn new(payload: T, typ: PyClassRef, dict: Option<PyDictRef>) -> PyObjectRef {
        PyObject { typ, dict, payload }.into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(self)
    }
}

impl PyObject<dyn PyObjectPayload> {
    #[inline]
    pub fn payload<T: PyObjectPayload>(&self) -> Option<&T> {
        self.payload.as_any().downcast_ref()
    }

    #[inline]
    pub fn payload_is<T: PyObjectPayload>(&self) -> bool {
        self.payload.as_any().is::<T>()
    }
}

pub trait PyValue: fmt::Debug + Sized + 'static {
    const HAVE_DICT: bool = false;

    fn class(vm: &VirtualMachine) -> PyClassRef;

    fn into_ref(self, vm: &VirtualMachine) -> PyRef<Self> {
        PyRef {
            obj: PyObject::new(self, Self::class(vm), None),
            _payload: PhantomData,
        }
    }

    fn into_ref_with_type(self, vm: &VirtualMachine, cls: PyClassRef) -> PyResult<PyRef<Self>> {
        let class = Self::class(vm);
        if objtype::issubclass(&cls, &class) {
            let dict = if !Self::HAVE_DICT && cls.is(&class) {
                None
            } else {
                Some(vm.ctx.new_dict())
            };
            Ok(PyRef {
                obj: PyObject::new(self, cls, dict),
                _payload: PhantomData,
            })
        } else {
            let subtype = vm.to_pystr(&cls.obj)?;
            let basetype = vm.to_pystr(&class.obj)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }
}

pub trait PyObjectPayload: Any + fmt::Debug + 'static {
    fn as_any(&self) -> &dyn Any;
}

impl<T: PyValue + 'static> PyObjectPayload for T {
    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub enum Either<A, B> {
    A(A),
    B(B),
}

/// This allows a builtin method to accept arguments that may be one of two
/// types, raising a `TypeError` if it is neither.
///
/// # Example
///
/// ```
/// use rustpython_vm::VirtualMachine;
/// use rustpython_vm::obj::{objstr::PyStringRef, objint::PyIntRef};
/// use rustpython_vm::pyobject::Either;
///
/// fn do_something(arg: Either<PyIntRef, PyStringRef>, vm: &VirtualMachine) {
///     match arg {
///         Either::A(int)=> {
///             // do something with int
///         }
///         Either::B(string) => {
///             // do something with string
///         }
///     }
/// }
/// ```
impl<A, B> TryFromObject for Either<PyRef<A>, PyRef<B>>
where
    A: PyValue,
    B: PyValue,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast::<A>()
            .map(Either::A)
            .or_else(|obj| obj.clone().downcast::<B>().map(Either::B))
            .map_err(|obj| {
                vm.new_type_error(format!(
                    "must be {} or {}, not {}",
                    A::class(vm),
                    B::class(vm),
                    obj.class()
                ))
            })
    }
}

pub trait PyClassDef {
    const NAME: &'static str;
    const DOC: Option<&'static str> = None;
}

impl<T> PyClassDef for PyRef<T>
where
    T: PyClassDef,
{
    const NAME: &'static str = T::NAME;
    const DOC: Option<&'static str> = T::DOC;
}

pub trait PyClassImpl: PyClassDef {
    fn impl_extend_class(ctx: &PyContext, class: &PyClassRef);

    fn extend_class(ctx: &PyContext, class: &PyClassRef) {
        Self::impl_extend_class(ctx, class);
        if let Some(doc) = Self::DOC {
            class.set_str_attr("__doc__", ctx.new_str(doc.into()));
        }
    }

    fn make_class(ctx: &PyContext) -> PyClassRef {
        Self::make_class_with_base(ctx, ctx.object())
    }

    fn make_class_with_base(ctx: &PyContext, base: PyClassRef) -> PyClassRef {
        let py_class = ctx.new_class(Self::NAME, base);
        Self::extend_class(ctx, &py_class);
        py_class
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_type() {
        // TODO: Write this test
        PyContext::new();
    }
}
