use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::iter;
use std::ops::RangeInclusive;
use std::rc::{Rc, Weak};

use num_bigint::BigInt;
use num_bigint::ToBigInt;
use num_complex::Complex64;
use num_traits::{One, Zero};

use crate::bytecode;
use crate::exceptions;
use crate::frame::{Frame, Scope, ScopeRef};
use crate::obj::objbool;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objcode;
use crate::obj::objcomplex::{self, PyComplex};
use crate::obj::objdict;
use crate::obj::objellipsis;
use crate::obj::objenumerate;
use crate::obj::objfilter;
use crate::obj::objfloat::{self, PyFloat};
use crate::obj::objframe;
use crate::obj::objfunction;
use crate::obj::objgenerator;
use crate::obj::objint;
use crate::obj::objiter;
use crate::obj::objlist;
use crate::obj::objmap;
use crate::obj::objmemory;
use crate::obj::objnone;
use crate::obj::objobject;
use crate::obj::objproperty;
use crate::obj::objrange;
use crate::obj::objset;
use crate::obj::objslice;
use crate::obj::objstr;
use crate::obj::objsuper;
use crate::obj::objtuple;
use crate::obj::objtype;
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

/// Same as PyObjectRef, except for being a weak reference.
pub type PyObjectWeakRef = Weak<PyObject>;

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
        match &self.payload {
            PyObjectPayload::Module { name, .. } => write!(f, "module '{}'", name),
            PyObjectPayload::Class { name, .. } => {
                let type_name = objtype::get_type_name(&self.typ());
                // We don't have access to a vm, so just assume that if its parent's name
                // is type, it's a type
                if type_name == "type" {
                    write!(f, "type object '{}'", name)
                } else {
                    write!(f, "'{}' object", type_name)
                }
            }
            _ => write!(f, "'{}' object", objtype::get_type_name(&self.typ())),
        }
    }
}

/*
 // Idea: implement the iterator trait upon PyObjectRef
impl Iterator for (VirtualMachine, PyObjectRef) {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        // call method ("_next__")
    }
}
*/

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
    pub set_type: PyObjectRef,
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
    pub module_type: PyObjectRef,
    pub bound_method_type: PyObjectRef,
    pub member_descriptor_type: PyObjectRef,
    pub object: PyObjectRef,
    pub exceptions: exceptions::ExceptionZoo,
}

fn _nothing() -> PyObjectRef {
    let obj: PyObject = Default::default();
    obj.into_ref()
}

pub fn create_type(
    name: &str,
    type_type: &PyObjectRef,
    base: &PyObjectRef,
    _dict_type: &PyObjectRef,
) -> PyObjectRef {
    let dict = PyAttributes::new();
    objtype::new(type_type.clone(), name, vec![base.clone()], dict).unwrap()
}

