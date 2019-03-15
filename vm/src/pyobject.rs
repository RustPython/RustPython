use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::iter;
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, RangeInclusive};
use std::ptr;
use std::rc::Rc;

use num_bigint::BigInt;
use num_complex::Complex64;
use num_traits::{One, Zero};

use crate::bytecode;
use crate::exceptions;
use crate::frame::{Frame, Scope};
use crate::obj::objbool;
use crate::obj::objbuiltinfunc::PyBuiltinFunction;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objclassmethod;
use crate::obj::objcode;
use crate::obj::objcomplex::{self, PyComplex};
use crate::obj::objdict::{self, PyDict};
use crate::obj::objellipsis;
use crate::obj::objenumerate;
use crate::obj::objfilter;
use crate::obj::objfloat::{self, PyFloat};
use crate::obj::objframe;
use crate::obj::objfunction::{self, PyFunction, PyMethod};
use crate::obj::objgenerator;
use crate::obj::objint::{self, PyInt};
use crate::obj::objiter;
use crate::obj::objlist::{self, PyList};
use crate::obj::objmap;
use crate::obj::objmemory;
use crate::obj::objmodule::{self, PyModule};
use crate::obj::objnone;
use crate::obj::objobject;
use crate::obj::objproperty;
use crate::obj::objrange;
use crate::obj::objset::{self, PySet};
use crate::obj::objslice;
use crate::obj::objstaticmethod;
use crate::obj::objstr;
use crate::obj::objsuper;
use crate::obj::objtuple::{self, PyTuple};
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::obj::objweakref;
use crate::obj::objzip;
use crate::vm::VirtualMachine;

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
pub type PyObjectRef = Rc<PyObject>;

/// Use this type for function which return a python object or and exception.
/// Both the python object and the python exception are `PyObjectRef` types
/// since exceptions are also python objects.
pub type PyResult<T = PyObjectRef> = Result<T, PyObjectRef>; // A valid value, or an exception

/// For attributes we do not use a dict, but a hashmap. This is probably
/// faster, unordered, and only supports strings as keys.
pub type PyAttributes = HashMap<String, PyObjectRef>;

impl fmt::Display for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::TypeProtocol;
        if let Some(PyClass { ref name, .. }) = self.payload::<PyClass>() {
            let type_name = objtype::get_type_name(&self.typ());
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
        write!(f, "'{}' object", objtype::get_type_name(&self.typ()))
    }
}

#[derive(Debug)]
pub struct PyContext {
    pub bytes_type: PyObjectRef,
    pub bytearray_type: PyObjectRef,
    pub bool_type: PyObjectRef,
    pub classmethod_type: PyObjectRef,
    pub code_type: PyObjectRef,
    pub dict_type: PyObjectRef,
    pub ellipsis_type: PyObjectRef,
    pub enumerate_type: PyObjectRef,
    pub filter_type: PyObjectRef,
    pub float_type: PyObjectRef,
    pub frame_type: PyObjectRef,
    pub frozenset_type: PyObjectRef,
    pub generator_type: PyObjectRef,
    pub int_type: PyObjectRef,
    pub iter_type: PyObjectRef,
    pub complex_type: PyObjectRef,
    pub true_value: PyObjectRef,
    pub false_value: PyObjectRef,
    pub list_type: PyObjectRef,
    pub map_type: PyObjectRef,
    pub memoryview_type: PyObjectRef,
    pub none: PyObjectRef,
    pub ellipsis: PyObjectRef,
    pub not_implemented: PyObjectRef,
    pub tuple_type: PyObjectRef,
    pub staticmethod_type: PyObjectRef,
    pub super_type: PyObjectRef,
    pub str_type: PyObjectRef,
    pub range_type: PyObjectRef,
    pub slice_type: PyObjectRef,
    pub type_type: PyObjectRef,
    pub zip_type: PyObjectRef,
    pub function_type: PyObjectRef,
    pub builtin_function_or_method_type: PyObjectRef,
    pub property_type: PyObjectRef,
    pub readonly_property_type: PyObjectRef,
    pub module_type: PyObjectRef,
    pub bound_method_type: PyObjectRef,
    pub weakref_type: PyObjectRef,
    pub object: PyObjectRef,
    pub exceptions: exceptions::ExceptionZoo,
    types: RefCell<HashMap<TypeId, PyClassRef>>
}

pub fn create_type(name: &str, type_type: &PyObjectRef, base: &PyObjectRef) -> PyObjectRef {
    let dict = PyAttributes::new();
    objtype::new(
        type_type.clone(),
        name,
        vec![FromPyObjectRef::from_pyobj(base)],
        dict,
    )
    .unwrap()
}

#[derive(Debug)]
pub struct PyNotImplemented;

impl PyValue for PyNotImplemented {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented().typ()
    }
}

#[derive(Debug)]
pub struct PyEllipsis;

impl PyValue for PyEllipsis {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.ellipsis_type.clone()
    }
}

