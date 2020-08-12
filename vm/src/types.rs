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
use crate::pyobject::{PyAttributes, PyContext, PyObject};
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
    pub bytesiterator_type: PyClassRef,
    pub bytearray_type: PyClassRef,
    pub bytearrayiterator_type: PyClassRef,
    pub bool_type: PyClassRef,
    pub callable_iterator: PyClassRef,
    pub classmethod_type: PyClassRef,
    pub code_type: PyClassRef,
    pub coroutine_type: PyClassRef,
    pub coroutine_wrapper_type: PyClassRef,
    pub dict_type: PyClassRef,
    pub enumerate_type: PyClassRef,
    pub filter_type: PyClassRef,
    pub float_type: PyClassRef,
    pub frame_type: PyClassRef,
    pub frozenset_type: PyClassRef,
    pub generator_type: PyClassRef,
    pub int_type: PyClassRef,
    pub iter_type: PyClassRef,
    pub complex_type: PyClassRef,
    pub list_type: PyClassRef,
    pub listiterator_type: PyClassRef,
    pub listreverseiterator_type: PyClassRef,
    pub striterator_type: PyClassRef,
    pub strreverseiterator_type: PyClassRef,
    pub dictkeyiterator_type: PyClassRef,
    pub dictvalueiterator_type: PyClassRef,
    pub dictitemiterator_type: PyClassRef,
    pub dictkeys_type: PyClassRef,
    pub dictvalues_type: PyClassRef,
    pub dictitems_type: PyClassRef,
    pub map_type: PyClassRef,
    pub memoryview_type: PyClassRef,
    pub tuple_type: PyClassRef,
    pub tupleiterator_type: PyClassRef,
    pub set_type: PyClassRef,
    pub setiterator_type: PyClassRef,
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
    pub method_descriptor_type: PyClassRef,
    pub property_type: PyClassRef,
    pub readonly_property_type: PyClassRef,
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

        let dict_type = create_type("dict", &type_type, &object_type);
        let module_type = create_type("module", &type_type, &object_type);
        let namespace_type = create_type("SimpleNamespace", &type_type, &object_type);
        let classmethod_type = create_type("classmethod", &type_type, &object_type);
        let staticmethod_type = create_type("staticmethod", &type_type, &object_type);
        let function_type = create_type("function", &type_type, &object_type);
        let builtin_function_or_method_type =
            create_type("builtin_function_or_method", &type_type, &object_type);
        let method_descriptor_type = create_type("method_descriptor", &type_type, &object_type);
        let property_type = create_type("property", &type_type, &object_type);
        let readonly_property_type = create_type("readonly_property", &type_type, &object_type);
        let getset_type = create_type("getset_descriptor", &type_type, &object_type);
        let super_type = create_type("super", &type_type, &object_type);
        let weakref_type = create_type("ref", &type_type, &object_type);
        let weakproxy_type = create_type("weakproxy", &type_type, &object_type);
        let generator_type = create_type("generator", &type_type, &object_type);
        let coroutine_type = create_type("coroutine", &type_type, &object_type);
        let coroutine_wrapper_type = create_type("coroutine_wrapper", &type_type, &object_type);
        let async_generator = create_type("async_generator", &type_type, &object_type);
        let async_generator_asend = create_type("async_generator_asend", &type_type, &object_type);
        let async_generator_athrow =
            create_type("async_generator_athrow", &type_type, &object_type);
        let async_generator_wrapped_value =
            create_type("async_generator_wrapped_value", &type_type, &object_type);
        let bound_method_type = create_type("method", &type_type, &object_type);
        let str_type = create_type("str", &type_type, &object_type);
        let list_type = create_type("list", &type_type, &object_type);
        let listiterator_type = create_type("list_iterator", &type_type, &object_type);
        let listreverseiterator_type =
            create_type("list_reverseiterator", &type_type, &object_type);
        let striterator_type = create_type("str_iterator", &type_type, &object_type);
        let strreverseiterator_type = create_type("str_reverseiterator", &type_type, &object_type);
        let dictkeys_type = create_type("dict_keys", &type_type, &object_type);
        let dictvalues_type = create_type("dict_values", &type_type, &object_type);
        let dictitems_type = create_type("dict_items", &type_type, &object_type);
        let dictkeyiterator_type = create_type("dict_keyiterator", &type_type, &object_type);
        let dictvalueiterator_type = create_type("dict_valueiterator", &type_type, &object_type);
        let dictitemiterator_type = create_type("dict_itemiterator", &type_type, &object_type);
        let set_type = create_type("set", &type_type, &object_type);
        let frozenset_type = create_type("frozenset", &type_type, &object_type);
        let setiterator_type = create_type("set_iterator", &type_type, &object_type);
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
        let traceback_type = create_type("traceback", &type_type, &object_type);
        let callable_iterator = create_type("callable_iterator", &type_type, &object_type);

        Self {
            async_generator,
            async_generator_asend,
            async_generator_athrow,
            async_generator_wrapped_value,
            bool_type,
            memoryview_type,
            bytearray_type,
            bytearrayiterator_type,
            bytes_type,
            bytesiterator_type,
            callable_iterator,
            code_type,
            coroutine_type,
            coroutine_wrapper_type,
            complex_type,
            classmethod_type,
            int_type,
            float_type,
            frame_type,
            staticmethod_type,
            list_type,
            listiterator_type,
            listreverseiterator_type,
            striterator_type,
            strreverseiterator_type,
            dictkeys_type,
            dictvalues_type,
            dictitems_type,
            dictkeyiterator_type,
            dictvalueiterator_type,
            dictitemiterator_type,
            set_type,
            frozenset_type,
            setiterator_type,
            tuple_type,
            tupleiterator_type,
            iter_type,
            enumerate_type,
            filter_type,
            map_type,
            zip_type,
            dict_type,
            str_type,
            range_type,
            rangeiterator_type,
            slice_type,
            object_type,
            function_type,
            builtin_function_or_method_type,
            method_descriptor_type,
            super_type,
            mappingproxy_type,
            property_type,
            readonly_property_type,
            getset_type,
            generator_type,
            module_type,
            namespace_type,
            bound_method_type,
            weakref_type,
            weakproxy_type,
            type_type,
            traceback_type,
        }
    }
}

pub fn create_type(name: &str, type_type: &PyClassRef, base: &PyClassRef) -> PyClassRef {
    let dict = PyAttributes::new();
    objtype::new(
        type_type.clone(),
        name,
        base.clone(),
        vec![base.clone()],
        dict,
        Default::default(),
        None,
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
                    name: String::from("type"),
                    bases: vec![],
                    mro: vec![],
                    subclasses: PyRwLock::default(),
                    attributes: PyRwLock::new(PyAttributes::new()),
                    slots: PyRwLock::default(),
                },
            },
            Uninit { typ }
        ));
        let object_type: PyRc<MaybeUninit<PyClassObj>> = PyRc::new(partially_init!(
            PyObject::<PyClass> {
                dict: None,
                payload: PyClass {
                    name: String::from("object"),
                    bases: vec![],
                    mro: vec![],
                    subclasses: PyRwLock::default(),
                    attributes: PyRwLock::new(PyAttributes::new()),
                    slots: PyRwLock::default(),
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
    objlist::init(&context);
    objset::init(&context);
    objtuple::init(&context);
    objobject::init(&context);
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
