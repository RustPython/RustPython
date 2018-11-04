use super::bytecode;
use super::exceptions;
use super::frame::Frame;
use super::obj::objbool;
use super::obj::objbytearray;
use super::obj::objbytes;
use super::obj::objcomplex;
use super::obj::objdict;
use super::obj::objfloat;
use super::obj::objfunction;
use super::obj::objgenerator;
use super::obj::objint;
use super::obj::objiter;
use super::obj::objlist;
use super::obj::objobject;
use super::obj::objproperty;
use super::obj::objset;
use super::obj::objstr;
use super::obj::objsuper;
use super::obj::objtuple;
use super::obj::objtype;
use super::vm::VirtualMachine;
use num_bigint::BigInt;
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
pub type PyResult = Result<PyObjectRef, PyObjectRef>; // A valid value, or an exception

/*
impl fmt::Display for PyObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Obj {:?}", self)
    }
}*/

#[derive(Debug)]
pub struct PyContext {
    pub type_type: PyObjectRef,
    pub none: PyObjectRef,
    pub classmethod_type: PyObjectRef,
    pub staticmethod_type: PyObjectRef,
    pub dict_type: PyObjectRef,
    pub int_type: PyObjectRef,
    pub float_type: PyObjectRef,
    pub complex_type: PyObjectRef,
    pub bytes_type: PyObjectRef,
    pub bytearray_type: PyObjectRef,
    pub bool_type: PyObjectRef,
    pub true_value: PyObjectRef,
    pub false_value: PyObjectRef,
    pub list_type: PyObjectRef,
    pub tuple_type: PyObjectRef,
    pub set_type: PyObjectRef,
    pub iter_type: PyObjectRef,
    pub super_type: PyObjectRef,
    pub str_type: PyObjectRef,
    pub function_type: PyObjectRef,
    pub property_type: PyObjectRef,
    pub generator_type: PyObjectRef,
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
    pub locals: PyObjectRef,         // Variables
    pub parent: Option<PyObjectRef>, // Parent scope
}

fn _nothing() -> PyObjectRef {
    PyObject {
        kind: PyObjectKind::None,
        typ: None,
    }
    .into_ref()
}