fn init_type_hierarchy() -> (PyObjectRef, PyObjectRef) {
    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    unsafe {
        let object_type = PyObject {
            typ: mem::uninitialized(), // !
            dict: Some(RefCell::new(PyAttributes::new())),
            payload: Box::new(PyClass {
                name: String::from("object"),
                mro: vec![],
            }),
        }
        .into_ref();

        let type_type = PyObject {
            typ: mem::uninitialized(), // !
            dict: Some(RefCell::new(PyAttributes::new())),
            payload: Box::new(PyClass {
                name: String::from("type"),
                mro: vec![FromPyObjectRef::from_pyobj(&object_type)],
            }),
        }
        .into_ref();

        let object_type_ptr = PyObjectRef::into_raw(object_type.clone()) as *mut PyObject;
        let type_type_ptr = PyObjectRef::into_raw(type_type.clone()) as *mut PyObject;
        ptr::write(&mut (*object_type_ptr).typ, type_type.clone());
        ptr::write(&mut (*type_type_ptr).typ, type_type.clone());

        (type_type, object_type)
    }
}

// Basic objects:
impl PyContext {
    pub fn new() -> Self {
        let (type_type, object_type) = init_type_hierarchy();

        let dict_type = create_type("dict", &type_type, &object_type);
        let module_type = create_type("module", &type_type, &object_type);
        let classmethod_type = create_type("classmethod", &type_type, &object_type);
        let staticmethod_type = create_type("staticmethod", &type_type, &object_type);
        let function_type = create_type("function", &type_type, &object_type);
        let builtin_function_or_method_type =
            create_type("builtin_function_or_method", &type_type, &object_type);
        let property_type = create_type("property", &type_type, &object_type);
        let readonly_property_type = create_type("readonly_property", &type_type, &object_type);
        let super_type = create_type("super", &type_type, &object_type);
        let weakref_type = create_type("ref", &type_type, &object_type);
        let generator_type = create_type("generator", &type_type, &object_type);
        let bound_method_type = create_type("method", &type_type, &object_type);
        let str_type = create_type("str", &type_type, &object_type);
        let list_type = create_type("list", &type_type, &object_type);
        let frozenset_type = create_type("frozenset", &type_type, &object_type);
        let int_type = create_type("int", &type_type, &object_type);
        let float_type = create_type("float", &type_type, &object_type);
        let frame_type = create_type("frame", &type_type, &object_type);
        let complex_type = create_type("complex", &type_type, &object_type);
        let bytes_type = create_type("bytes", &type_type, &object_type);
        let bytearray_type = create_type("bytearray", &type_type, &object_type);
        let tuple_type = create_type("tuple", &type_type, &object_type);
        let iter_type = create_type("iter", &type_type, &object_type);
        let ellipsis_type = create_type("EllipsisType", &type_type, &object_type);
        let enumerate_type = create_type("enumerate", &type_type, &object_type);
        let filter_type = create_type("filter", &type_type, &object_type);
        let map_type = create_type("map", &type_type, &object_type);
        let zip_type = create_type("zip", &type_type, &object_type);
        let bool_type = create_type("bool", &type_type, &int_type);
        let memoryview_type = create_type("memoryview", &type_type, &object_type);
        let code_type = create_type("code", &type_type, &int_type);
        let range_type = create_type("range", &type_type, &object_type);
        let slice_type = create_type("slice", &type_type, &object_type);
        let exceptions = exceptions::ExceptionZoo::new(&type_type, &object_type);

        let none = PyObject::new(
            objnone::PyNone,
            create_type("NoneType", &type_type, &object_type),
        );

        let ellipsis = PyObject::new(PyEllipsis, ellipsis_type.clone());

        let not_implemented = PyObject::new(
            PyNotImplemented,
            create_type("NotImplementedType", &type_type, &object_type),
        );

        let true_value = PyObject::new(PyInt::new(BigInt::one()), bool_type.clone());
        let false_value = PyObject::new(PyInt::new(BigInt::zero()), bool_type.clone());
        let context = PyContext {
            bool_type,
            memoryview_type,
            bytearray_type,
            bytes_type,
            code_type,
            complex_type,
            classmethod_type,
            int_type,
            float_type,
            frame_type,
            staticmethod_type,
            list_type,
            frozenset_type,
            true_value,
            false_value,
            tuple_type,
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
            slice_type,
            object: object_type,
            function_type,
            builtin_function_or_method_type,
            super_type,
            property_type,
            readonly_property_type,
            generator_type,
            module_type,
            bound_method_type,
            weakref_type,
            type_type,
            exceptions,
            types: Default::default()
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
        objnone::init(&context);
        objmodule::init(&context);
        exceptions::init(&context);
        context
    }

    pub fn get_type<T:PyImmutableClass + 'static>(&self) -> Option<PyClassRef> {
        self.types.borrow().get(&TypeId::of::<T>()).cloned()
    }

    pub fn set_type<T:PyImmutableClass>(&self, cls: PyClassRef) {
        let mut map = self.types.borrow_mut();
        let entry = map.entry(TypeId::of::<T>());
        if let Entry::Vacant(entry) = entry {
            entry.insert(cls);
        } else {
            panic!("Can't overwrite type in PyContext");
        }
    }

    pub fn bytearray_type(&self) -> PyObjectRef {
        self.bytearray_type.clone()
    }

    pub fn bytes_type(&self) -> PyObjectRef {
        self.bytes_type.clone()
    }

    pub fn code_type(&self) -> PyObjectRef {
        self.code_type.clone()
    }

    pub fn complex_type(&self) -> PyObjectRef {
        self.complex_type.clone()
    }

    pub fn dict_type(&self) -> PyObjectRef {
        self.dict_type.clone()
    }

    pub fn float_type(&self) -> PyObjectRef {
        self.float_type.clone()
    }