// Basic objects:
impl PyContext {
    pub fn new() -> Self {
        let type_type = _nothing();
        let object_type = _nothing();
        let dict_type = _nothing();

        objtype::create_type(type_type.clone(), object_type.clone(), dict_type.clone());
        objobject::create_object(type_type.clone(), object_type.clone(), dict_type.clone());
        objdict::create_type(type_type.clone(), object_type.clone(), dict_type.clone());

        let module_type = create_type("module", &type_type, &object_type, &dict_type);
        let classmethod_type = create_type("classmethod", &type_type, &object_type, &dict_type);
        let staticmethod_type = create_type("staticmethod", &type_type, &object_type, &dict_type);
        let function_type = create_type("function", &type_type, &object_type, &dict_type);
        let builtin_function_or_method_type = create_type(
            "builtin_function_or_method",
            &type_type,
            &object_type,
            &dict_type,
        );
        let property_type = create_type("property", &type_type, &object_type, &dict_type);
        let super_type = create_type("super", &type_type, &object_type, &dict_type);
        let generator_type = create_type("generator", &type_type, &object_type, &dict_type);
        let bound_method_type = create_type("method", &type_type, &object_type, &dict_type);
        let member_descriptor_type =
            create_type("member_descriptor", &type_type, &object_type, &dict_type);
        let str_type = create_type("str", &type_type, &object_type, &dict_type);
        let list_type = create_type("list", &type_type, &object_type, &dict_type);
        let set_type = create_type("set", &type_type, &object_type, &dict_type);
        let frozenset_type = create_type("frozenset", &type_type, &object_type, &dict_type);
        let int_type = create_type("int", &type_type, &object_type, &dict_type);
        let float_type = create_type("float", &type_type, &object_type, &dict_type);
        let frame_type = create_type("frame", &type_type, &object_type, &dict_type);
        let complex_type = create_type("complex", &type_type, &object_type, &dict_type);
        let bytes_type = create_type("bytes", &type_type, &object_type, &dict_type);
        let bytearray_type = create_type("bytearray", &type_type, &object_type, &dict_type);
        let tuple_type = create_type("tuple", &type_type, &object_type, &dict_type);
        let iter_type = create_type("iter", &type_type, &object_type, &dict_type);
        let ellipsis_type = create_type("EllipsisType", &type_type, &object_type, &dict_type);
        let enumerate_type = create_type("enumerate", &type_type, &object_type, &dict_type);
        let filter_type = create_type("filter", &type_type, &object_type, &dict_type);
        let map_type = create_type("map", &type_type, &object_type, &dict_type);
        let zip_type = create_type("zip", &type_type, &object_type, &dict_type);
        let bool_type = create_type("bool", &type_type, &int_type, &dict_type);
        let memoryview_type = create_type("memoryview", &type_type, &object_type, &dict_type);
        let code_type = create_type("code", &type_type, &int_type, &dict_type);
        let range_type = create_type("range", &type_type, &object_type, &dict_type);
        let slice_type = create_type("slice", &type_type, &object_type, &dict_type);
        let exceptions = exceptions::ExceptionZoo::new(&type_type, &object_type, &dict_type);

        let none = PyObject::new(
            PyObjectPayload::NoPayload,
            create_type("NoneType", &type_type, &object_type, &dict_type),
        );

        let ellipsis = PyObject::new(PyObjectPayload::NoPayload, ellipsis_type.clone());

        let not_implemented = PyObject::new(
            PyObjectPayload::NoPayload,
            create_type("NotImplementedType", &type_type, &object_type, &dict_type),
        );

        let true_value = PyObject::new(
            PyObjectPayload::Integer { value: One::one() },
            bool_type.clone(),
        );
        let false_value = PyObject::new(
            PyObjectPayload::Integer {
                value: Zero::zero(),
            },
            bool_type.clone(),
        );
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
            set_type,
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
            generator_type,
            module_type,
            bound_method_type,
            member_descriptor_type,
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
        objnone::init(&context);
        exceptions::init(&context);
        context
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

    pub fn set_type(&self) -> PyObjectRef {
        self.set_type.clone()
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
    pub fn member_descriptor_type(&self) -> PyObjectRef {
        self.member_descriptor_type.clone()
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

    pub fn new_int<T: ToBigInt>(&self, i: T) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Integer {
                value: i.to_bigint().unwrap(),
            },
            self.int_type(),
        )
    }

    pub fn new_float(&self, value: f64) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(PyFloat::from(value)),
            },
            self.float_type(),
        )
    }

    pub fn new_complex(&self, value: Complex64) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(PyComplex::from(value)),
            },
            self.complex_type(),
        )
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(objstr::PyString { value: s }),
            },
            self.str_type(),
        )
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(objbytes::PyBytes::new(data)),
            },
            self.bytes_type(),
        )
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(objbytearray::PyByteArray::new(data)),
            },
            self.bytearray_type(),
        )
    }

    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        if b {
            self.true_value.clone()
        } else {
            self.false_value.clone()
        }
    }

    pub fn new_tuple(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Sequence {
                elements: RefCell::new(elements),
            },
            self.tuple_type(),
        )
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Sequence {
                elements: RefCell::new(elements),
            },
            self.list_type(),
        )
    }

    pub fn new_set(&self) -> PyObjectRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the objset code itself.
        PyObject::new(
            PyObjectPayload::Set {
                elements: RefCell::new(HashMap::new()),
            },
            self.set_type(),
        )
    }

    pub fn new_dict(&self) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Dict {
                elements: RefCell::new(HashMap::new()),
            },
            self.dict_type(),
        )
    }

    pub fn new_class(&self, name: &str, base: PyObjectRef) -> PyObjectRef {
        objtype::new(self.type_type(), name, vec![base], PyAttributes::new()).unwrap()
    }

    pub fn new_scope(&self, parent: Option<ScopeRef>) -> ScopeRef {
        let locals = self.new_dict();
        Rc::new(Scope { locals, parent })
    }

    pub fn new_module(&self, name: &str, scope: ScopeRef) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Module {
                name: name.to_string(),
                scope,
            },
            self.module_type.clone(),
        )
    }

    pub fn new_rustfunc<F, T, R>(&self, f: F) -> PyObjectRef
    where
        F: IntoPyNativeFunc<T, R>,
    {
        PyObject::new(
            PyObjectPayload::RustFunction {
                function: f.into_func(),
            },
            self.builtin_function_or_method_type(),
        )
    }

    pub fn new_frame(&self, code: PyObjectRef, scope: ScopeRef) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Frame {
                frame: Frame::new(code, scope),
            },
            self.frame_type(),
        )
    }

    pub fn new_property<F: 'static + Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult>(
        &self,
        function: F,
    ) -> PyObjectRef {
        let fget = self.new_rustfunc(function);
        let py_obj = self.new_instance(self.property_type(), None);
        self.set_attr(&py_obj, "fget", fget.clone());
        py_obj
    }

    pub fn new_code_object(&self, code: bytecode::CodeObject) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Code { code }, self.code_type())
    }

    pub fn new_function(
        &self,
        code_obj: PyObjectRef,
        scope: ScopeRef,
        defaults: PyObjectRef,
    ) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Function {
                code: code_obj,
                scope,
                defaults,
            },
            self.function_type(),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::BoundMethod { function, object },
            self.bound_method_type(),
        )
    }

    pub fn new_member_descriptor<F: 'static + Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult>(
        &self,
        function: F,
    ) -> PyObjectRef {
        let mut dict = PyAttributes::new();
        dict.insert("function".to_string(), self.new_rustfunc(function));
        self.new_instance(self.member_descriptor_type(), Some(dict))
    }

    pub fn new_instance(&self, class: PyObjectRef, dict: Option<PyAttributes>) -> PyObjectRef {
        let dict = if let Some(dict) = dict {
            dict
        } else {
            PyAttributes::new()
        };
        PyObject::new(
            PyObjectPayload::Instance {
                dict: RefCell::new(dict),
            },
            class,
        )
    }

    // Item set/get:
    pub fn set_item(&self, obj: &PyObjectRef, key: &str, v: PyObjectRef) {
        match obj.payload {
            PyObjectPayload::Dict { ref elements } => {
                let key = self.new_str(key.to_string());
                objdict::set_item_in_content(&mut elements.borrow_mut(), &key, &v);
            }
            ref k => panic!("TODO {:?}", k),
        };
    }

    pub fn get_attr(&self, obj: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
        // This does not need to be on the PyContext.
        // We do not require to make a new key as string for this function
        // (yet)...
        obj.get_attr(attr_name)
    }

    pub fn set_attr(&self, obj: &PyObjectRef, attr_name: &str, value: PyObjectRef) {
        match obj.payload {
            PyObjectPayload::Module { ref scope, .. } => {
                scope.locals.set_item(self, attr_name, value)
            }
            PyObjectPayload::Instance { ref dict } | PyObjectPayload::Class { ref dict, .. } => {
                dict.borrow_mut().insert(attr_name.to_string(), value);
            }
            ref payload => unimplemented!("set_attr unimplemented for: {:?}", payload),
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
#[derive(Default)]
pub struct PyObject {
    pub payload: PyObjectPayload,
    pub typ: Option<PyObjectRef>,
    pub dict: Option<HashMap<String, PyObjectRef>>, // __dict__ member
}

pub trait IdProtocol {
    fn get_id(&self) -> usize;
    fn is(&self, other: &PyObjectRef) -> bool;
}

impl IdProtocol for PyObjectRef {
    fn get_id(&self) -> usize {
        &*self as &PyObject as *const PyObject as usize
    }

    fn is(&self, other: &PyObjectRef) -> bool {
        self.get_id() == other.get_id()
    }
}

pub trait FromPyObjectRef {
    fn from_pyobj(obj: &PyObjectRef) -> Self;
}

pub trait TypeProtocol {
    fn typ(&self) -> PyObjectRef;
}

impl TypeProtocol for PyObjectRef {
    fn typ(&self) -> PyObjectRef {
        (**self).typ()
    }
}

impl TypeProtocol for PyObject {
    fn typ(&self) -> PyObjectRef {
        match self.typ {
            Some(ref typ) => typ.clone(),
            None => panic!("Object {:?} doesn't have a type!", self),
        }
    }
}

pub trait AttributeProtocol {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef>;
    fn has_attr(&self, attr_name: &str) -> bool;
}

fn class_get_item(class: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
    match class.payload {
        PyObjectPayload::Class { ref dict, .. } => dict.borrow().get(attr_name).cloned(),
        _ => panic!("Only classes should be in MRO!"),
    }
}

fn class_has_item(class: &PyObjectRef, attr_name: &str) -> bool {
    match class.payload {
        PyObjectPayload::Class { ref dict, .. } => dict.borrow().contains_key(attr_name),
        _ => panic!("Only classes should be in MRO!"),
    }
}

impl AttributeProtocol for PyObjectRef {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        match self.payload {
            PyObjectPayload::Module { ref scope, .. } => scope.locals.get_item(attr_name),
            PyObjectPayload::Class { ref mro, .. } => {
                if let Some(item) = class_get_item(self, attr_name) {
                    return Some(item);
                }
                for class in mro {
                    if let Some(item) = class_get_item(class, attr_name) {
                        return Some(item);
                    }
                }
                None
            }
            PyObjectPayload::Instance { ref dict } => dict.borrow().get(attr_name).cloned(),
            _ => None,
        }
    }

    fn has_attr(&self, attr_name: &str) -> bool {
        match self.payload {
            PyObjectPayload::Module { ref scope, .. } => scope.locals.contains_key(attr_name),
            PyObjectPayload::Class { ref mro, .. } => {
                class_has_item(self, attr_name) || mro.iter().any(|d| class_has_item(d, attr_name))
            }
            PyObjectPayload::Instance { ref dict } => dict.borrow().contains_key(attr_name),
            _ => false,
        }
    }
}

pub trait DictProtocol {
    fn contains_key(&self, k: &str) -> bool;
    fn get_item(&self, k: &str) -> Option<PyObjectRef>;
    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)>;
    fn set_item(&self, ctx: &PyContext, key: &str, v: PyObjectRef);
}