pub fn create_type(
    name: &str,
    type_type: &PyObjectRef,
    base: &PyObjectRef,
    dict_type: &PyObjectRef,
) -> PyObjectRef {
    let dict = PyObject::new(
        PyObjectKind::Dict {
            elements: HashMap::new(),
        },
        dict_type.clone(),
    );
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
        let property_type = create_type("property", &type_type, &object_type, &dict_type);
        let super_type = create_type("property", &type_type, &object_type, &dict_type);
        let generator_type = create_type("generator", &type_type, &object_type, &dict_type);
        let bound_method_type = create_type("method", &type_type, &object_type, &dict_type);
        let member_descriptor_type =
            create_type("member_descriptor", &type_type, &object_type, &dict_type);
        let str_type = create_type("str", &type_type, &object_type, &dict_type);
        let list_type = create_type("list", &type_type, &object_type, &dict_type);
        let set_type = create_type("set", &type_type, &object_type, &dict_type);
        let int_type = create_type("int", &type_type, &object_type, &dict_type);
        let float_type = create_type("float", &type_type, &object_type, &dict_type);
        let complex_type = create_type("complex", &type_type, &object_type, &dict_type);
        let bytes_type = create_type("bytes", &type_type, &object_type, &dict_type);
        let bytearray_type = create_type("bytearray", &type_type, &object_type, &dict_type);
        let tuple_type = create_type("tuple", &type_type, &object_type, &dict_type);
        let iter_type = create_type("iter", &type_type, &object_type, &dict_type);
        let bool_type = create_type("bool", &type_type, &int_type, &dict_type);
        let exceptions = exceptions::ExceptionZoo::new(&type_type, &object_type, &dict_type);

        let none = PyObject::new(
            PyObjectKind::None,
            create_type("NoneType", &type_type, &object_type, &dict_type),
        );

        let true_value = PyObject::new(
            PyObjectKind::Integer { value: One::one() },
            bool_type.clone(),
        );
        let false_value = PyObject::new(
            PyObjectKind::Integer {
                value: Zero::zero(),
            },
            bool_type.clone(),
        );
        let context = PyContext {
            int_type: int_type,
            float_type: float_type,
            complex_type: complex_type,
            classmethod_type: classmethod_type,
            staticmethod_type: staticmethod_type,
            bytes_type: bytes_type,
            bytearray_type: bytearray_type,
            list_type: list_type,
            set_type: set_type,
            bool_type: bool_type,
            true_value: true_value,
            false_value: false_value,
            tuple_type: tuple_type,
            iter_type: iter_type,
            dict_type: dict_type,
            none: none,
            str_type: str_type,
            object: object_type,
            function_type: function_type,
            super_type: super_type,
            property_type: property_type,
            generator_type: generator_type,
            module_type: module_type,
            bound_method_type: bound_method_type,
            member_descriptor_type: member_descriptor_type,
            type_type: type_type,
            exceptions: exceptions,
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
        objstr::init(&context);
        objsuper::init(&context);
        objtuple::init(&context);
        objiter::init(&context);
        objbool::init(&context);
        exceptions::init(&context);
        context
    }

    pub fn int_type(&self) -> PyObjectRef {
        self.int_type.clone()
    }

    pub fn float_type(&self) -> PyObjectRef {
        self.float_type.clone()
    }

    pub fn complex_type(&self) -> PyObjectRef {
        self.complex_type.clone()
    }

    pub fn bytes_type(&self) -> PyObjectRef {
        self.bytes_type.clone()
    }

    pub fn bytearray_type(&self) -> PyObjectRef {
        self.bytearray_type.clone()
    }

    pub fn list_type(&self) -> PyObjectRef {
        self.list_type.clone()
    }

    pub fn set_type(&self) -> PyObjectRef {
        self.set_type.clone()
    }

    pub fn bool_type(&self) -> PyObjectRef {
        self.bool_type.clone()
    }

    pub fn tuple_type(&self) -> PyObjectRef {
        self.tuple_type.clone()
    }

    pub fn iter_type(&self) -> PyObjectRef {
        self.iter_type.clone()
    }

    pub fn dict_type(&self) -> PyObjectRef {
        self.dict_type.clone()
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
    pub fn object(&self) -> PyObjectRef {
        self.object.clone()
    }

    pub fn new_object(&self) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::Instance {
                dict: self.new_dict(),
            },
            self.object(),
        )
    }

    pub fn new_int(&self, i: BigInt) -> PyObjectRef {
        PyObject::new(PyObjectKind::Integer { value: i }, self.int_type())
    }

    pub fn new_float(&self, i: f64) -> PyObjectRef {
        PyObject::new(PyObjectKind::Float { value: i }, self.float_type())
    }

    pub fn new_complex(&self, i: Complex64) -> PyObjectRef {
        PyObject::new(PyObjectKind::Complex { value: i }, self.complex_type())
    }

    pub fn new_str(&self, s: String) -> PyObjectRef {
        PyObject::new(PyObjectKind::String { value: s }, self.str_type())
    }

    pub fn new_bytes(&self, data: Vec<u8>) -> PyObjectRef {
        PyObject::new(PyObjectKind::Bytes { value: data }, self.bytes_type())
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
            PyObjectKind::Sequence { elements: elements },
            self.tuple_type(),
        )
    }

    pub fn new_list(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::Sequence { elements: elements },
            self.list_type(),
        )
    }

    pub fn new_set(&self, elements: Vec<PyObjectRef>) -> PyObjectRef {
        let elements = objset::sequence_to_hashmap(&elements);
        PyObject::new(PyObjectKind::Set { elements: elements }, self.set_type())
    }

    pub fn new_dict(&self) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::Dict {
                elements: HashMap::new(),
            },
            self.dict_type(),
        )
    }

    pub fn new_class(&self, name: &str, base: PyObjectRef) -> PyObjectRef {
        objtype::new(self.type_type(), name, vec![base], self.new_dict()).unwrap()
    }

    pub fn new_scope(&self, parent: Option<PyObjectRef>) -> PyObjectRef {
        let locals = self.new_dict();
        let scope = Scope {
            locals: locals,
            parent: parent,
        };
        PyObject {
            kind: PyObjectKind::Scope { scope: scope },
            typ: None,
        }
        .into_ref()
    }

    pub fn new_module(&self, name: &str, scope: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::Module {
                name: name.to_string(),
                dict: scope.clone(),
            },
            self.module_type.clone(),
        )
    }

    pub fn new_rustfunc(&self, function: RustPyFunc) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::RustFunction { function: function },
            self.function_type(),
        )
    }

    pub fn new_function(
        &self,
        code_obj: PyObjectRef,
        scope: PyObjectRef,
        defaults: PyObjectRef,
    ) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::Function {
                code: code_obj,
                scope: scope,
                defaults: defaults,
            },
            self.function_type(),
        )
    }

    pub fn new_bound_method(&self, function: PyObjectRef, object: PyObjectRef) -> PyObjectRef {
        PyObject::new(
            PyObjectKind::BoundMethod {
                function: function,
                object: object,
            },
            self.bound_method_type(),
        )
    }

    pub fn new_member_descriptor(&self, function: RustPyFunc) -> PyObjectRef {
        let dict = self.new_dict();
        dict.set_item(&String::from("function"), self.new_rustfunc(function));
        self.new_instance(dict, self.member_descriptor_type())
    }

    pub fn new_instance(&self, dict: PyObjectRef, class: PyObjectRef) -> PyObjectRef {
        PyObject::new(PyObjectKind::Instance { dict: dict }, class)
    }
}