    pub fn frame_type(&self) -> PyObjectRef {
        self.frame_type.clone()
    }

    pub fn int_type(&self) -> PyObjectRef {
        self.int_type.clone()
    }

    pub fn list_type(&self) -> PyObjectRef {
        self.list_type.clone()
    }

    pub fn module_type(&self) -> PyObjectRef {
        self.module_type.clone()
    }

    pub fn range_type(&self) -> PyObjectRef {
        self.range_type.clone()
    }

    pub fn slice_type(&self) -> PyObjectRef {
        self.slice_type.clone()
    }

    pub fn frozenset_type(&self) -> PyObjectRef {
        self.frozenset_type.clone()
    }

    pub fn bool_type(&self) -> PyObjectRef {
        self.bool_type.clone()
    }

    pub fn memoryview_type(&self) -> PyObjectRef {
        self.memoryview_type.clone()
    }

    pub fn tuple_type(&self) -> PyObjectRef {
        self.tuple_type.clone()
    }

    pub fn iter_type(&self) -> PyObjectRef {
        self.iter_type.clone()
    }

    pub fn enumerate_type(&self) -> PyObjectRef {
        self.enumerate_type.clone()
    }

    pub fn filter_type(&self) -> PyObjectRef {
        self.filter_type.clone()
    }

    pub fn map_type(&self) -> PyObjectRef {
        self.map_type.clone()
    }

    pub fn zip_type(&self) -> PyObjectRef {
        self.zip_type.clone()
    }

    pub fn str_type(&self) -> PyObjectRef {
        self.str_type.clone()
    }

    pub fn super_type(&self) -> PyObjectRef {
        self.super_type.clone()
    }

    pub fn function_type(&self) -> PyObjectRef {
        self.function_type.clone()
    }

    pub fn builtin_function_or_method_type(&self) -> PyObjectRef {
        self.builtin_function_or_method_type.clone()
    }

    pub fn property_type(&self) -> PyObjectRef {
        self.property_type.clone()
    }

    pub fn readonly_property_type(&self) -> PyObjectRef {
        self.readonly_property_type.clone()
    }

    pub fn classmethod_type(&self) -> PyObjectRef {
        self.classmethod_type.clone()
    }

    pub fn staticmethod_type(&self) -> PyObjectRef {
        self.staticmethod_type.clone()
    }

    pub fn generator_type(&self) -> PyObjectRef {
        self.generator_type.clone()
    }

    pub fn bound_method_type(&self) -> PyObjectRef {
        self.bound_method_type.clone()
    }

    pub fn weakref_type(&self) -> PyObjectRef {
        self.weakref_type.clone()
    }

    pub fn type_type(&self) -> PyObjectRef {
        self.type_type.clone()
    }

    pub fn none(&self) -> PyObjectRef {
        self.none.clone()
    }

    pub fn ellipsis(&self) -> PyObjectRef {
        self.ellipsis.clone()
    }

    pub fn not_implemented(&self) -> PyObjectRef {
        self.not_implemented.clone()
    }

    pub fn object(&self) -> PyObjectRef {
        self.object.clone()
    }

    pub fn new_object(&self) -> PyObjectRef {
        self.new_instance(self.object(), None)
    }

    pub fn new_int<T: Into<BigInt>>(&self, i: T) -> PyObjectRef {
        PyObject::new(PyInt::new(i), self.int_type())
    }

    pub fn new_float(&self, value: f64) -> PyObjectRef {
        PyObject::new(PyFloat::from(value), self.float_type())
    }

