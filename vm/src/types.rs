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
use crate::obj::objobject;
use crate::obj::objproperty;
use crate::obj::objrange;
use crate::obj::objset;
use crate::obj::objsingletons;
use crate::obj::objslice;
use crate::obj::objstaticmethod;
use crate::obj::objstr;
use crate::obj::objsuper;
use crate::obj::objtraceback;
use crate::obj::objtuple;
use crate::obj::objtype::{self, PyType, PyTypeRef};
use crate::obj::objweakproxy;
use crate::obj::objweakref;
use crate::obj::objzip;
use crate::pyobject::{
    PyAttributes, PyClassDef, PyClassImpl, PyContext, PyObject, PyObjectRc, PyObjectRef,
};
use crate::slots::PyTypeSlots;
use rustpython_common::{cell::PyRwLock, rc::PyRc};
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
            dict_reversekeyiterator_type: create_type!(objdict::PyDictReverseKeyIterator),
            dict_valueiterator_type: create_type!(objdict::PyDictValueIterator),
            dict_reversevalueiterator_type: create_type!(objdict::PyDictReverseValueIterator),
            dict_itemiterator_type: create_type!(objdict::PyDictItemIterator),
            dict_reverseitemiterator_type: create_type!(objdict::PyDictReverseItemIterator),
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
            module_type: create_type!(objmodule::PyModule),
            namespace_type: create_type!(objnamespace::PyNamespace),
            property_type: create_type!(objproperty::PyProperty),
            range_type: create_type!(objrange::PyRange),
            range_iterator_type: create_type!(objrange::PyRangeIterator),
            set_type: create_type!(objset::PySet),
            set_iterator_type: create_type!(objset::PySetIterator),
            slice_type: create_type!(objslice::PySlice),
            staticmethod_type: create_type!(objstaticmethod::PyStaticMethod),
            str_type: create_type!(objstr::PyStr),
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
            slots: objtype::PyType::make_slots(),
        };
        let object_payload = PyType {
            name: objobject::PyBaseObject::NAME.to_owned(),
            base: None,
            bases: vec![],
            mro: vec![],
            subclasses: PyRwLock::default(),
            attributes: PyRwLock::new(PyAttributes::new()),
            slots: objobject::PyBaseObject::make_slots(),
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
    objenumerate::init(&context);
    objfilter::init(&context);
    objmap::init(&context);
    objzip::init(&context);
    objbool::init(&context);
    objcode::init(&context);
    objframe::init(&context);
    objweakref::init(&context);
    objweakproxy::init(&context);
    objsingletons::init(&context);
    objmodule::init(&context);
    objnamespace::init(&context);
    objmappingproxy::init(&context);
    objtraceback::init(&context);
}
