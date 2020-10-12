use crate::builtins::asyncgenerator;
use crate::builtins::builtinfunc;
use crate::builtins::bytearray;
use crate::builtins::bytes;
use crate::builtins::classmethod;
use crate::builtins::code;
use crate::builtins::complex;
use crate::builtins::coroutine;
use crate::builtins::dict;
use crate::builtins::enumerate;
use crate::builtins::filter;
use crate::builtins::float;
use crate::builtins::frame;
use crate::builtins::function;
use crate::builtins::generator;
use crate::builtins::getset;
use crate::builtins::int;
use crate::builtins::iter;
use crate::builtins::list;
use crate::builtins::map;
use crate::builtins::mappingproxy;
use crate::builtins::memory;
use crate::builtins::module;
use crate::builtins::namespace;
use crate::builtins::object;
use crate::builtins::property;
use crate::builtins::pybool;
use crate::builtins::pystr;
use crate::builtins::pysuper;
use crate::builtins::pytype::{self, PyType, PyTypeRef};
use crate::builtins::range;
use crate::builtins::set;
use crate::builtins::singletons;
use crate::builtins::slice;
use crate::builtins::staticmethod;
use crate::builtins::traceback;
use crate::builtins::tuple;
use crate::builtins::weakproxy;
use crate::builtins::weakref;
use crate::builtins::zip;
use crate::pyobject::{
    PyAttributes, PyClassDef, PyClassImpl, PyContext, PyObject, PyObjectRc, PyObjectRef,
};
use crate::slots::PyTypeSlots;
use rustpython_common::{lock::PyRwLock, rc::PyRc};
use std::mem::MaybeUninit;
use std::ptr;

/// Holder of references to builtin types.
#[derive(Debug)]
pub struct TypeZoo {
    pub async_generator: PyTypeRef,
    pub async_generator_asend: PyTypeRef,
    pub async_generator_athrow: PyTypeRef,
    pub async_generator_wrapped_value: PyTypeRef,
    pub bytes_type: PyTypeRef,
    pub bytes_iterator_type: PyTypeRef,
    pub bytearray_type: PyTypeRef,
    pub bytearray_iterator_type: PyTypeRef,
    pub bool_type: PyTypeRef,
    pub callable_iterator: PyTypeRef,
    pub classmethod_type: PyTypeRef,
    pub code_type: PyTypeRef,
    pub coroutine_type: PyTypeRef,
    pub coroutine_wrapper_type: PyTypeRef,
    pub dict_type: PyTypeRef,
    pub enumerate_type: PyTypeRef,
    pub filter_type: PyTypeRef,
    pub float_type: PyTypeRef,
    pub frame_type: PyTypeRef,
    pub frozenset_type: PyTypeRef,
    pub generator_type: PyTypeRef,
    pub int_type: PyTypeRef,
    pub iter_type: PyTypeRef,
    pub complex_type: PyTypeRef,
    pub list_type: PyTypeRef,
    pub list_iterator_type: PyTypeRef,
    pub list_reverseiterator_type: PyTypeRef,
    pub str_iterator_type: PyTypeRef,
    pub str_reverseiterator_type: PyTypeRef,
    pub dict_keyiterator_type: PyTypeRef,
    pub dict_reversekeyiterator_type: PyTypeRef,
    pub dict_valueiterator_type: PyTypeRef,
    pub dict_reversevalueiterator_type: PyTypeRef,
    pub dict_itemiterator_type: PyTypeRef,
    pub dict_reverseitemiterator_type: PyTypeRef,
    pub dict_keys_type: PyTypeRef,
    pub dict_values_type: PyTypeRef,
    pub dict_items_type: PyTypeRef,
    pub map_type: PyTypeRef,
    pub memoryview_type: PyTypeRef,
    pub tuple_type: PyTypeRef,
    pub tuple_iterator_type: PyTypeRef,
    pub set_type: PyTypeRef,
    pub set_iterator_type: PyTypeRef,
    pub staticmethod_type: PyTypeRef,
    pub super_type: PyTypeRef,
    pub str_type: PyTypeRef,
    pub range_type: PyTypeRef,
    pub range_iterator_type: PyTypeRef,
    pub slice_type: PyTypeRef,
    pub type_type: PyTypeRef,
    pub zip_type: PyTypeRef,
    pub function_type: PyTypeRef,
    pub builtin_function_or_method_type: PyTypeRef,
    pub method_descriptor_type: PyTypeRef,
    pub property_type: PyTypeRef,
    pub getset_type: PyTypeRef,
    pub module_type: PyTypeRef,
    pub namespace_type: PyTypeRef,
    pub bound_method_type: PyTypeRef,
    pub weakref_type: PyTypeRef,
    pub weakproxy_type: PyTypeRef,
    pub mappingproxy_type: PyTypeRef,
    pub traceback_type: PyTypeRef,
    pub object_type: PyTypeRef,
}