    pub fn new_complex(&self, value: Complex64) -> PyObjectRef {
        PyObject::new(PyComplex::from(value), self.complex_type())
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(objstr::PyString { value: s }, self.str_type())
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(objbytes::PyBytes::new(data), self.bytes_type())
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(objbytearray::PyByteArray::new(data), self.bytearray_type())
    }

    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        if b {
            self.true_value.clone()
        } else {
            self.false_value.clone()
        }
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyTuple::from(elements), self.tuple_type())
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyList::from(elements), self.list_type())
    }

    pub fn new_set(&self) -> PyObjectRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the objset code itself.
        PySet::default().into_ref_with_context(self).into_object()
    }

    pub fn new_dict(&self) -> PyObjectRef {
        PyObject::new(PyDict::default(), self.dict_type())
    }

    pub fn new_class(&self, name: &str, base: PyObjectRef) -> PyObjectRef {
        objtype::new(
            self.type_type(),
            name,
            vec![FromPyObjectRef::from_pyobj(&base)],
            PyAttributes::new(),
        )
        .unwrap()
    }

    pub fn new_scope(&self) -> Scope {
        Scope::new(None, self.new_dict())
    }

    pub fn new_module(&self, name: &str, dict: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyModule {
                name: name.to_string(),
                dict,
            },
            self.module_type.clone(),
        )
    }

    pub fn new_rustfunc<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyBuiltinFunction::new(f.into_func()),
            self.builtin_function_or_method_type(),
        )
    }

    pub fn new_frame(&self, code: PyObjectRef, scope: Scope) -> PyObjectRef {
        PyObject::new(Frame::new(code, scope), self.frame_type())
    }

    pub fn new_property<F, I, V>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<I, V>,
    {
        PropertyBuilder::new(self).add_getter(f).create()
    }

    pub fn new_code_object(&self, code: bytecode::CodeObject) -> PyObjectRef {
        PyObject::new(objcode::PyCode::new(code), self.code_type())
    }

    pub fn new_function(
        &self,
        code_obj: PyObjectRef,
        scope: Scope,
        defaults: PyObjectRef,
    ) -> PyObjectRef {
        PyObject::new(
            PyFunction::new(code_obj, scope, defaults),
            self.function_type(),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(PyMethod::new(object, function), self.bound_method_type())
    }

    pub fn new_instance(&self, class: PyObjectRef, dict: Option<PyAttributes>) -> PyObjectRef {
        let dict = dict.unwrap_or_default();
        PyObject {
            typ: class,
            dict: Some(RefCell::new(dict)),
            payload: Box::new(objobject::PyInstance),
        }
        .into_ref()
    }

    // Item set/get:
    pub fn set_item(&self, obj: &PyObjectRef, key: &str, v: PyObjectRef) {
        if let Some(dict) = obj.payload::<PyDict>() {
            let key = self.new_str(key.to_string());
            objdict::set_item_in_content(&mut dict.entries.borrow_mut(), &key, &v);
        } else {
            unimplemented!()
        };
    }

    pub fn get_attr(&self, obj: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
        // This does not need to be on the PyContext.
        // We do not require to make a new key as string for this function
        // (yet)...
        obj.get_attr(attr_name)
    }

    pub fn set_attr<T:Into<PyObjectRef>>(&self, obj: &PyObjectRef, attr_name: &str, value: T) {
        if let Some(PyModule { ref dict, .. }) = obj.payload::<PyModule>() {
            dict.set_item(self, attr_name, value.into())
        } else if let Some(ref dict) = obj.dict {
            dict.borrow_mut().insert(attr_name.to_string(), value.into());
        } else {
            unimplemented!("set_attr unimplemented for: {:?}", obj);
        };
    }

    pub fn unwrap_constant(&mut self, value: &bytecode::Constant) -> PyObjectRef {
        match *value {
            bytecode::Constant::Integer { ref value } => self.new_int(value.clone()),
            bytecode::Constant::Float { ref value } => self.new_float(*value),
            bytecode::Constant::Complex { ref value } => self.new_complex(*value),
            bytecode::Constant::String { ref value } => self.new_str(value.clone()),
            bytecode::Constant::Bytes { ref value } => self.new_bytes(value.clone()),
            bytecode::Constant::Boolean { ref value } => self.new_bool(value.clone()),
            bytecode::Constant::Code { ref code } => self.new_code_object(*code.clone()),
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
pub struct PyObject {
    pub typ: PyObjectRef,
    pub dict: Option<RefCell<PyAttributes>>, // __dict__ member
    pub payload: Box<dyn PyObjectPayload>,
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
#[derive(Clone, Debug)]
pub struct PyRef<T> {
    // invariant: this obj must always have payload of type T
    obj: PyObjectRef,
    _payload: PhantomData<T>,
}

impl<T: PyValue> PyRef<T> {
    pub fn new(vm: &mut VirtualMachine, payload: T) -> Self {
        PyRef {
            obj: PyObject::new(payload, T::class(vm)),
            _payload: PhantomData,
        }
    }

    pub fn new_with_type(vm: &mut VirtualMachine, payload: T, cls: PyClassRef) -> PyResult<Self> {
        let class = T::class(vm);
        if objtype::issubclass(&cls.obj, &class) {
            Ok(PyRef {
                obj: PyObject::new(payload, cls.obj),
                _payload: PhantomData,
            })
        } else {
            let subtype = vm.to_pystr(&cls.obj)?;
            let basetype = vm.to_pystr(&class)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }

    pub fn as_object(&self) -> &PyObjectRef {
        &self.obj
    }
    pub fn into_object(self) -> PyObjectRef {
        self.obj
    }

    pub fn typ(&self) -> PyClassRef {
        PyRef {
            obj: self.obj.typ(),
            _payload: PhantomData,
        }
    }

    // Temporary escape hatch, until more things are using PyRef instead of PyObjectRef
    // This is used in contexts were we don't have access to a VM
    pub unsafe fn from_object_unchecked(obj: PyObjectRef) -> Self {
        PyRef {
            obj,
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
    fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if objtype::isinstance(&obj, &T::class(vm)) {
            Ok(PyRef {
                obj,
                _payload: PhantomData,
            })
        } else {
            let class = T::class(vm);
            let expected_type = vm.to_pystr(&class)?;
            let actual_type = vm.to_pystr(&obj.typ())?;
            Err(vm.new_type_error(format!(
                "Expected type {}, not {}",
                expected_type, actual_type,
            )))
        }
    }
}

impl<T> IntoPyObject for PyRef<T> {
    fn into_pyobject(self, _vm: &mut VirtualMachine) -> PyResult {
        Ok(self.obj)
    }
}

impl<T> Into<PyObjectRef> for PyRef<T> {
    fn into(self) -> PyObjectRef {
        self.obj
    }
}

impl<T> fmt::Display for PyRef<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.obj.fmt(f)
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

impl IdProtocol for PyObjectRef {
    fn get_id(&self) -> usize {
        &*self as &PyObject as *const PyObject as usize
    }
}

pub trait FromPyObjectRef {
    fn from_pyobj(obj: &PyObjectRef) -> Self;
}

pub trait TypeProtocol {
    fn typ(&self) -> PyObjectRef {
        self.type_ref().clone()
    }
    fn type_pyref(&self) -> PyClassRef {
        FromPyObjectRef::from_pyobj(self.type_ref())
    }
    fn type_ref(&self) -> &PyObjectRef;
}

impl TypeProtocol for PyObjectRef {
    fn type_ref(&self) -> &PyObjectRef {
        (**self).type_ref()
    }
}

impl TypeProtocol for PyObject {
    fn type_ref(&self) -> &PyObjectRef {
        &self.typ
    }
}

pub trait AttributeProtocol {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef>;
    fn has_attr(&self, attr_name: &str) -> bool;
}

fn class_get_item(class: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
    if let Some(ref dict) = class.dict {
        dict.borrow().get(attr_name).cloned()
    } else {
        panic!("Only classes should be in MRO!");
    }
}

fn class_has_item(class: &PyObjectRef, attr_name: &str) -> bool {
    if let Some(ref dict) = class.dict {
        dict.borrow().contains_key(attr_name)
    } else {
        panic!("Only classes should be in MRO!");
    }
}

impl AttributeProtocol for PyObjectRef {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        if let Some(PyClass { ref mro, .. }) = self.payload::<PyClass>() {
            if let Some(item) = class_get_item(self, attr_name) {
                return Some(item);
            }
            for class in mro {
                if let Some(item) = class_get_item(class.as_object(), attr_name) {
                    return Some(item);
                }
            }
            return None;
        }

        if let Some(PyModule { ref dict, .. }) = self.payload::<PyModule>() {
            return dict.get_item(attr_name);
        }

        if let Some(ref dict) = self.dict {
            dict.borrow().get(attr_name).cloned()
        } else {
            None
        }
    }

    fn has_attr(&self, attr_name: &str) -> bool {
        if let Some(PyClass { ref mro, .. }) = self.payload::<PyClass>() {
            return class_has_item(self, attr_name)
                || mro.iter().any(|d| class_has_item(d.as_object(), attr_name));
        }

        if let Some(PyModule { ref dict, .. }) = self.payload::<PyModule>() {
            return dict.contains_key(attr_name);
        }

        if let Some(ref dict) = self.dict {
            dict.borrow().contains_key(attr_name)
        } else {
            false
        }
    }
}

pub trait DictProtocol {
    fn contains_key(&self, k: &str) -> bool;
    fn get_item(&self, k: &str) -> Option<PyObjectRef>;
    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)>;
    fn set_item(&self, ctx: &PyContext, key: &str, v: PyObjectRef);
    fn del_item(&self, key: &str);
}

impl DictProtocol for PyObjectRef {
    fn contains_key(&self, k: &str) -> bool {
        if let Some(dict) = self.payload::<PyDict>() {
            objdict::content_contains_key_str(&dict.entries.borrow(), k)
        } else {
            unimplemented!()
        }
    }

    fn get_item(&self, k: &str) -> Option<PyObjectRef> {
        if let Some(dict) = self.payload::<PyDict>() {
            objdict::content_get_key_str(&dict.entries.borrow(), k)
        } else if let Some(PyModule { ref dict, .. }) = self.payload::<PyModule>() {
            dict.get_item(k)
        } else {
            panic!("TODO {:?}", k)
        }
    }

    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        if let Some(_) = self.payload::<PyDict>() {
            objdict::get_key_value_pairs(self)
        } else if let Some(PyModule { ref dict, .. }) = self.payload::<PyModule>() {
            dict.get_key_value_pairs()
        } else {
            panic!("TODO")
        }
    }

    // Item set/get:
    fn set_item(&self, ctx: &PyContext, key: &str, v: PyObjectRef) {
        if let Some(dict) = self.payload::<PyDict>() {
            let key = ctx.new_str(key.to_string());
            objdict::set_item_in_content(&mut dict.entries.borrow_mut(), &key, &v);
        } else if let Some(PyModule { ref dict, .. }) = self.payload::<PyModule>() {
            dict.set_item(ctx, key, v);
        } else {
            panic!("TODO {:?}", self);
        }
    }

    fn del_item(&self, key: &str) {
        let mut elements = objdict::get_mut_elements(self);
        elements.remove(key).unwrap();
    }
}

pub trait BufferProtocol {
    fn readonly(&self) -> bool;
}

impl BufferProtocol for PyObjectRef {
    fn readonly(&self) -> bool {
        match objtype::get_type_name(&self.typ()).as_ref() {
            "bytes" => false,
            "bytearray" | "memoryview" => true,
            _ => panic!("Bytes-Like type expected not {:?}", self),
        }
    }
}

impl fmt::Debug for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObj {:?}]", self.payload)
    }
}