impl DictProtocol for PyObjectRef {
    fn contains_key(&self, k: &str) -> bool {
        match self.payload {
            PyObjectPayload::Dict { ref elements } => {
                objdict::content_contains_key_str(&elements.borrow(), k)
            }
            ref payload => unimplemented!("TODO {:?}", payload),
        }
    }

    fn get_item(&self, k: &str) -> Option<PyObjectRef> {
        match self.payload {
            PyObjectPayload::Dict { ref elements } => {
                objdict::content_get_key_str(&elements.borrow(), k)
            }
            PyObjectPayload::Module { ref scope, .. } => scope.locals.get_item(k),
            ref k => panic!("TODO {:?}", k),
        }
    }

    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        match self.payload {
            PyObjectPayload::Dict { .. } => objdict::get_key_value_pairs(self),
            PyObjectPayload::Module { ref scope, .. } => scope.locals.get_key_value_pairs(),
            _ => panic!("TODO"),
        }
    }

    // Item set/get:
    fn set_item(&self, ctx: &PyContext, key: &str, v: PyObjectRef) {
        match &self.payload {
            PyObjectPayload::Dict { elements } => {
                let key = ctx.new_str(key.to_string());
                objdict::set_item_in_content(&mut elements.borrow_mut(), &key, &v);
            }
            PyObjectPayload::Module { scope, .. } => {
                scope.locals.set_item(ctx, key, v);
            }
            ref k => panic!("TODO {:?}", k),
        };
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

pub struct OptArg<T>(Option<T>);

impl<T> std::ops::Deref for OptArg<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Option<T> {
        &self.0
    }
}