/// This is an actual python object. It consists of a `typ` which is the
/// python class, and carries some rust payload optionally. This rust
/// payload can be a rust float or rust int in case of float and int objects.
pub struct PyObject {
    pub kind: PyObjectKind,
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
        match self.borrow().typ {
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
        match self.borrow().kind {
            PyObjectKind::Scope { ref scope } => match scope.parent {
                Some(_) => true,
                None => false,
            },
            _ => panic!("Only scopes have parent (not {:?}", self),
        }
    }

    fn get_parent(&self) -> PyObjectRef {
        match self.borrow().kind {
            PyObjectKind::Scope { ref scope } => match scope.parent {
                Some(ref value) => value.clone(),
                None => panic!("OMG"),
            },
            _ => panic!("TODO"),
        }
    }
}

pub trait AttributeProtocol {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef>;
    fn set_attr(&self, attr_name: &str, value: PyObjectRef);
    fn has_attr(&self, attr_name: &str) -> bool;
}

fn class_get_item(class: &PyObjectRef, attr_name: &str) -> Option<PyObjectRef> {
    let class = class.borrow();
    match class.kind {
        PyObjectKind::Class { ref dict, .. } => dict.get_item(attr_name),
        _ => panic!("Only classes should be in MRO!"),
    }
}

fn class_has_item(class: &PyObjectRef, attr_name: &str) -> bool {
    let class = class.borrow();
    match class.kind {
        PyObjectKind::Class { ref dict, .. } => dict.contains_key(attr_name),
        _ => panic!("Only classes should be in MRO!"),
    }
}

impl AttributeProtocol for PyObjectRef {
    fn get_attr(&self, attr_name: &str) -> Option<PyObjectRef> {
        let obj = self.borrow();
        match obj.kind {
            PyObjectKind::Module { ref dict, .. } => dict.get_item(attr_name),
            PyObjectKind::Class { ref mro, .. } => {
                if let Some(item) = class_get_item(self, attr_name) {
                    return Some(item);
                }
                for ref class in mro {
                    if let Some(item) = class_get_item(class, attr_name) {
                        return Some(item);
                    }
                }
                None
            }
            PyObjectKind::Instance { ref dict } => dict.get_item(attr_name),
            _ => None,
        }
    }

    fn has_attr(&self, attr_name: &str) -> bool {
        let obj = self.borrow();
        match obj.kind {
            PyObjectKind::Module { name: _, ref dict } => dict.contains_key(attr_name),
            PyObjectKind::Class { ref mro, .. } => {
                class_has_item(self, attr_name)
                    || mro.into_iter().any(|d| class_has_item(d, attr_name))
            }
            PyObjectKind::Instance { ref dict } => dict.contains_key(attr_name),
            _ => false,
        }
    }