/// The `PyFuncArgs` struct is one of the most used structs then creating
/// a rust function that can be called from python. It holds both positional
/// arguments, as well as keyword arguments passed to the function.
#[derive(Debug, Default, Clone)]
pub struct PyFuncArgs {
    pub args: Vec<PyObjectRef>,
    pub kwargs: Vec<(String, PyObjectRef)>,
}

/// Conversion from vector of python objects to function arguments.
impl From<Vec<PyObjectRef>> for PyFuncArgs {
    fn from(args: Vec<PyObjectRef>) -> Self {
        PyFuncArgs {
            args: args,
            kwargs: vec![],
        }
    }
}

impl From<PyObjectRef> for PyFuncArgs {
    fn from(arg: PyObjectRef) -> Self {
        PyFuncArgs {
            args: vec![arg],
            kwargs: vec![],
        }
    }
}

impl PyFuncArgs {
    pub fn new(mut args: Vec<PyObjectRef>, kwarg_names: Vec<String>) -> PyFuncArgs {
        let mut kwargs = vec![];
        for name in kwarg_names.iter().rev() {
            kwargs.push((name.clone(), args.pop().unwrap()));
        }
        PyFuncArgs { args, kwargs }
    }

    pub fn insert(&self, item: PyObjectRef) -> PyFuncArgs {
        let mut args = PyFuncArgs {
            args: self.args.clone(),
            kwargs: self.kwargs.clone(),
        };
        args.args.insert(0, item);
        args
    }