impl Default for TypeZoo {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeZoo {
    pub fn new() -> Self {
        let (type_type, object_type) = init_type_hierarchy();

        macro_rules! create_type {
            ($class:ty) => {
                <$class>::create_bare_type(&type_type, object_type.clone())
            };
            ($class:ty, $base:expr) => {
                <$class>::create_bare_type(&type_type, $base.clone())
            };
        }

        let int_type = create_type!(int::PyInt);
        Self {
            async_generator: create_type!(asyncgenerator::PyAsyncGen),
            async_generator_asend: create_type!(asyncgenerator::PyAsyncGenASend),
            async_generator_athrow: create_type!(asyncgenerator::PyAsyncGenAThrow),
            async_generator_wrapped_value: create_type!(asyncgenerator::PyAsyncGenWrappedValue),
            bool_type: create_type!(pybool::PyBool, int_type),
            bound_method_type: create_type!(function::PyBoundMethod),
            builtin_function_or_method_type: create_type!(builtinfunc::PyBuiltinFunction),
            bytearray_type: create_type!(bytearray::PyByteArray),
            bytearray_iterator_type: create_type!(bytearray::PyByteArrayIterator),
            bytes_type: create_type!(bytes::PyBytes),
            bytes_iterator_type: create_type!(bytes::PyBytesIterator),
            callable_iterator: create_type!(iter::PyCallableIterator),
            classmethod_type: create_type!(classmethod::PyClassMethod),
            code_type: create_type!(code::PyCodeRef),
            complex_type: create_type!(complex::PyComplex),
            coroutine_type: create_type!(coroutine::PyCoroutine),
            coroutine_wrapper_type: create_type!(coroutine::PyCoroutineWrapper),
            dict_type: create_type!(dict::PyDict),
            dict_keys_type: create_type!(dict::PyDictKeys),
            dict_values_type: create_type!(dict::PyDictValues),
            dict_items_type: create_type!(dict::PyDictItems),
            dict_keyiterator_type: create_type!(dict::PyDictKeyIterator),
            dict_reversekeyiterator_type: create_type!(dict::PyDictReverseKeyIterator),
            dict_valueiterator_type: create_type!(dict::PyDictValueIterator),
            dict_reversevalueiterator_type: create_type!(dict::PyDictReverseValueIterator),
            dict_itemiterator_type: create_type!(dict::PyDictItemIterator),
            dict_reverseitemiterator_type: create_type!(dict::PyDictReverseItemIterator),
            enumerate_type: create_type!(enumerate::PyEnumerate),
            filter_type: create_type!(filter::PyFilter),
            float_type: create_type!(float::PyFloat),
            frame_type: create_type!(crate::frame::FrameRef),
            frozenset_type: create_type!(set::PyFrozenSet),
            function_type: create_type!(function::PyFunction),
            generator_type: create_type!(generator::PyGenerator),
            getset_type: create_type!(getset::PyGetSet),
            int_type,
            iter_type: create_type!(iter::PySequenceIterator),
            list_type: create_type!(list::PyList),
            list_iterator_type: create_type!(list::PyListIterator),
            list_reverseiterator_type: create_type!(list::PyListReverseIterator),
            map_type: create_type!(map::PyMap),
            mappingproxy_type: create_type!(mappingproxy::PyMappingProxy),
            memoryview_type: create_type!(memory::PyMemoryView),
            module_type: create_type!(module::PyModule),
            namespace_type: create_type!(namespace::PyNamespace),
            property_type: create_type!(property::PyProperty),
            range_type: create_type!(range::PyRange),
            range_iterator_type: create_type!(range::PyRangeIterator),
            set_type: create_type!(set::PySet),
            set_iterator_type: create_type!(set::PySetIterator),
            slice_type: create_type!(slice::PySlice),
            staticmethod_type: create_type!(staticmethod::PyStaticMethod),
            str_type: create_type!(pystr::PyStr),
            str_iterator_type: create_type!(pystr::PyStrIterator),
            str_reverseiterator_type: create_type!(pystr::PyStrReverseIterator),
            super_type: create_type!(pysuper::PySuper),
            traceback_type: create_type!(traceback::PyTraceback),
            tuple_type: create_type!(tuple::PyTuple),
            tuple_iterator_type: create_type!(tuple::PyTupleIterator),
            weakproxy_type: create_type!(weakproxy::PyWeakProxy),
            weakref_type: create_type!(weakref::PyWeak),
            method_descriptor_type: create_type!(builtinfunc::PyBuiltinMethod),
            zip_type: create_type!(zip::PyZip),
            type_type,
            object_type,
        }
    }
}

pub fn create_type(name: &str, type_type: &PyTypeRef, base: PyTypeRef) -> PyTypeRef {
    create_type_with_slots(name, type_type, base, Default::default())
}

pub fn create_type_with_slots(
    name: &str,
    type_type: &PyTypeRef,
    base: PyTypeRef,
    slots: PyTypeSlots,
) -> PyTypeRef {
    let dict = PyAttributes::new();
    pytype::new(
        type_type.clone(),
        name,
        base.clone(),
        vec![base],
        dict,
        slots,
    )
    .expect("Failed to create a new type in internal code.")
}

/// Paritally initialize a struct, ensuring that all fields are
/// either given values or explicitly left uninitialized
macro_rules! partially_init {
    (
        $ty:path {$($init_field:ident: $init_value:expr),*$(,)?},
        Uninit { $($uninit_field:ident),*$(,)? }$(,)?
    ) => {{
        // check all the fields are there but *don't* actually run it
        if false {
            #[allow(invalid_value, dead_code, unreachable_code)]
            let _ = {$ty {
                $($init_field: $init_value,)*
                $($uninit_field: unreachable!(),)*
            }};
        }
        let mut m = ::std::mem::MaybeUninit::<$ty>::uninit();
        unsafe {
            $(::std::ptr::write(&mut (*m.as_mut_ptr()).$init_field, $init_value);)*
        }
        m
    }};
}

fn init_type_hierarchy() -> (PyTypeRef, PyTypeRef) {
    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        type PyTypeObj = PyObject<PyType>;
        type UninitRef<T> = PyRwLock<PyRc<MaybeUninit<PyObject<T>>>>;

        let type_payload = PyType {
            name: PyTypeRef::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::new()),
            slots: PyType::make_slots(),
        };
        let object_payload = PyType {
            name: object::PyBaseObject::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::new()),
            slots: object::PyBaseObject::make_slots(),
        };
        let type_type: PyRc<MaybeUninit<PyTypeObj>> = PyRc::new(partially_init!(
            PyObject::<PyType> {
                dict: None,
                payload: type_payload,
            },
            Uninit { typ }
        ));
        let object_type: PyRc<MaybeUninit<PyTypeObj>> = PyRc::new(partially_init!(
            PyObject::<PyType> {
                dict: None,
                payload: object_payload,
            },
            Uninit { typ },
        ));

        let object_type_ptr =
            PyRc::into_raw(object_type) as *mut MaybeUninit<PyTypeObj> as *mut PyTypeObj;
        let type_type_ptr =
            PyRc::into_raw(type_type.clone()) as *mut MaybeUninit<PyTypeObj> as *mut PyTypeObj;

        unsafe {
            ptr::write(
                &mut (*object_type_ptr).typ as *mut PyRwLock<PyObjectRc<PyType>>
                    as *mut UninitRef<PyType>,
                PyRwLock::new(type_type.clone()),
            );
            ptr::write(
                &mut (*type_type_ptr).typ as *mut PyRwLock<PyObjectRc<PyType>>
                    as *mut UninitRef<PyType>,
                PyRwLock::new(type_type),
            );

            let type_type = PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(type_type_ptr));
            let object_type = PyTypeRef::from_obj_unchecked(PyObjectRef::from_raw(object_type_ptr));

            (*type_type_ptr).payload.mro = vec![object_type.clone()];
            (*type_type_ptr).payload.bases = vec![object_type.clone()];
            (*type_type_ptr).payload.base = Some(object_type.clone());

            (type_type, object_type)
        }
    };

    object_type
        .subclasses
        .write()
        .push(weakref::PyWeak::downgrade(&type_type.as_object()));

    (type_type, object_type)
}

/// Fill attributes of builtin types.
pub fn initialize_types(context: &PyContext) {
    pytype::init(&context);
    object::init(&context);
    list::init(&context);
    set::init(&context);
    tuple::init(&context);
    dict::init(&context);
    builtinfunc::init(&context);
    function::init(&context);
    staticmethod::init(&context);
    classmethod::init(&context);
    generator::init(&context);
    coroutine::init(&context);
    asyncgenerator::init(&context);
    int::init(&context);
    float::init(&context);
    complex::init(&context);
    bytes::init(&context);
    bytearray::init(&context);
    property::init(&context);
    getset::init(&context);
    memory::init(&context);
    pystr::init(&context);
    range::init(&context);
    slice::init(&context);
    pysuper::init(&context);
    iter::init(&context);
    enumerate::init(&context);
    filter::init(&context);
    map::init(&context);
    zip::init(&context);
    pybool::init(&context);
    code::init(&context);
    frame::init(&context);
    weakref::init(&context);
    weakproxy::init(&context);
    singletons::init(&context);
    module::init(&context);
    namespace::init(&context);
    mappingproxy::init(&context);
    traceback::init(&context);
}