impl<T> FromArgs for OptArg<T>
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
        Ok(OptArg(if let Some(PyArg::Positional(_)) = args.peek() {
            let value = if let Some(PyArg::Positional(value)) = args.next() {
                value
            } else {
                unreachable!()
            };
            Some(T::try_from_object(vm, value)?)
        } else {
            None
        }))
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
    fn into_pyobject(self, ctx: &PyContext) -> PyResult;
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, _ctx: &PyContext) -> PyResult {
        Ok(self)
    }
}

impl<T> IntoPyObject for PyResult<T>
where
    T: IntoPyObject,
{
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        self.and_then(|res| T::into_pyobject(res, ctx))
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl IntoPyObject for () {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(ctx.none())
    }
}

// TODO: Allow a built-in function to return an `Option`, i.e.:
//
//     impl<T: IntoPyObject> IntoPyObject for Option<T>
//
// Option::None should map to a Python `None`.

// Allows a built-in function to return any built-in object payload without
// explicitly implementing `IntoPyObject`.
impl<T> IntoPyObject for T
where
    T: PyObjectPayload2 + Sized,
{
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(PyObject::new(
            PyObjectPayload::AnyRustValue {
                value: Box::new(self),
            },
            T::required_type(ctx),
        ))
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
    ($(($n:tt, $T:ident)),+) => {
        impl<F, $($T,)+ R> IntoPyNativeFunc<($($T,)+), R> for F
        where
            F: Fn($($T,)+ &mut VirtualMachine) -> R + 'static,
            $($T: FromArgs,)+
            ($($T,)+): FromArgs,
            R: IntoPyObject,
        {
            fn into_func(self) -> PyNativeFunc {
                Box::new(move |vm, args| {
                    let ($($n,)+) = args.bind::<($($T,)+)>(vm)?;

                    (self)($($n,)+ vm).into_pyobject(&vm.ctx)
                })
            }
        }
    };
}