    pub fn shift(&mut self) -> PyObjectRef {
        self.args.remove(0)
    }

    pub fn get_kwarg(&self, key: &str, default: PyObjectRef) -> PyObjectRef {
        for (arg_name, arg_value) in self.kwargs.iter() {
            if arg_name == key {
                return arg_value.clone();
            }
        }
        default.clone()
    }

    pub fn get_optional_kwarg(&self, key: &str) -> Option<PyObjectRef> {
        for (arg_name, arg_value) in self.kwargs.iter() {
            if arg_name == key {
                return Some(arg_value.clone());
            }
        }
        None
    }

    pub fn get_optional_kwarg_with_type(
        &self,
        key: &str,
        ty: PyObjectRef,
        vm: &mut VirtualMachine,
    ) -> Result<Option<PyObjectRef>, PyObjectRef> {
        match self.get_optional_kwarg(key) {
            Some(kwarg) => {
                if objtype::isinstance(&kwarg, &ty) {
                    Ok(Some(kwarg))
                } else {
                    let expected_ty_name = vm.to_pystr(&ty)?;
                    let actual_ty_name = vm.to_pystr(&kwarg.typ())?;
                    Err(vm.new_type_error(format!(
                        "argument of type {} is required for named parameter `{}` (got: {})",
                        expected_ty_name, key, actual_ty_name
                    )))
                }
            }
            None => Ok(None),
        }
    }

    /// Serializes these arguments into an iterator starting with the positional
    /// arguments followed by keyword arguments.
    fn into_iter(self) -> impl Iterator<Item = PyArg> {
        self.args.into_iter().map(PyArg::Positional).chain(
            self.kwargs
                .into_iter()
                .map(|(name, value)| PyArg::Keyword(name, value)),
        )
    }

    /// Binds these arguments to their respective values.
    ///
    /// If there is an insufficient number of arguments, there are leftover
    /// arguments after performing the binding, or if an argument is not of
    /// the expected type, a TypeError is raised.
    ///
    /// If the given `FromArgs` includes any conversions, exceptions raised
    /// during the conversion will halt the binding and return the error.
    fn bind<T: FromArgs>(self, vm: &mut VirtualMachine) -> PyResult<T> {
        let given_args = self.args.len();
        let mut args = self.into_iter().peekable();
        let bound = match T::from_args(vm, &mut args) {
            Ok(args) => args,
            Err(ArgumentError::TooFewArgs) => {
                return Err(vm.new_type_error(format!(
                    "Expected at least {} arguments ({} given)",
                    T::arity().start(),
                    given_args,
                )));
            }
            Err(ArgumentError::Exception(ex)) => {
                return Err(ex);
            }
        };

        match args.next() {
            None => Ok(bound),
            Some(PyArg::Positional(_)) => Err(vm.new_type_error(format!(
                "Expected at most {} arguments ({} given)",
                T::arity().end(),
                given_args,
            ))),
            Some(PyArg::Keyword(name, _)) => {
                Err(vm.new_type_error(format!("Unexpected keyword argument {}", name)))
            }
        }
    }
}

/// Implemented by any type that can be accepted as a parameter to a built-in
/// function.
///
pub trait FromArgs: Sized {
    /// The range of positional arguments permitted by the function signature.
    ///
    /// Returns an empty range if not applicable.
    fn arity() -> RangeInclusive<usize> {
        0..=0
    }

    /// Extracts this item from the next argument(s).
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>;
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
    pub fn iter<'a>(&self, vm: &'a mut VirtualMachine) -> PyResult<PyIterator<'a, T>> {
        let iter_obj = vm.invoke(
            self.method.clone(),
            PyFuncArgs {
                args: vec![],
                kwargs: vec![],
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
    vm: &'a mut VirtualMachine,
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
                let stop_ex = self.vm.ctx.exceptions.stop_iteration.clone();
                if objtype::isinstance(&err, &stop_ex) {
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
    fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(PyIterable {
            method: vm.get_method(obj, "__iter__")?,
            _item: std::marker::PhantomData,
        })
    }
}

impl TryFromObject for PyObjectRef {
    fn try_from_object(_vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(obj)
    }
}

impl<T: TryFromObject> TryFromObject for Option<T> {
    fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        if vm.get_none().is(&obj) {
            Ok(None)
        } else {
            T::try_from_object(vm, obj).map(|x| Some(x))
        }
    }
}

/// A map of keyword arguments to their values.
///
/// A built-in function with a `KwArgs` parameter is analagous to a Python
/// function with `*kwargs`. All remaining keyword arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `KwArgs` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
pub struct KwArgs<T = PyObjectRef>(HashMap<String, T>);

