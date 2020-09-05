use crate::obj::objasyncgenerator;
use crate::obj::objbool;
use crate::obj::objbuiltinfunc;
use crate::obj::objbytearray;
use crate::obj::objbytes;
use crate::obj::objclassmethod;
use crate::obj::objcode;
use crate::obj::objcomplex;
use crate::obj::objcoroutine;
use crate::obj::objdict;
use crate::obj::objellipsis;
use crate::obj::objenumerate;
use crate::obj::objfilter;
use crate::obj::objfloat;
use crate::obj::objframe;
use crate::obj::objfunction;
use crate::obj::objgenerator;
use crate::obj::objgetset;
use crate::obj::objint;
use crate::obj::objiter;
use crate::obj::objlist;
use crate::obj::objmap;
use crate::obj::objmappingproxy;
use crate::obj::objmemory;
use crate::obj::objmodule;
use crate::obj::objnamespace;
use crate::obj::objnone;
use crate::obj::objobject;
use crate::obj::objproperty;
use crate::obj::objrange;
use crate::obj::objset;
use crate::obj::objslice;
use crate::obj::objstaticmethod;
use crate::obj::objstr;
use crate::obj::objsuper;
use crate::obj::objtraceback;
use crate::obj::objtuple;
use crate::obj::objtype::{self, PyClass, PyClassRef};
use crate::obj::objweakproxy;
use crate::obj::objweakref;
use crate::obj::objzip;
use crate::pyobject::{PyAttributes, PyClassDef, PyClassImpl, PyContext, PyObject};
use crate::slots::PyClassSlots;
use rustpython_common::{cell::PyRwLock, rc::PyRc};
use std::mem::MaybeUninit;
use std::ptr;

/// Holder of references to builtin types.
#[derive(Debug)]
pub struct TypeZoo {
    pub async_generator: PyClassRef,
    pub async_generator_asend: PyClassRef,
    pub async_generator_athrow: PyClassRef,
    pub async_generator_wrapped_value: PyClassRef,
    pub bytes_type: PyClassRef,
    pub bytes_iterator_type: PyClassRef,
    pub bytearray_type: PyClassRef,
    pub bytearray_iterator_type: PyClassRef,
    pub bool_type: PyClassRef,
    pub callable_iterator: PyClassRef,
    pub classmethod_type: PyClassRef,
    pub code_type: PyClassRef,
    pub coroutine_type: PyClassRef,
    pub coroutine_wrapper_type: PyClassRef,
    pub dict_type: PyClassRef,
    pub enumerate_type: PyClassRef,
    pub ellipsis_type: PyClassRef,
    pub filter_type: PyClassRef,
    pub float_type: PyClassRef,
    pub frame_type: PyClassRef,
    pub frozenset_type: PyClassRef,
    pub generator_type: PyClassRef,
    pub int_type: PyClassRef,
    pub iter_type: PyClassRef,
    pub complex_type: PyClassRef,
    pub list_type: PyClassRef,
    pub list_iterator_type: PyClassRef,
    pub list_reverseiterator_type: PyClassRef,
    pub str_iterator_type: PyClassRef,
    pub str_reverseiterator_type: PyClassRef,
    pub dict_keyiterator_type: PyClassRef,
    pub dict_valueiterator_type: PyClassRef,
    pub dict_itemiterator_type: PyClassRef,
    pub dict_keys_type: PyClassRef,
    pub dict_values_type: PyClassRef,
    pub dict_items_type: PyClassRef,
    pub map_type: PyClassRef,
    pub memoryview_type: PyClassRef,
    pub tuple_type: PyClassRef,
    pub tuple_iterator_type: PyClassRef,
    pub set_type: PyClassRef,
    pub set_iterator_type: PyClassRef,
    pub staticmethod_type: PyClassRef,
    pub super_type: PyClassRef,
    pub str_type: PyClassRef,
    pub range_type: PyClassRef,
    pub range_iterator_type: PyClassRef,
    pub slice_type: PyClassRef,
    pub type_type: PyClassRef,
    pub zip_type: PyClassRef,
    pub function_type: PyClassRef,
    pub builtin_function_or_method_type: PyClassRef,
    pub method_descriptor_type: PyClassRef,
    pub property_type: PyClassRef,
    pub getset_type: PyClassRef,
    pub module_type: PyClassRef,
    pub namespace_type: PyClassRef,
    pub bound_method_type: PyClassRef,
    pub weakref_type: PyClassRef,
    pub weakproxy_type: PyClassRef,
    pub mappingproxy_type: PyClassRef,
    pub traceback_type: PyClassRef,
    pub object_type: PyClassRef,
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

