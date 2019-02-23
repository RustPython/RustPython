use crate::bytecode;
use crate::exceptions;
use crate::frame::Frame;
use crate::obj::objbool;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objcode;
use crate::obj::objcomplex;
use crate::obj::objdict;
use crate::obj::objenumerate;
use crate::obj::objfilter;
use crate::obj::objfloat;
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
use num_bigint::BigInt;
use num_bigint::ToBigInt;
use num_complex::Complex64;
use num_traits::{One, Zero};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::{Rc, Weak};

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

/*
The PyRef type implements
https://doc.rust-lang.org/std/cell/index.html#introducing-mutability-inside-of-something-immutable
*/
pub type PyRef<T> = Rc<RefCell<T>>;

/// The `PyObjectRef` is one of the most used types. It is a reference to a
/// python object. A single python object can have multiple references, and
/// this reference counting is accounted for by this type. Use the `.clone()`
/// method to create a new reference and increment the amount of references
/// to the python object by 1.
pub type PyObjectRef = PyRef<PyObject>;

/// Same as PyObjectRef, except for being a weak reference.
pub type PyObjectWeakRef = Weak<RefCell<PyObject>>;

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

/*
 * So a scope is a linked list of scopes.
 * When a name is looked up, it is check in its scope.
 */
#[derive(Debug)]
pub struct Scope {
    pub locals: PyObjectRef, // Variables
    // TODO: pub locals: RefCell<PyAttributes>,         // Variables
    pub parent: Option<PyObjectRef>, // Parent scope
}