into_py_native_func_tuple!((a, A));
into_py_native_func_tuple!((a, A), (b, B));
into_py_native_func_tuple!((a, A), (b, B), (c, C));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D));
into_py_native_func_tuple!((a, A), (b, B), (c, C), (d, D), (e, E));

/// Rather than determining the type of a python object, this enum is more
/// a holder for the rust payload of a python object. It is more a carrier
/// of rust data for a particular python object. Determine the python type
/// by using for example the `.typ()` method on a python object.
pub enum PyObjectPayload {
    Integer {
        value: BigInt,
    },
    Sequence {
        elements: RefCell<Vec<PyObjectRef>>,
    },
    Dict {
        elements: RefCell<objdict::DictContentType>,
    },
    Iterator {
        position: Cell<usize>,
        iterated_obj: PyObjectRef,
    },
    EnumerateIterator {
        counter: RefCell<BigInt>,
        iterator: PyObjectRef,
    },
    FilterIterator {
        predicate: PyObjectRef,
        iterator: PyObjectRef,
    },
    MapIterator {
        mapper: PyObjectRef,
        iterators: Vec<PyObjectRef>,
    },
    ZipIterator {
        iterators: Vec<PyObjectRef>,
    },
    Slice {
        start: Option<BigInt>,
        stop: Option<BigInt>,
        step: Option<BigInt>,
    },
    MemoryView {
        obj: PyObjectRef,
    },
    Code {
        code: bytecode::CodeObject,
    },
    Frame {
        frame: Frame,
    },
    Function {
        code: PyObjectRef,
        scope: ScopeRef,
        defaults: PyObjectRef,
    },
    Generator {
        frame: PyObjectRef,
    },
    BoundMethod {
        function: PyObjectRef,
        object: PyObjectRef,
    },
    Module {
        name: String,
        scope: ScopeRef,
    },
    NoPayload,
    Class {
        name: String,
        dict: RefCell<PyAttributes>,
        mro: Vec<PyObjectRef>,
    },
    Set {
        elements: RefCell<HashMap<u64, PyObjectRef>>,
    },
    WeakRef {
        referent: PyObjectWeakRef,
    },
    Instance {
        dict: RefCell<PyAttributes>,
    },
    RustFunction {
        function: PyNativeFunc,
    },
    AnyRustValue {
        value: Box<dyn std::any::Any>,
    },
}