        let int_type = create_type!(objint::PyInt);
        Self {
            async_generator: create_type!(objasyncgenerator::PyAsyncGen),
            async_generator_asend: create_type!(objasyncgenerator::PyAsyncGenASend),
            async_generator_athrow: create_type!(objasyncgenerator::PyAsyncGenAThrow),
            async_generator_wrapped_value: create_type!(objasyncgenerator::PyAsyncGenWrappedValue),
            bool_type: create_type!(objbool::PyBool, int_type),
            bound_method_type: create_type!(objfunction::PyBoundMethod),
            builtin_function_or_method_type: create_type!(objbuiltinfunc::PyBuiltinFunction),
            bytearray_type: create_type!(objbytearray::PyByteArray),
            bytearray_iterator_type: create_type!(objbytearray::PyByteArrayIterator),
            bytes_type: create_type!(objbytes::PyBytes),
            bytes_iterator_type: create_type!(objbytes::PyBytesIterator),
            callable_iterator: create_type!(objiter::PyCallableIterator),
            classmethod_type: create_type!(objclassmethod::PyClassMethod),
            code_type: create_type!(objcode::PyCodeRef),
            complex_type: create_type!(objcomplex::PyComplex),
            coroutine_type: create_type!(objcoroutine::PyCoroutine),
            coroutine_wrapper_type: create_type!(objcoroutine::PyCoroutineWrapper),
            dict_type: create_type!(objdict::PyDict),
            dict_keys_type: create_type!(objdict::PyDictKeys),
            dict_values_type: create_type!(objdict::PyDictValues),
            dict_items_type: create_type!(objdict::PyDictItems),
            dict_keyiterator_type: create_type!(objdict::PyDictKeyIterator),
            dict_valueiterator_type: create_type!(objdict::PyDictValueIterator),
            dict_itemiterator_type: create_type!(objdict::PyDictItemIterator),
            ellipsis_type: create_type!(objellipsis::PyEllipsis),
            enumerate_type: create_type!(objenumerate::PyEnumerate),
            filter_type: create_type!(objfilter::PyFilter),
            float_type: create_type!(objfloat::PyFloat),
            frame_type: create_type!(crate::frame::FrameRef),
            frozenset_type: create_type!(objset::PyFrozenSet),
            function_type: create_type!(objfunction::PyFunction),
            generator_type: create_type!(objgenerator::PyGenerator),
            getset_type: create_type!(objgetset::PyGetSet),
            int_type,
            iter_type: create_type!(objiter::PySequenceIterator),
            list_type: create_type!(objlist::PyList),
            list_iterator_type: create_type!(objlist::PyListIterator),
            list_reverseiterator_type: create_type!(objlist::PyListReverseIterator),
            map_type: create_type!(objmap::PyMap),
            mappingproxy_type: create_type!(objmappingproxy::PyMappingProxy),
            memoryview_type: create_type!(objmemory::PyMemoryView),
            module_type: create_type!(objmodule::PyModuleRef),
            namespace_type: create_type!(objnamespace::PyNamespace),
            property_type: create_type!(objproperty::PyProperty),
            range_type: create_type!(objrange::PyRange),
            range_iterator_type: create_type!(objrange::PyRangeIterator),
            set_type: create_type!(objset::PySet),
            set_iterator_type: create_type!(objset::PySetIterator),
            slice_type: create_type!(objslice::PySlice),
            staticmethod_type: create_type!(objstaticmethod::PyStaticMethod),
            str_type: create_type!(objstr::PyString),
            str_iterator_type: create_type!(objstr::PyStringIterator),
            str_reverseiterator_type: create_type!(objstr::PyStringReverseIterator),
            super_type: create_type!(objsuper::PySuper),
            traceback_type: create_type!(objtraceback::PyTraceback),
            tuple_type: create_type!(objtuple::PyTuple),
            tuple_iterator_type: create_type!(objtuple::PyTupleIterator),
            weakproxy_type: create_type!(objweakproxy::PyWeakProxy),
            weakref_type: create_type!(objweakref::PyWeak),
            method_descriptor_type: create_type!(objbuiltinfunc::PyBuiltinMethod),
            zip_type: create_type!(objzip::PyZip),
            type_type,
            object_type,
        }
    }
}

