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
use crate::pyobject::{PyAttributes, PyContext, StaticType};
use crate::slots::PyTypeSlots;

/// Holder of references to builtin types.
#[derive(Debug, Clone)]
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
    pub cell_type: PyTypeRef,
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
    pub ellipsis_type: PyTypeRef,
    pub none_type: PyTypeRef,
    pub not_implemented_type: PyTypeRef,
}

impl TypeZoo {
    pub(crate) fn init() -> Self {
        let (type_type, object_type) = crate::pyobjectrc::init_type_hierarchy();
        Self {
            // the order matters for type, object and int
            type_type: pytype::PyType::init_manually(type_type).clone(),
            object_type: object::PyBaseObject::init_manually(object_type).clone(),
            int_type: int::PyInt::init_bare_type().clone(),

            // types exposed as builtins
            bool_type: pybool::PyBool::init_bare_type().clone(),
            bytearray_type: bytearray::PyByteArray::init_bare_type().clone(),
            bytes_type: bytes::PyBytes::init_bare_type().clone(),
            classmethod_type: classmethod::PyClassMethod::init_bare_type().clone(),
            complex_type: complex::PyComplex::init_bare_type().clone(),
            dict_type: dict::PyDict::init_bare_type().clone(),
            enumerate_type: enumerate::PyEnumerate::init_bare_type().clone(),
            float_type: float::PyFloat::init_bare_type().clone(),
            frozenset_type: set::PyFrozenSet::init_bare_type().clone(),
            filter_type: filter::PyFilter::init_bare_type().clone(),
            list_type: list::PyList::init_bare_type().clone(),
            map_type: map::PyMap::init_bare_type().clone(),
            memoryview_type: memory::PyMemoryView::init_bare_type().clone(),
            property_type: property::PyProperty::init_bare_type().clone(),
            range_type: range::PyRange::init_bare_type().clone(),
            set_type: set::PySet::init_bare_type().clone(),
            slice_type: slice::PySlice::init_bare_type().clone(),
            staticmethod_type: staticmethod::PyStaticMethod::init_bare_type().clone(),
            str_type: pystr::PyStr::init_bare_type().clone(),
            super_type: pysuper::PySuper::init_bare_type().clone(),
            tuple_type: tuple::PyTuple::init_bare_type().clone(),
            zip_type: zip::PyZip::init_bare_type().clone(),

            // hidden internal types. is this really need to be cached here?
            async_generator: asyncgenerator::PyAsyncGen::init_bare_type().clone(),
            async_generator_asend: asyncgenerator::PyAsyncGenASend::init_bare_type().clone(),
            async_generator_athrow: asyncgenerator::PyAsyncGenAThrow::init_bare_type().clone(),
            async_generator_wrapped_value: asyncgenerator::PyAsyncGenWrappedValue::init_bare_type()
                .clone(),
            bound_method_type: function::PyBoundMethod::init_bare_type().clone(),
            builtin_function_or_method_type: builtinfunc::PyBuiltinFunction::init_bare_type()
                .clone(),
            bytearray_iterator_type: bytearray::PyByteArrayIterator::init_bare_type().clone(),
            bytes_iterator_type: bytes::PyBytesIterator::init_bare_type().clone(),
            callable_iterator: iter::PyCallableIterator::init_bare_type().clone(),
            cell_type: function::PyCell::init_bare_type().clone(),
            code_type: code::PyCode::init_bare_type().clone(),
            coroutine_type: coroutine::PyCoroutine::init_bare_type().clone(),
            coroutine_wrapper_type: coroutine::PyCoroutineWrapper::init_bare_type().clone(),
            dict_keys_type: dict::PyDictKeys::init_bare_type().clone(),
            dict_values_type: dict::PyDictValues::init_bare_type().clone(),
            dict_items_type: dict::PyDictItems::init_bare_type().clone(),
            dict_keyiterator_type: dict::PyDictKeyIterator::init_bare_type().clone(),
            dict_reversekeyiterator_type: dict::PyDictReverseKeyIterator::init_bare_type().clone(),
            dict_valueiterator_type: dict::PyDictValueIterator::init_bare_type().clone(),
            dict_reversevalueiterator_type: dict::PyDictReverseValueIterator::init_bare_type()
                .clone(),
            dict_itemiterator_type: dict::PyDictItemIterator::init_bare_type().clone(),
            dict_reverseitemiterator_type: dict::PyDictReverseItemIterator::init_bare_type()
                .clone(),
            ellipsis_type: slice::PyEllipsis::init_bare_type().clone(),
            frame_type: crate::frame::Frame::init_bare_type().clone(),
            function_type: function::PyFunction::init_bare_type().clone(),
            generator_type: generator::PyGenerator::init_bare_type().clone(),
            getset_type: getset::PyGetSet::init_bare_type().clone(),
            iter_type: iter::PySequenceIterator::init_bare_type().clone(),
            list_iterator_type: list::PyListIterator::init_bare_type().clone(),
            list_reverseiterator_type: list::PyListReverseIterator::init_bare_type().clone(),
            mappingproxy_type: mappingproxy::PyMappingProxy::init_bare_type().clone(),
            module_type: module::PyModule::init_bare_type().clone(),
            namespace_type: namespace::PyNamespace::init_bare_type().clone(),
            range_iterator_type: range::PyRangeIterator::init_bare_type().clone(),
            set_iterator_type: set::PySetIterator::init_bare_type().clone(),
            str_iterator_type: pystr::PyStrIterator::init_bare_type().clone(),
            str_reverseiterator_type: pystr::PyStrReverseIterator::init_bare_type().clone(),
            traceback_type: traceback::PyTraceback::init_bare_type().clone(),
            tuple_iterator_type: tuple::PyTupleIterator::init_bare_type().clone(),
            weakproxy_type: weakproxy::PyWeakProxy::init_bare_type().clone(),
            weakref_type: weakref::PyWeak::init_bare_type().clone(),
            method_descriptor_type: builtinfunc::PyBuiltinMethod::init_bare_type().clone(),
            none_type: singletons::PyNone::init_bare_type().clone(),
            not_implemented_type: singletons::PyNotImplemented::init_bare_type().clone(),
        }
    }

    /// Fill attributes of builtin types.
    pub(crate) fn extend(context: &PyContext) {
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
}

pub fn create_simple_type(name: &str, base: &PyTypeRef) -> PyTypeRef {
    create_type_with_slots(name, PyType::static_type(), base, Default::default())
}

pub fn create_type_with_slots(
    name: &str,
    type_type: &PyTypeRef,
    base: &PyTypeRef,
    slots: PyTypeSlots,
) -> PyTypeRef {
    let dict = PyAttributes::default();
    pytype::new(
        type_type.clone(),
        name,
        base.clone(),
        vec![base.clone()],
        dict,
        slots,
    )
    .expect("Failed to create a new type in internal code.")
}