    fn set_attr(&self, attr_name: &str, value: PyObjectRef) {
        match self.borrow().kind {
            PyObjectKind::Instance { ref dict } => dict.set_item(attr_name, value),
            PyObjectKind::Class {
                name: _,
                ref dict,
                mro: _,
            } => dict.set_item(attr_name, value),
            ref kind => unimplemented!("set_attr unimplemented for: {:?}", kind),
        };
    }
}

pub trait DictProtocol {
    fn contains_key(&self, k: &str) -> bool;
    fn get_item(&self, k: &str) -> Option<PyObjectRef>;
    fn set_item(&self, k: &str, v: PyObjectRef);
}

impl DictProtocol for PyObjectRef {
    fn contains_key(&self, k: &str) -> bool {
        match self.borrow().kind {
            PyObjectKind::Dict { ref elements } => elements.contains_key(k),
            PyObjectKind::Module { name: _, ref dict } => dict.contains_key(k),
            PyObjectKind::Scope { ref scope } => scope.locals.contains_key(k),
            ref kind => unimplemented!("TODO {:?}", kind),
        }
    }

    fn get_item(&self, k: &str) -> Option<PyObjectRef> {
        match self.borrow().kind {
            PyObjectKind::Dict { ref elements } => match elements.get(k) {
                Some(v) => Some(v.clone()),
                None => None,
            },
            PyObjectKind::Module { name: _, ref dict } => dict.get_item(k),
            PyObjectKind::Scope { ref scope } => scope.locals.get_item(k),
            _ => panic!("TODO"),
        }
    }

    fn set_item(&self, k: &str, v: PyObjectRef) {
        match self.borrow_mut().kind {
            PyObjectKind::Dict {
                elements: ref mut el,
            } => {
                el.insert(k.to_string(), v);
            }
            PyObjectKind::Module {
                name: _,
                ref mut dict,
            } => dict.set_item(k, v),
            PyObjectKind::Scope { ref mut scope } => {
                scope.locals.set_item(k, v);
            }
            ref k => panic!("TODO {:?}", k),
        };
    }
}

impl fmt::Debug for PyObject {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[PyObj {:?}]", self.kind)
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
        PyFuncArgs {
            args: args,
            kwargs: kwargs,
        }
    }

    pub fn insert(&self, item: PyObjectRef) -> PyFuncArgs {
        let mut args = PyFuncArgs {
            args: self.args.clone(),
            kwargs: self.kwargs.clone(),
        };
        args.args.insert(0, item);
        return args;
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
}

type RustPyFunc = fn(vm: &mut VirtualMachine, PyFuncArgs) -> PyResult;

/// Rather than determining the type of a python object, this enum is more
/// a holder for the rust payload of a python object. It is more a carrier
/// of rust data for a particular python object. Determine the python type
/// by using for example the `.typ()` method on a python object.
pub enum PyObjectKind {
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
        elements: HashMap<String, PyObjectRef>,
    },
    Set {
        elements: HashMap<usize, PyObjectRef>,
    },
    Iterator {
        position: usize,
        iterated_obj: PyObjectRef,
    },
    Slice {
        start: Option<i32>,
        stop: Option<i32>,
        step: Option<i32>,
    },
    Code {
        code: bytecode::CodeObject,
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
    Class {
        name: String,
        dict: PyObjectRef,
        mro: Vec<PyObjectRef>,
    },
    WeakRef {
        referent: PyObjectWeakRef,
    },
    Instance {
        dict: PyObjectRef,
    },
    RustFunction {
        function: RustPyFunc,
    },
}

