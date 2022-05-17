use crate::{
    builtins::{
        asyncgenerator, bool_, builtinfunc, bytearray, bytes, classmethod, code, complex,
        coroutine, dict, enumerate, filter, float, frame, function, generator, genericalias,
        getset, int, iter, list, map, mappingproxy, memory, module, namespace, object, property,
        pystr, range, set, singletons, slice, staticmethod, super_, traceback, tuple,
        type_::{self, PyTypeRef},
        union_, weakproxy, weakref, zip,
    },
    class::StaticType,
    vm::Context,
};

/// Holder of references to builtin types.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TypeZoo {
    pub async_generator: &'static PyTypeRef,
    pub async_generator_asend: &'static PyTypeRef,
    pub async_generator_athrow: &'static PyTypeRef,
    pub async_generator_wrapped_value: &'static PyTypeRef,
    pub bytes_type: &'static PyTypeRef,
    pub bytes_iterator_type: &'static PyTypeRef,
    pub bytearray_type: &'static PyTypeRef,
    pub bytearray_iterator_type: &'static PyTypeRef,
    pub bool_type: &'static PyTypeRef,
    pub callable_iterator: &'static PyTypeRef,
    pub cell_type: &'static PyTypeRef,
    pub classmethod_type: &'static PyTypeRef,
    pub code_type: &'static PyTypeRef,
    pub coroutine_type: &'static PyTypeRef,
    pub coroutine_wrapper_type: &'static PyTypeRef,
    pub dict_type: &'static PyTypeRef,
    pub enumerate_type: &'static PyTypeRef,
    pub filter_type: &'static PyTypeRef,
    pub float_type: &'static PyTypeRef,
    pub frame_type: &'static PyTypeRef,
    pub frozenset_type: &'static PyTypeRef,
    pub generator_type: &'static PyTypeRef,
    pub int_type: &'static PyTypeRef,
    pub iter_type: &'static PyTypeRef,
    pub reverse_iter_type: &'static PyTypeRef,
    pub complex_type: &'static PyTypeRef,
    pub list_type: &'static PyTypeRef,
    pub list_iterator_type: &'static PyTypeRef,
    pub list_reverseiterator_type: &'static PyTypeRef,
    pub str_iterator_type: &'static PyTypeRef,
    pub dict_keyiterator_type: &'static PyTypeRef,
    pub dict_reversekeyiterator_type: &'static PyTypeRef,
    pub dict_valueiterator_type: &'static PyTypeRef,
    pub dict_reversevalueiterator_type: &'static PyTypeRef,
    pub dict_itemiterator_type: &'static PyTypeRef,
    pub dict_reverseitemiterator_type: &'static PyTypeRef,
    pub dict_keys_type: &'static PyTypeRef,
    pub dict_values_type: &'static PyTypeRef,
    pub dict_items_type: &'static PyTypeRef,
    pub map_type: &'static PyTypeRef,
    pub memoryview_type: &'static PyTypeRef,
    pub tuple_type: &'static PyTypeRef,
    pub tuple_iterator_type: &'static PyTypeRef,
    pub set_type: &'static PyTypeRef,
    pub set_iterator_type: &'static PyTypeRef,
    pub staticmethod_type: &'static PyTypeRef,
    pub super_type: &'static PyTypeRef,
    pub str_type: &'static PyTypeRef,
    pub range_type: &'static PyTypeRef,
    pub range_iterator_type: &'static PyTypeRef,
    pub longrange_iterator_type: &'static PyTypeRef,
    pub slice_type: &'static PyTypeRef,
    pub type_type: &'static PyTypeRef,
    pub zip_type: &'static PyTypeRef,
    pub function_type: &'static PyTypeRef,
    pub builtin_function_or_method_type: &'static PyTypeRef,
    pub method_descriptor_type: &'static PyTypeRef,
    pub property_type: &'static PyTypeRef,
    pub getset_type: &'static PyTypeRef,
    pub module_type: &'static PyTypeRef,
    pub namespace_type: &'static PyTypeRef,
    pub bound_method_type: &'static PyTypeRef,
    pub weakref_type: &'static PyTypeRef,
    pub weakproxy_type: &'static PyTypeRef,
    pub mappingproxy_type: &'static PyTypeRef,
    pub traceback_type: &'static PyTypeRef,
    pub object_type: &'static PyTypeRef,
    pub ellipsis_type: &'static PyTypeRef,
    pub none_type: &'static PyTypeRef,
    pub not_implemented_type: &'static PyTypeRef,
    pub generic_alias_type: &'static PyTypeRef,
    pub union_type: &'static PyTypeRef,
}