pub fn create_type(name: &str, type_type: &PyClassRef, base: PyClassRef) -> PyClassRef {
    create_type_with_slots(name, type_type, base, Default::default())
}

pub fn create_type_with_slots(
    name: &str,
    type_type: &PyClassRef,
    base: PyClassRef,
    slots: PyClassSlots,
) -> PyClassRef {
    let dict = PyAttributes::new();
    objtype::new(
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

fn init_type_hierarchy() -> (PyClassRef, PyClassRef) {
    // `type` inherits from `object`
    // and both `type` and `object are instances of `type`.
    // to produce this circular dependency, we need an unsafe block.
    // (and yes, this will never get dropped. TODO?)
    let (type_type, object_type) = {
        type PyClassObj = PyObject<PyClass>;
        type UninitRef<T> = PyRwLock<PyRc<MaybeUninit<T>>>;

        let type_type: PyRc<MaybeUninit<PyClassObj>> = PyRc::new(partially_init!(
            PyObject::<PyClass> {
                dict: None,
                payload: PyClass {
                    name: PyClassRef::NAME.to_owned(),
                    base: None,
                    bases: vec![],
                    mro: vec![],
                    subclasses: PyRwLock::default(),
                    attributes: PyRwLock::new(PyAttributes::new()),
                    slots: objtype::PyClassRef::make_slots(),
                },
            },
            Uninit { typ }
        ));
        let object_type: PyRc<MaybeUninit<PyClassObj>> = PyRc::new(partially_init!(
            PyObject::<PyClass> {
                dict: None,
                payload: PyClass {
                    name: objobject::PyBaseObject::NAME.to_owned(),
                    base: None,
                    bases: vec![],
                    mro: vec![],
                    subclasses: PyRwLock::default(),
                    attributes: PyRwLock::new(PyAttributes::new()),
                    slots: objobject::PyBaseObject::make_slots(),
                },
            },
            Uninit { typ },
        ));

        let object_type_ptr =
            PyRc::into_raw(object_type) as *mut MaybeUninit<PyClassObj> as *mut PyClassObj;
        let type_type_ptr =
            PyRc::into_raw(type_type.clone()) as *mut MaybeUninit<PyClassObj> as *mut PyClassObj;

        unsafe {
            ptr::write(
                &mut (*object_type_ptr).typ as *mut PyRwLock<PyRc<PyClassObj>>
                    as *mut UninitRef<PyClassObj>,
                PyRwLock::new(type_type.clone()),
            );
            ptr::write(
                &mut (*type_type_ptr).typ as *mut PyRwLock<PyRc<PyClassObj>>
                    as *mut UninitRef<PyClassObj>,
                PyRwLock::new(type_type),
            );

            let type_type = PyClassRef::from_obj_unchecked(PyRc::from_raw(type_type_ptr));
            let object_type = PyClassRef::from_obj_unchecked(PyRc::from_raw(object_type_ptr));

            (*type_type_ptr).payload.mro = vec![object_type.clone()];
            (*type_type_ptr).payload.bases = vec![object_type.clone()];
            (*type_type_ptr).payload.base = Some(object_type.clone());

            (type_type, object_type)
        }
    };

    object_type
        .subclasses
        .write()
        .push(objweakref::PyWeak::downgrade(&type_type.as_object()));

    (type_type, object_type)
}

/// Fill attributes of builtin types.
pub fn initialize_types(context: &PyContext) {
    objtype::init(&context);
    objobject::init(&context);
    objlist::init(&context);
    objset::init(&context);
    objtuple::init(&context);
    objdict::init(&context);
    objbuiltinfunc::init(&context);
    objfunction::init(&context);
    objstaticmethod::init(&context);
    objclassmethod::init(&context);
    objgenerator::init(&context);
    objcoroutine::init(&context);
    objasyncgenerator::init(&context);
    objint::init(&context);
    objfloat::init(&context);
    objcomplex::init(&context);
    objbytes::init(&context);
    objbytearray::init(&context);
    objproperty::init(&context);
    objgetset::init(&context);
    objmemory::init(&context);
    objstr::init(&context);
    objrange::init(&context);
    objslice::init(&context);
    objsuper::init(&context);
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
    objtraceback::init(&context);
}