impl fmt::Debug for PyObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &PyObjectKind::String { ref value } => write!(f, "str \"{}\"", value),
            &PyObjectKind::Integer { ref value } => write!(f, "int {}", value),
            &PyObjectKind::Float { ref value } => write!(f, "float {}", value),
            &PyObjectKind::Complex { ref value } => write!(f, "complex {}", value),
            &PyObjectKind::Bytes { ref value } => write!(f, "bytes/bytearray {:?}", value),
            &PyObjectKind::Sequence { elements: _ } => write!(f, "list or tuple"),
            &PyObjectKind::Dict { elements: _ } => write!(f, "dict"),
            &PyObjectKind::Set { elements: _ } => write!(f, "set"),
            &PyObjectKind::WeakRef { .. } => write!(f, "weakref"),
            &PyObjectKind::Iterator {
                position: _,
                iterated_obj: _,
            } => write!(f, "iterator"),
            &PyObjectKind::Slice {
                start: _,
                stop: _,
                step: _,
            } => write!(f, "slice"),
            &PyObjectKind::Code { ref code } => write!(f, "code: {:?}", code),
            &PyObjectKind::Function { .. } => write!(f, "function"),
            &PyObjectKind::Generator { .. } => write!(f, "generator"),
            &PyObjectKind::BoundMethod {
                ref function,
                ref object,
            } => write!(f, "bound-method: {:?} of {:?}", function, object),
            &PyObjectKind::Module { name: _, dict: _ } => write!(f, "module"),
            &PyObjectKind::Scope { scope: _ } => write!(f, "scope"),
            &PyObjectKind::None => write!(f, "None"),
            &PyObjectKind::Class {
                ref name,
                dict: _,
                mro: _,
            } => write!(f, "class {:?}", name),
            &PyObjectKind::Instance { dict: _ } => write!(f, "instance"),
            &PyObjectKind::RustFunction { function: _ } => write!(f, "rust function"),
        }
    }
}

impl PyObject {
    pub fn new(kind: PyObjectKind, /* dict: PyObjectRef,*/ typ: PyObjectRef) -> PyObjectRef {
        PyObject {
            kind: kind,
            typ: Some(typ),
            // dict: HashMap::new(),  // dict,
        }
        .into_ref()
    }

    /// Deprecated method, please call `vm.to_pystr`
    pub fn str(&self) -> String {
        match self.kind {
            PyObjectKind::String { ref value } => value.clone(),
            PyObjectKind::Integer { ref value } => format!("{:?}", value),
            PyObjectKind::Float { ref value } => format!("{:?}", value),
            PyObjectKind::Complex { ref value } => format!("{:?}", value),
            PyObjectKind::Bytes { ref value } => format!("b'{:?}'", value),
            PyObjectKind::Sequence { ref elements } => format!(
                "(/[{}]/)",
                elements
                    .iter()
                    .map(|elem| elem.borrow().str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PyObjectKind::Dict { ref elements } => format!(
                "{{ {} }}",
                elements
                    .iter()
                    .map(|elem| format!("{}: {}", elem.0, elem.1.borrow().str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PyObjectKind::Set { ref elements } => format!(
                "{{ {} }}",
                elements
                    .iter()
                    .map(|elem| elem.1.borrow().str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PyObjectKind::WeakRef { .. } => String::from("weakref"),
            PyObjectKind::None => String::from("None"),
            PyObjectKind::Class {
                ref name,
                dict: ref _dict,
                mro: _,
            } => format!("<class '{}'>", name),
            PyObjectKind::Instance { dict: _ } => format!("<instance>"),
            PyObjectKind::Code { code: _ } => format!("<code>"),
            PyObjectKind::Function { .. } => format!("<func>"),
            PyObjectKind::Generator { .. } => format!("<generator>"),
            PyObjectKind::BoundMethod { .. } => format!("<bound-method>"),
            PyObjectKind::RustFunction { function: _ } => format!("<rustfunc>"),
            PyObjectKind::Module { ref name, dict: _ } => format!("<module '{}'>", name),
            PyObjectKind::Scope { ref scope } => format!("<scope '{:?}'>", scope),
            PyObjectKind::Slice {
                ref start,
                ref stop,
                ref step,
            } => format!("<slice '{:?}:{:?}:{:?}'>", start, stop, step),
            PyObjectKind::Iterator {
                ref position,
                ref iterated_obj,
            } => format!(
                "<iter pos {} in {}>",
                position,
                iterated_obj.borrow_mut().str()
            ),
        }
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