impl TypeZoo {
    #[cold]
    pub(crate) fn init() -> Self {
        let (type_type, object_type, weakref_type) = crate::object::init_type_hierarchy();
        Self {
            // the order matters for type, object, weakref, and int
            type_type: type_::PyType::init_manually(type_type),
            object_type: object::PyBaseObject::init_manually(object_type),
            weakref_type: weakref::PyWeak::init_manually(weakref_type),
            int_type: int::PyInt::init_bare_type(),

            // types exposed as builtins
            bool_type: bool_::PyBool::init_bare_type(),
            bytearray_type: bytearray::PyByteArray::init_bare_type(),
            bytes_type: bytes::PyBytes::init_bare_type(),
            classmethod_type: classmethod::PyClassMethod::init_bare_type(),
            complex_type: complex::PyComplex::init_bare_type(),
            dict_type: dict::PyDict::init_bare_type(),
            enumerate_type: enumerate::PyEnumerate::init_bare_type(),
            float_type: float::PyFloat::init_bare_type(),
            frozenset_type: set::PyFrozenSet::init_bare_type(),
            filter_type: filter::PyFilter::init_bare_type(),
            list_type: list::PyList::init_bare_type(),
            map_type: map::PyMap::init_bare_type(),
            memoryview_type: memory::PyMemoryView::init_bare_type(),
            property_type: property::PyProperty::init_bare_type(),
            range_type: range::PyRange::init_bare_type(),
            set_type: set::PySet::init_bare_type(),
            slice_type: slice::PySlice::init_bare_type(),
            staticmethod_type: staticmethod::PyStaticMethod::init_bare_type(),
            str_type: pystr::PyStr::init_bare_type(),
            super_type: super_::PySuper::init_bare_type(),
            tuple_type: tuple::PyTuple::init_bare_type(),
            zip_type: zip::PyZip::init_bare_type(),

            // hidden internal types. is this really need to be cached here?
            async_generator: asyncgenerator::PyAsyncGen::init_bare_type(),
            async_generator_asend: asyncgenerator::PyAsyncGenASend::init_bare_type(),
            async_generator_athrow: asyncgenerator::PyAsyncGenAThrow::init_bare_type(),
            async_generator_wrapped_value: asyncgenerator::PyAsyncGenWrappedValue::init_bare_type(),
            bound_method_type: function::PyBoundMethod::init_bare_type(),
            builtin_function_or_method_type: builtinfunc::PyBuiltinFunction::init_bare_type(),
            bytearray_iterator_type: bytearray::PyByteArrayIterator::init_bare_type(),
            bytes_iterator_type: bytes::PyBytesIterator::init_bare_type(),
            callable_iterator: iter::PyCallableIterator::init_bare_type(),
            cell_type: function::PyCell::init_bare_type(),
            code_type: code::PyCode::init_bare_type(),
            coroutine_type: coroutine::PyCoroutine::init_bare_type(),
            coroutine_wrapper_type: coroutine::PyCoroutineWrapper::init_bare_type(),
            dict_keys_type: dict::PyDictKeys::init_bare_type(),
            dict_values_type: dict::PyDictValues::init_bare_type(),
            dict_items_type: dict::PyDictItems::init_bare_type(),
            dict_keyiterator_type: dict::PyDictKeyIterator::init_bare_type(),
            dict_reversekeyiterator_type: dict::PyDictReverseKeyIterator::init_bare_type(),
            dict_valueiterator_type: dict::PyDictValueIterator::init_bare_type(),
            dict_reversevalueiterator_type: dict::PyDictReverseValueIterator::init_bare_type(),
            dict_itemiterator_type: dict::PyDictItemIterator::init_bare_type(),
            dict_reverseitemiterator_type: dict::PyDictReverseItemIterator::init_bare_type(),
            ellipsis_type: slice::PyEllipsis::init_bare_type(),
            frame_type: crate::frame::Frame::init_bare_type(),
            function_type: function::PyFunction::init_bare_type(),
            generator_type: generator::PyGenerator::init_bare_type(),
            getset_type: getset::PyGetSet::init_bare_type(),
            iter_type: iter::PySequenceIterator::init_bare_type(),
            reverse_iter_type: enumerate::PyReverseSequenceIterator::init_bare_type(),
            list_iterator_type: list::PyListIterator::init_bare_type(),
            list_reverseiterator_type: list::PyListReverseIterator::init_bare_type(),
            mappingproxy_type: mappingproxy::PyMappingProxy::init_bare_type(),
            module_type: module::PyModule::init_bare_type(),
            namespace_type: namespace::PyNamespace::init_bare_type(),
            range_iterator_type: range::PyRangeIterator::init_bare_type(),
            longrange_iterator_type: range::PyLongRangeIterator::init_bare_type(),
            set_iterator_type: set::PySetIterator::init_bare_type(),
            str_iterator_type: pystr::PyStrIterator::init_bare_type(),
            traceback_type: traceback::PyTraceback::init_bare_type(),
            tuple_iterator_type: tuple::PyTupleIterator::init_bare_type(),
            weakproxy_type: weakproxy::PyWeakProxy::init_bare_type(),
            method_descriptor_type: builtinfunc::PyBuiltinMethod::init_bare_type(),
            none_type: singletons::PyNone::init_bare_type(),
            not_implemented_type: singletons::PyNotImplemented::init_bare_type(),
            generic_alias_type: genericalias::PyGenericAlias::init_bare_type(),
            union_type: union_::PyUnion::init_bare_type(),
        }
    }

    /// Fill attributes of builtin types.
    #[cold]
    pub(crate) fn extend(context: &Context) {
        type_::init(context);
        object::init(context);
        list::init(context);
        set::init(context);
        tuple::init(context);
        dict::init(context);
        builtinfunc::init(context);
        function::init(context);
        staticmethod::init(context);
        classmethod::init(context);
        generator::init(context);
        coroutine::init(context);
        asyncgenerator::init(context);
        int::init(context);
        float::init(context);
        complex::init(context);
        bytes::init(context);
        bytearray::init(context);
        property::init(context);
        getset::init(context);
        memory::init(context);
        pystr::init(context);
        range::init(context);
        slice::init(context);
        super_::init(context);
        iter::init(context);
        enumerate::init(context);
        filter::init(context);
        map::init(context);
        zip::init(context);
        bool_::init(context);
        code::init(context);
        frame::init(context);
        weakref::init(context);
        weakproxy::init(context);
        singletons::init(context);
        module::init(context);
        namespace::init(context);
        mappingproxy::init(context);
        traceback::init(context);
        genericalias::init(context);
        union_::init(context);
    }
}