fn _nothing() -> PyObjectRef {
    PyObject {
        payload: PyObjectPayload::None,
        typ: None,
    }
    .into_ref()
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
            PyObjectPayload::None,
            create_type("NoneType", &type_type, &object_type, &dict_type),
        );

        let not_implemented = PyObject::new(
            PyObjectPayload::NotImplemented,
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
            enumerate_type,
            filter_type,
            map_type,
            zip_type,
            dict_type,
            none,
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

    pub fn new_float(&self, i: f64) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Float { value: i }, self.float_type())
    }

    pub fn new_complex(&self, i: Complex64) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Complex { value: i }, self.complex_type())
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(PyObjectPayload::String { value: s }, self.str_type())
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Bytes { value: data }, self.bytes_type())
    }

    pub fn new_bytearray(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Bytes { value: data },
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
        PyObject::new(PyObjectPayload::Sequence { elements }, self.tuple_type())
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Sequence { elements }, self.list_type())
    }

    pub fn new_set(&self) -> PyObjectRef {
        // Initialized empty, as calling __hash__ is required for adding each object to the set
        // which requires a VM context - this is done in the objset code itself.
        let elements: HashMap<u64, PyObjectRef> = HashMap::new();
        PyObject::new(PyObjectPayload::Set { elements }, self.set_type())
    }

    pub fn new_dict(&self) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Dict {
                elements: HashMap::new(),
            },
            self.dict_type(),
        )
    }

    pub fn new_class(&self, name: &str, base: PyObjectRef) -> PyObjectRef {
        objtype::new(self.type_type(), name, vec![base], PyAttributes::new()).unwrap()
    }

    pub fn new_scope(&self, parent: Option<PyObjectRef>) -> PyObjectRef {
        let locals = self.new_dict();
        let scope = Scope { locals, parent };
        PyObject {
            payload: PyObjectPayload::Scope { scope },
            typ: None,
        }
        .into_ref()
    }

    pub fn new_module(&self, name: &str, scope: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::Module {
                name: name.to_string(),
                dict: scope.clone(),
            },
            self.module_type.clone(),
        )
    }

    pub fn new_rustfunc<F, T, R>(&self, factory: F) -> PyObjectRef
    where
        F: PyNativeFuncFactory<T, R>,
        T: FromPyFuncArgs,
        R: IntoPyObject,
    {
        PyObject::new(
            PyObjectPayload::RustFunction {
                function: factory.create(),
            },
            self.builtin_function_or_method_type(),
        )
    }

    pub fn new_rustfunc_from_box(
        &self,
        function: Box<Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult>,
    ) -> PyObjectRef {
        PyObject::new(
            PyObjectPayload::RustFunction { function },
            self.builtin_function_or_method_type(),
        )
    }

    pub fn new_frame(&self, frame: Frame) -> PyObjectRef {
        PyObject::new(PyObjectPayload::Frame { frame }, self.frame_type())
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
        scope: PyObjectRef,
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
        match obj.borrow_mut().payload {
            PyObjectPayload::Dict { ref mut elements } => {
                let key = self.new_str(key.to_string());
                objdict::set_item_in_content(elements, &key, &v);
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
        match obj.borrow().payload {
            PyObjectPayload::Module { ref dict, .. } => self.set_attr(dict, attr_name, value),
            PyObjectPayload::Instance { ref dict } | PyObjectPayload::Class { ref dict, .. } => {
                dict.borrow_mut().insert(attr_name.to_string(), value);
            }
            PyObjectPayload::Scope { ref scope } => {
                self.set_item(&scope.locals, attr_name, value);
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
            bytecode::Constant::Code { ref code } => self.new_code_object(code.clone()),
            bytecode::Constant::Tuple { ref elements } => {
                let elements = elements
                    .iter()
                    .map(|value| self.unwrap_constant(value))
                    .collect();
                self.new_tuple(elements)
            }
            bytecode::Constant::None => self.none(),
        }
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
pub struct PyObject {
    pub payload: PyObjectPayload,
    pub typ: Option<PyObjectRef>,
    // pub dict: HashMap<String, PyObjectRef>, // __dict__ member
}

pub trait IdProtocol {
    fn get_id(&self) -> usize;
    fn is(&self, other: &PyObjectRef) -> bool;
}

impl IdProtocol for PyObjectRef {
    fn get_id(&self) -> usize {
        self.as_ptr() as usize
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
        self.borrow().typ()
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

pub trait ParentProtocol {
    fn has_parent(&self) -> bool;
    fn get_parent(&self) -> PyObjectRef;
}

impl ParentProtocol for PyObjectRef {
    fn has_parent(&self) -> bool {
        match self.borrow().payload {
            PyObjectPayload::Scope { ref scope } => scope.parent.is_some(),
            _ => panic!("Only scopes have parent (not {:?}", self),
        }
    }

    fn get_parent(&self) -> PyObjectRef {
        match self.borrow().payload {
            PyObjectPayload::Scope { ref scope } => match scope.parent {
                Some(ref value) => value.clone(),
                None => panic!("OMG"),
            },
            _ => panic!("TODO"),
        }
    }
}

pub trait AttributeProtocol {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef>;
    fn has_attr(&self, attr_name: &str) -> bool;
}

fn class_get_item(class: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
    let class = class.borrow();
    match class.payload {
        PyObjectPayload::Class { ref dict, .. } => dict.borrow().get(attr_name).cloned(),
        _ => panic!("Only classes should be in MRO!"),
    }
}

fn class_has_item(class: &PyObjectRef, attr_name: &str) -> bool {
    let class = class.borrow();
    match class.payload {
        PyObjectPayload::Class { ref dict, .. } => dict.borrow().contains_key(attr_name),
        _ => panic!("Only classes should be in MRO!"),
    }
}

impl AttributeProtocol for PyObjectRef {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        let obj = self.borrow();
        match obj.payload {
            PyObjectPayload::Module { ref dict, .. } => dict.get_item(attr_name),
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
        let obj = self.borrow();
        match obj.payload {
            PyObjectPayload::Module { ref dict, .. } => dict.contains_key(attr_name),
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
}

impl DictProtocol for PyObjectRef {
    fn contains_key(&self, k: &str) -> bool {
        match self.borrow().payload {
            PyObjectPayload::Dict { ref elements } => {
                objdict::content_contains_key_str(elements, k)
            }
            PyObjectPayload::Scope { ref scope } => scope.locals.contains_key(k),
            ref payload => unimplemented!("TODO {:?}", payload),
        }
    }

    fn get_item(&self, k: &str) -> Option<PyObjectRef> {
        match self.borrow().payload {
            PyObjectPayload::Dict { ref elements } => objdict::content_get_key_str(elements, k),
            PyObjectPayload::Scope { ref scope } => scope.locals.get_item(k),
            _ => panic!("TODO"),
        }
    }

    fn get_key_value_pairs(&self) -> Vec<(PyObjectRef, PyObjectRef)> {
        match self.borrow().payload {
            PyObjectPayload::Dict { .. } => objdict::get_key_value_pairs(self),
            PyObjectPayload::Module { ref dict, .. } => dict.get_key_value_pairs(),
            PyObjectPayload::Scope { ref scope } => scope.locals.get_key_value_pairs(),
            _ => panic!("TODO"),
        }
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
}

pub trait FromPyObject: Sized {
    fn from_pyobject(obj: PyObjectRef) -> PyResult<Self>;
}

impl FromPyObject for PyObjectRef {
    fn from_pyobject(obj: PyObjectRef) -> PyResult<Self> {
        Ok(obj)
    }
}

pub trait IntoPyObject {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult;
}

impl IntoPyObject for PyObjectRef {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        Ok(self)
    }
}

impl IntoPyObject for PyResult {
    fn into_pyobject(self, ctx: &PyContext) -> PyResult {
        self
    }
}

pub trait FromPyFuncArgs: Sized {
    fn from_py_func_args(args: &mut PyFuncArgs) -> PyResult<Self>;
}

impl<T> FromPyFuncArgs for Vec<T>
where
    T: FromPyFuncArgs,
{
    fn from_py_func_args(args: &mut PyFuncArgs) -> PyResult<Self> {
        let mut v = Vec::with_capacity(args.args.len());
        // TODO: This will loop infinitely if T::from_py_func_args doesn't
        //       consume any positional args. Check for this and panic.
        while !args.args.is_empty() {
            v.push(T::from_py_func_args(args)?);
        }
        Ok(v)
    }
}

macro_rules! tuple_from_py_func_args {
    ($($T:ident),+) => {
        impl<$($T),+> FromPyFuncArgs for ($($T,)+)
        where
            $($T: FromPyFuncArgs),+
        {
            fn from_py_func_args(args: &mut PyFuncArgs) -> PyResult<Self> {
                Ok(($($T::from_py_func_args(args)?,)+))
            }
        }
    };
}

tuple_from_py_func_args!(A);
tuple_from_py_func_args!(A, B);
tuple_from_py_func_args!(A, B, C);
tuple_from_py_func_args!(A, B, C, D);
tuple_from_py_func_args!(A, B, C, D, E);

impl<T> FromPyFuncArgs for T
where
    T: FromPyObject,
{
    fn from_py_func_args(args: &mut PyFuncArgs) -> PyResult<Self> {
        Self::from_pyobject(args.shift())
    }
}

pub type PyNativeFunc = Box<dyn Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult>;

pub trait PyNativeFuncFactory<T, R> {
    fn create(self) -> PyNativeFunc;
}

impl<F, A, R> PyNativeFuncFactory<(A,), R> for F
where
    F: Fn(&mut VirtualMachine, A) -> R + 'static,
    A: FromPyFuncArgs,
    R: IntoPyObject,
{
    fn create(self) -> PyNativeFunc {
        Box::new(move |vm, mut args| {
            // TODO: type-checking!
            (self)(vm, A::from_py_func_args(&mut args)?).into_pyobject(&vm.ctx)
        })
    }
}

impl<F, A, B, R> PyNativeFuncFactory<(A, B), R> for F
where
    F: Fn(&mut VirtualMachine, A, B) -> R + 'static,
    A: FromPyFuncArgs,
    B: FromPyFuncArgs,
    R: IntoPyObject,
{
    fn create(self) -> PyNativeFunc {
        Box::new(move |vm, mut args| {
            (self)(
                vm,
                A::from_py_func_args(&mut args)?,
                B::from_py_func_args(&mut args)?,
            )
            .into_pyobject(&vm.ctx)
        })
    }
}

impl FromPyFuncArgs for PyFuncArgs {
    fn from_py_func_args(args: &mut PyFuncArgs) -> PyResult<PyFuncArgs> {
        // HACK HACK HACK
        // TODO: get rid of this clone!
        Ok(args.clone())
    }
}

/// Rather than determining the type of a python object, this enum is more
/// a holder for the rust payload of a python object. It is more a carrier
/// of rust data for a particular python object. Determine the python type
/// by using for example the `.typ()` method on a python object.
pub enum PyObjectPayload {
    String {
        value: String,
    },
    Integer {
        value: BigInt,
    },
    Float {
        value: f64,
    },
    Complex {
        value: Complex64,
    },
    Bytes {
        value: Vec<u8>,
    },
    Sequence {
        elements: Vec<PyObjectRef>,
    },
    Dict {
        elements: objdict::DictContentType,
    },
    Set {
        elements: HashMap<u64, PyObjectRef>,
    },
    Iterator {
        position: usize,
        iterated_obj: PyObjectRef,
    },
    EnumerateIterator {
        counter: BigInt,
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
    Range {
        range: objrange::RangeType,
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
        scope: PyObjectRef,
        defaults: PyObjectRef,
    },
    Generator {
        frame: Frame,
    },
    BoundMethod {
        function: PyObjectRef,
        object: PyObjectRef,
    },
    Scope {
        scope: Scope,
    },
    Module {
        name: String,
        dict: PyObjectRef,
    },
    None,
    NotImplemented,
    Class {
        name: String,
        dict: RefCell<PyAttributes>,
        mro: Vec<PyObjectRef>,
    },
    WeakRef {
        referent: PyObjectWeakRef,
    },
    Instance {
        dict: RefCell<PyAttributes>,
    },
    RustFunction {
        function: Box<Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult>,
    },
}

impl fmt::Debug for PyObjectPayload {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PyObjectPayload::String { ref value } => write!(f, "str \"{}\"", value),
            PyObjectPayload::Integer { ref value } => write!(f, "int {}", value),
            PyObjectPayload::Float { ref value } => write!(f, "float {}", value),
            PyObjectPayload::Complex { ref value } => write!(f, "complex {}", value),
            PyObjectPayload::Bytes { ref value } => write!(f, "bytes/bytearray {:?}", value),
            PyObjectPayload::MemoryView { ref obj } => write!(f, "bytes/bytearray {:?}", obj),
            PyObjectPayload::Sequence { .. } => write!(f, "list or tuple"),
            PyObjectPayload::Dict { .. } => write!(f, "dict"),
            PyObjectPayload::Set { .. } => write!(f, "set"),
            PyObjectPayload::WeakRef { .. } => write!(f, "weakref"),
            PyObjectPayload::Range { .. } => write!(f, "range"),
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
            PyObjectPayload::Scope { .. } => write!(f, "scope"),
            PyObjectPayload::None => write!(f, "None"),
            PyObjectPayload::NotImplemented => write!(f, "NotImplemented"),
            PyObjectPayload::Class { ref name, .. } => write!(f, "class {:?}", name),
            PyObjectPayload::Instance { .. } => write!(f, "instance"),
            PyObjectPayload::RustFunction { .. } => write!(f, "rust function"),
            PyObjectPayload::Frame { .. } => write!(f, "frame"),
        }
    }
}

impl PyObject {
    pub fn new(
        payload: PyObjectPayload,
        /* dict: PyObjectRef,*/ typ: PyObjectRef,
    ) -> PyObjectRef {
        PyObject {
            payload,
            typ: Some(typ),
            // dict: HashMap::new(),  // dict,
        }
        .into_ref()
    }

    // Move this object into a reference object, transferring ownership.
    pub fn into_ref(self) -> PyObjectRef {
        Rc::new(RefCell::new(self))
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