impl<T> FromArgs for KwArgs<T>
where
    T: TryFromObject,
{
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        let mut kwargs = HashMap::new();
        while let Some(PyArg::Keyword(name, value)) = args.next() {
            kwargs.insert(name, T::try_from_object(vm, value)?);
        }
        Ok(KwArgs(kwargs))
    }
}

/// A list of positional argument values.
///
/// A built-in function with a `Args` parameter is analagous to a Python
/// function with `*args`. All remaining positional arguments are extracted
/// (and hence the function will permit an arbitrary number of them).
///
/// `Args` optionally accepts a generic type parameter to allow type checks
/// or conversions of each argument.
pub struct Args<T>(Vec<T>);

impl<T> FromArgs for Args<T>
where
    T: TryFromObject,
{
    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        let mut varargs = Vec::new();
        while let Some(PyArg::Positional(value)) = args.next() {
            varargs.push(T::try_from_object(vm, value)?);
        }
        Ok(Args(varargs))
    }
}

impl<T> FromArgs for T
where
    T: TryFromObject,
{
    fn arity() -> RangeInclusive<usize> {
        1..=1
    }

    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        if let Some(PyArg::Positional(value)) = args.next() {
            Ok(T::try_from_object(vm, value)?)
        } else {
            Err(ArgumentError::TooFewArgs)
        }
    }
}

/// An argument that may or may not be provided by the caller.
///
/// This style of argument is not possible in pure Python.
pub enum OptionalArg<T> {
    Present(T),
    Missing,
}

use self::OptionalArg::*;
use crate::obj::objproperty::PropertyBuilder;
use std::collections::hash_map::Entry;

impl<T> OptionalArg<T> {
    pub fn into_option(self) -> Option<T> {
        match self {
            Present(value) => Some(value),
            Missing => None,
        }
    }
}

impl<T> FromArgs for OptionalArg<T>
where
    T: TryFromObject,
{
    fn arity() -> RangeInclusive<usize> {
        0..=1
    }

    fn from_args<I>(
        vm: &mut VirtualMachine,
        args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        Ok(if let Some(PyArg::Positional(_)) = args.peek() {
            let value = if let Some(PyArg::Positional(value)) = args.next() {
                value
            } else {
                unreachable!()
            };
            Present(T::try_from_object(vm, value)?)
        } else {
            Missing
        })
    }
}

pub enum PyArg {
    Positional(PyObjectRef),
    Keyword(String, PyObjectRef),
}

pub enum ArgumentError {
    TooFewArgs,
    Exception(PyObjectRef),
}

impl From<PyObjectRef> for ArgumentError {
    fn from(ex: PyObjectRef) -> Self {
        ArgumentError::Exception(ex)
    }
}

/// Implemented by any type that can be created from a Python object.
///
/// Any type that implements `TryFromObject` is automatically `FromArgs`, and
/// so can be accepted as a argument to a built-in function.
pub trait TryFromObject: Sized {
    /// Attempt to convert a Python object to a value of this type.
    fn try_from_object(vm: &mut VirtualMachine, obj: PyObjectRef) -> PyResult<Self>;
}

/// Implemented by any type that can be returned from a built-in Python function.
///
/// `IntoPyObject` has a blanket implementation for any built-in object payload,
/// and should be implemented by many primitive Rust types, allowing a built-in
/// function to simply return a `bool` or a `usize` for example.
pub trait IntoPyObject {
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult;
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, _vm: &mut VirtualMachine) -> PyResult {
        Ok(self)
    }
}

impl<T> IntoPyObject for PyResult<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult {
        self.and_then(|res| T::into_pyobject(res, vm))
    }
}

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyValue + Sized,
{
    fn into_pyobject(self, vm: &mut VirtualMachine) -> PyResult {
        Ok(PyObject::new(self, T::class(vm)))
    }
}

// For functions that accept no arguments. Implemented explicitly instead of via
// macro below to avoid unused warnings.
impl FromArgs for () {
    fn from_args<I>(
        _vm: &mut VirtualMachine,
        _args: &mut iter::Peekable<I>,
    ) -> Result<Self, ArgumentError>
    where
        I: Iterator<Item = PyArg>,
    {
        Ok(())
    }
}

// A tuple of types that each implement `FromArgs` represents a sequence of
// arguments that can be bound and passed to a built-in function.
//
// Technically, a tuple can contain tuples, which can contain tuples, and so on,
// so this actually represents a tree of values to be bound from arguments, but
// in practice this is only used for the top-level parameters.
macro_rules! tuple_from_py_func_args {
    ($($T:ident),+) => {
        impl<$($T),+> FromArgs for ($($T,)+)
        where
            $($T: FromArgs),+
        {
            fn arity() -> RangeInclusive<usize> {
                let mut min = 0;
                let mut max = 0;
                $(
                    let (start, end) = $T::arity().into_inner();
                    min += start;
                    max += end;
                )+
                min..=max
            }

            fn from_args<I>(
                vm: &mut VirtualMachine,
                args: &mut iter::Peekable<I>
            ) -> Result<Self, ArgumentError>
            where
                I: Iterator<Item = PyArg>
            {
                Ok(($($T::from_args(vm, args)?,)+))
            }
        }
    };
}