impl Default for PyObjectPayload {
    fn default() -> Self {
        PyObjectPayload::NoPayload
    }
}

impl fmt::Debug for PyObjectPayload {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PyObjectPayload::Integer { ref value } => write!(f, "int {}", value),
            PyObjectPayload::MemoryView { ref obj } => write!(f, "bytes/bytearray {:?}", obj),
            PyObjectPayload::Sequence { .. } => write!(f, "list or tuple"),
            PyObjectPayload::Dict { .. } => write!(f, "dict"),
            PyObjectPayload::Set { .. } => write!(f, "set"),
            PyObjectPayload::WeakRef { .. } => write!(f, "weakref"),
            PyObjectPayload::Iterator { .. } => write!(f, "iterator"),
            PyObjectPayload::EnumerateIterator { .. } => write!(f, "enumerate"),
            PyObjectPayload::FilterIterator { .. } => write!(f, "filter"),
            PyObjectPayload::MapIterator { .. } => write!(f, "map"),
            PyObjectPayload::ZipIterator { .. } => write!(f, "zip"),
            PyObjectPayload::Slice { .. } => write!(f, "slice"),
            PyObjectPayload::Code { ref code } => write!(f, "code: {:?}", code),
            PyObjectPayload::Function { .. } => write!(f, "function"),
            PyObjectPayload::Generator { .. } => write!(f, "generator"),
            PyObjectPayload::BoundMethod {
                ref function,
                ref object,
            } => write!(f, "bound-method: {:?} of {:?}", function, object),
            PyObjectPayload::Module { .. } => write!(f, "module"),
            PyObjectPayload::NoPayload => write!(f, "NoPayload"),
            PyObjectPayload::Class { ref name, .. } => write!(f, "class {:?}", name),
            PyObjectPayload::Instance { .. } => write!(f, "instance"),
            PyObjectPayload::RustFunction { .. } => write!(f, "rust function"),
            PyObjectPayload::Frame { .. } => write!(f, "frame"),
            PyObjectPayload::AnyRustValue { .. } => write!(f, "some rust value"),
        }
    }
}

impl PyObject {
    pub fn new(payload: PyObjectPayload, typ: PyObjectRef) -> PyObjectRef {
        PyObject {
            payload,
            typ: Some(typ),
            dict: None,
        }
        .into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(self)
    }

    pub fn payload<T: PyObjectPayload2>(&self) -> Option<&T> {
        if let PyObjectPayload::AnyRustValue { ref value } = self.payload {
            value.downcast_ref()
        } else {
            None
        }
    }
}

// The intention is for this to replace `PyObjectPayload` once everything is
// converted to use `PyObjectPayload::AnyRustvalue`.
pub trait PyObjectPayload2: std::any::Any + fmt::Debug {
    fn required_type(ctx: &PyContext) -> PyObjectRef;
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