// Implement `FromArgs` for up to 5-tuples, allowing built-in functions to bind
// up to 5 top-level parameters (note that `Args`, `KwArgs`, nested tuples, etc.
// count as 1, so this should actually be more than enough).
tuple_from_py_func_args!(A);
tuple_from_py_func_args!(A, B);
tuple_from_py_func_args!(A, B, C);
tuple_from_py_func_args!(A, B, C, D);
tuple_from_py_func_args!(A, B, C, D, E);

/// A built-in Python function.
pub type PyNativeFunc = Box<dyn Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult + 'static>;

/// Implemented by types that are or can generate built-in functions.
///
/// For example, any function that:
///
/// - Accepts a sequence of types that implement `FromArgs`, followed by a
///   `&mut VirtualMachine`
/// - Returns some type that implements `IntoPyObject`
///
/// will generate a `PyNativeFunc` that performs the appropriate type and arity
/// checking, any requested conversions, and then if successful call the function
/// with the bound values.
///
/// A bare `PyNativeFunc` also implements this trait, allowing the above to be
/// done manually, for rare situations that don't fit into this model.
pub trait IntoPyNativeFunc<T, R> {
    fn into_func(self) -> PyNativeFunc;
}

impl<F> IntoPyNativeFunc<PyFuncArgs, PyResult> for F
where
    F: Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult + 'static,
{
    fn into_func(self) -> PyNativeFunc {
        Box::new(self)
    }
}

impl IntoPyNativeFunc<PyFuncArgs, PyResult> for PyNativeFunc {
    fn into_func(self) -> PyNativeFunc {
        self
    }
}

// This is the "magic" that allows rust functions of varying signatures to
// generate native python functions.
//
// Note that this could be done without a macro - it is simply to avoid repetition.
macro_rules! into_py_native_func_tuple {
    ($(($n:tt, $T:ident)),*) => {
        impl<F, $($T,)* R> IntoPyNativeFunc<($($T,)*), R> for F
        where
            F: Fn($($T,)* &mut VirtualMachine) -> R + 'static,
            $($T: FromArgs,)*
            ($($T,)*): FromArgs,
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                Box::new(move |vm, args| {
                    let ($($n,)*) = args.bind::<($($T,)*)>(vm)?;

                    (self)($($n,)* vm).into_pyobject(vm)
                })
            }
        }
    };
}

into_py_native_func_tuple!();
into_py_native_func_tuple!((a, A));
into_py_native_func_tuple!((a, A), (b, B));
into_py_native_func_tuple!((a, A), (b, B), (c, C));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D), (e, E));

// TODO: This is a workaround and shouldn't exist.
//       Each iterable type should have its own distinct iterator type.
#[derive(Debug)]
pub struct PyIteratorValue {
    pub position: Cell<usize>,
    pub iterated_obj: PyObjectRef,
}

impl PyValue for PyIteratorValue {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        vm.ctx.iter_type()
    }
}

impl PyObject {
    pub fn new<T: PyObjectPayload>(payload: T, typ: PyObjectRef) -> PyObjectRef {
        PyObject {
            typ,
            dict: Some(RefCell::new(PyAttributes::new())),
            payload: Box::new(payload),
        }
        .into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(self)
    }

    #[inline]
    pub fn payload<T: PyValue>(&self) -> Option<&T> {
        self.payload.as_any().downcast_ref()
    }
}

pub trait PyValue: fmt::Debug + Sized + 'static {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef;

    fn into_ref(self, vm: &mut VirtualMachine) -> PyRef<Self> {
        PyRef {
            obj: PyObject::new(self, Self::class(vm)),
            _payload: PhantomData,
        }
    }

    fn into_ref_with_type(self, vm: &mut VirtualMachine, cls: PyClassRef) -> PyResult<PyRef<Self>> {
        let class = Self::class(vm);
        if objtype::issubclass(&cls.obj, &class) {
            Ok(PyRef {
                obj: PyObject::new(self, cls.obj),
                _payload: PhantomData,
            })
        } else {
            let subtype = vm.to_pystr(&cls.obj)?;
            let basetype = vm.to_pystr(&class)?;
            Err(vm.new_type_error(format!("{} is not a subtype of {}", subtype, basetype)))
        }
    }
}


pub trait PyImmutableClass : PyValue {
    fn create_type(ctx: &PyContext) -> PyClassRef;

    fn into_ref_with_context(self, ctx: &PyContext) -> PyRef<Self> {
        PyRef {
            obj: PyObject::new(self, Self::class_from_ctx(ctx).into_object()),
            _payload: PhantomData,
        }
    }

    fn class_from_ctx(ctx: &PyContext) -> PyClassRef {
        if let Some(class) = ctx.get_type::<Self>() {
            class
        } else {
            let class = Self::create_type(ctx);
            ctx.set_type::<Self>(class.clone());
            class
        }
    }
}

impl<T:PyImmutableClass + 'static> PyValue for T {
    fn class(vm: &mut VirtualMachine) -> PyObjectRef {
        T::class_from_ctx(&vm.ctx).into_object()
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

impl FromPyObjectRef for PyRef<PyClass> {
    fn from_pyobj(obj: &PyObjectRef) -> Self {
        if let Some(_) = obj.payload::<PyClass>() {
            PyRef {
                obj: obj.clone(),
                _payload: PhantomData,
            }
        } else {
            panic!("Error getting inner type: {:?}", obj.typ)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PyContext;

    #[test]
    fn test_type_type() {
        // TODO: Write this test
        PyContext::new();
    }
}
