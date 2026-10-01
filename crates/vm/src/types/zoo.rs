use crate::{
    Py,
    builtins::{
        asyncgenerator, bool_, builtin_func, bytearray, bytes, capsule, classmethod, code, complex,
        coroutine, descriptor, dict, enumerate, filter, float, frame, frame_locals_proxy, function,
        generator, genericalias, getset, int, interpolation, iter, list, map, mappingproxy, memory,
        module, namespace, object, property, pystr, range, set, singletons, slice, staticmethod,
        super_, template, traceback, tuple,
        type_::{self, PyType},
        union_, weakproxy, weakref, zip,
    },
    class::StaticType,
    vm::Context,
};

/// Holder of references to builtin types.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TypeZoo {
    pub async_generator: &'static Py<PyType>,
    pub async_generator_asend: &'static Py<PyType>,
    pub async_generator_athrow: &'static Py<PyType>,
    pub async_generator_wrapped_value: &'static Py<PyType>,
    pub anext_awaitable: &'static Py<PyType>,
    pub bytes_type: &'static Py<PyType>,
    pub bytes_iterator_type: &'static Py<PyType>,
    pub bytearray_type: &'static Py<PyType>,
    pub bytearray_iterator_type: &'static Py<PyType>,
    pub bool_type: &'static Py<PyType>,
    pub callable_iterator: &'static Py<PyType>,
    pub capsule_type: &'static Py<PyType>,
    pub cell_type: &'static Py<PyType>,
    pub classmethod_type: &'static Py<PyType>,
    pub code_type: &'static Py<PyType>,
    pub coroutine_type: &'static Py<PyType>,
    pub coroutine_wrapper_type: &'static Py<PyType>,
    pub dict_type: &'static Py<PyType>,
    pub enumerate_type: &'static Py<PyType>,
    pub filter_type: &'static Py<PyType>,
    pub float_type: &'static Py<PyType>,
    pub frame_type: &'static Py<PyType>,
    pub frame_locals_proxy_type: &'static Py<PyType>,
    pub frozenset_type: &'static Py<PyType>,
    pub generator_type: &'static Py<PyType>,
    pub int_type: &'static Py<PyType>,
    pub iter_type: &'static Py<PyType>,
    pub reverse_iter_type: &'static Py<PyType>,
    pub complex_type: &'static Py<PyType>,
    pub list_type: &'static Py<PyType>,
    pub list_iterator_type: &'static Py<PyType>,
    pub list_reverseiterator_type: &'static Py<PyType>,
    pub str_iterator_type: &'static Py<PyType>,
    pub dict_keyiterator_type: &'static Py<PyType>,
    pub dict_reversekeyiterator_type: &'static Py<PyType>,
    pub dict_valueiterator_type: &'static Py<PyType>,
    pub dict_reversevalueiterator_type: &'static Py<PyType>,
    pub dict_itemiterator_type: &'static Py<PyType>,
    pub dict_reverseitemiterator_type: &'static Py<PyType>,
    pub dict_keys_type: &'static Py<PyType>,
    pub dict_values_type: &'static Py<PyType>,
    pub dict_items_type: &'static Py<PyType>,
    pub map_type: &'static Py<PyType>,
    pub memoryview_type: &'static Py<PyType>,
    pub memoryviewiterator_type: &'static Py<PyType>,
    pub tuple_type: &'static Py<PyType>,
    pub tuple_iterator_type: &'static Py<PyType>,
    pub set_type: &'static Py<PyType>,
    pub set_iterator_type: &'static Py<PyType>,
    pub staticmethod_type: &'static Py<PyType>,
    pub super_type: &'static Py<PyType>,
    pub str_type: &'static Py<PyType>,
    pub range_type: &'static Py<PyType>,
    pub range_iterator_type: &'static Py<PyType>,
    pub long_range_iterator_type: &'static Py<PyType>,
    pub slice_type: &'static Py<PyType>,
    pub type_type: &'static Py<PyType>,
    pub zip_type: &'static Py<PyType>,
    pub function_type: &'static Py<PyType>,
    pub builtin_function_or_method_type: &'static Py<PyType>,
    pub builtin_method_type: &'static Py<PyType>,
    pub method_descriptor_type: &'static Py<PyType>,
    pub classmethod_descriptor_type: &'static Py<PyType>,
    pub property_type: &'static Py<PyType>,
    pub getset_type: &'static Py<PyType>,
    pub module_type: &'static Py<PyType>,
    pub namespace_type: &'static Py<PyType>,
    pub bound_method_type: &'static Py<PyType>,
    pub weakref_type: &'static Py<PyType>,
    pub weakproxy_type: &'static Py<PyType>,
    pub weakcallableproxy_type: &'static Py<PyType>,
    pub mappingproxy_type: &'static Py<PyType>,
    pub traceback_type: &'static Py<PyType>,
    pub object_type: &'static Py<PyType>,
    pub ellipsis_type: &'static Py<PyType>,
    pub none_type: &'static Py<PyType>,
    pub typing_no_default_type: &'static Py<PyType>,
    pub not_implemented_type: &'static Py<PyType>,
    pub generic_alias_type: &'static Py<PyType>,
    pub generic_alias_iterator_type: &'static Py<PyType>,
    pub union_type: &'static Py<PyType>,
    pub interpolation_type: &'static Py<PyType>,
    pub template_type: &'static Py<PyType>,
    pub template_iter_type: &'static Py<PyType>,
    pub member_descriptor_type: &'static Py<PyType>,
    pub wrapper_descriptor_type: &'static Py<PyType>,
    pub method_wrapper_type: &'static Py<PyType>,

    // RustPython-original types
    pub method_def: &'static Py<PyType>,
}

impl TypeZoo {
    #[cold]
    pub(crate) fn init() -> (Self, crate::builtins::PyTupleRef) {
        let hierarchy = crate::object::init_type_hierarchy();
        // These core types must be published before any other static type is created.
        let type_type = type_::PyType::init_manually(hierarchy.type_type);
        let object_type = object::PyBaseObject::init_manually(hierarchy.object_type);
        let tuple_type = tuple::PyTuple::init_manually(hierarchy.tuple_type);
        let weakref_type = weakref::PyWeak::init_manually(hierarchy.weakref_type);
        let int_type = unsafe { int::PyInt::init_builtin_type() };

        // builtin_function_or_method and builtin_method share the same type (CPython behavior)
        let builtin_function_or_method_type =
            unsafe { builtin_func::PyNativeFunction::init_builtin_type() };

        let types = Self {
            type_type,
            object_type,
            weakref_type,
            int_type,

            // types exposed as builtins
            bool_type: unsafe { bool_::PyBool::init_builtin_type() },
            bytearray_type: unsafe { bytearray::PyByteArray::init_builtin_type() },
            bytes_type: unsafe { bytes::PyBytes::init_builtin_type() },
            classmethod_type: unsafe { classmethod::PyClassMethod::init_builtin_type() },
            complex_type: unsafe { complex::PyComplex::init_builtin_type() },
            dict_type: unsafe { dict::PyDict::init_builtin_type() },
            enumerate_type: unsafe { enumerate::PyEnumerate::init_builtin_type() },
            float_type: unsafe { float::PyFloat::init_builtin_type() },
            frozenset_type: unsafe { set::PyFrozenSet::init_builtin_type() },
            filter_type: unsafe { filter::PyFilter::init_builtin_type() },
            list_type: unsafe { list::PyList::init_builtin_type() },
            map_type: unsafe { map::PyMap::init_builtin_type() },
            memoryview_type: unsafe { memory::PyMemoryView::init_builtin_type() },
            property_type: unsafe { property::PyProperty::init_builtin_type() },
            range_type: unsafe { range::PyRange::init_builtin_type() },
            set_type: unsafe { set::PySet::init_builtin_type() },
            slice_type: unsafe { slice::PySlice::init_builtin_type() },
            staticmethod_type: unsafe { staticmethod::PyStaticMethod::init_builtin_type() },
            str_type: unsafe { pystr::PyStr::init_builtin_type() },
            super_type: unsafe { super_::PySuper::init_builtin_type() },
            tuple_type,
            zip_type: unsafe { zip::PyZip::init_builtin_type() },

            // hidden internal types. is this really need to be cached here?
            async_generator: unsafe { asyncgenerator::PyAsyncGen::init_builtin_type() },
            async_generator_asend: unsafe { asyncgenerator::PyAsyncGenASend::init_builtin_type() },
            async_generator_athrow: unsafe {
                asyncgenerator::PyAsyncGenAThrow::init_builtin_type()
            },
            async_generator_wrapped_value: unsafe {
                asyncgenerator::PyAsyncGenWrappedValue::init_builtin_type()
            },
            anext_awaitable: unsafe { asyncgenerator::PyAnextAwaitable::init_builtin_type() },
            bound_method_type: unsafe { function::PyBoundMethod::init_builtin_type() },
            builtin_function_or_method_type,
            builtin_method_type: builtin_function_or_method_type,
            bytearray_iterator_type: unsafe { bytearray::PyByteArrayIterator::init_builtin_type() },
            bytes_iterator_type: unsafe { bytes::PyBytesIterator::init_builtin_type() },
            callable_iterator: unsafe { iter::PyCallableIterator::init_builtin_type() },
            capsule_type: unsafe { capsule::PyCapsule::init_builtin_type() },
            cell_type: unsafe { function::PyCell::init_builtin_type() },
            code_type: unsafe { code::PyCode::init_builtin_type() },
            coroutine_type: unsafe { coroutine::PyCoroutine::init_builtin_type() },
            coroutine_wrapper_type: unsafe { coroutine::PyCoroutineWrapper::init_builtin_type() },
            dict_keys_type: unsafe { dict::PyDictKeys::init_builtin_type() },
            dict_values_type: unsafe { dict::PyDictValues::init_builtin_type() },
            dict_items_type: unsafe { dict::PyDictItems::init_builtin_type() },
            dict_keyiterator_type: unsafe { dict::PyDictKeyIterator::init_builtin_type() },
            dict_reversekeyiterator_type: unsafe {
                dict::PyDictReverseKeyIterator::init_builtin_type()
            },
            dict_valueiterator_type: unsafe { dict::PyDictValueIterator::init_builtin_type() },
            dict_reversevalueiterator_type: unsafe {
                dict::PyDictReverseValueIterator::init_builtin_type()
            },
            dict_itemiterator_type: unsafe { dict::PyDictItemIterator::init_builtin_type() },
            dict_reverseitemiterator_type: unsafe {
                dict::PyDictReverseItemIterator::init_builtin_type()
            },
            ellipsis_type: unsafe { slice::PyEllipsis::init_builtin_type() },
            frame_type: unsafe { crate::frame::FrameObject::init_builtin_type() },
            frame_locals_proxy_type: unsafe {
                frame_locals_proxy::FrameLocalsProxy::init_builtin_type()
            },
            function_type: unsafe { function::PyFunction::init_builtin_type() },
            generator_type: unsafe { generator::PyGenerator::init_builtin_type() },
            getset_type: unsafe { getset::PyGetSet::init_builtin_type() },
            iter_type: unsafe { iter::PySequenceIterator::init_builtin_type() },
            reverse_iter_type: unsafe { enumerate::PyReverseSequenceIterator::init_builtin_type() },
            list_iterator_type: unsafe { list::PyListIterator::init_builtin_type() },
            list_reverseiterator_type: unsafe { list::PyListReverseIterator::init_builtin_type() },
            mappingproxy_type: unsafe { mappingproxy::PyMappingProxy::init_builtin_type() },
            memoryviewiterator_type: unsafe { memory::PyMemoryViewIterator::init_builtin_type() },
            module_type: unsafe { module::PyModule::init_builtin_type() },
            namespace_type: unsafe { namespace::PyNamespace::init_builtin_type() },
            range_iterator_type: unsafe { range::PyRangeIterator::init_builtin_type() },
            long_range_iterator_type: unsafe { range::PyLongRangeIterator::init_builtin_type() },
            set_iterator_type: unsafe { set::PySetIterator::init_builtin_type() },
            str_iterator_type: unsafe { pystr::PyStrIterator::init_builtin_type() },
            traceback_type: unsafe { traceback::PyTraceback::init_builtin_type() },
            tuple_iterator_type: unsafe { tuple::PyTupleIterator::init_builtin_type() },
            weakproxy_type: unsafe { weakproxy::PyWeakProxy::init_builtin_type() },
            weakcallableproxy_type: unsafe { weakproxy::PyWeakCallableProxy::init_builtin_type() },
            method_descriptor_type: unsafe { descriptor::PyMethodDescriptor::init_builtin_type() },
            classmethod_descriptor_type: unsafe {
                descriptor::PyClassMethodDescriptor::init_builtin_type()
            },
            none_type: unsafe { singletons::PyNone::init_builtin_type() },
            typing_no_default_type: unsafe {
                crate::stdlib::_typing::NoDefault::init_builtin_type()
            },
            not_implemented_type: unsafe { singletons::PyNotImplemented::init_builtin_type() },
            generic_alias_type: unsafe { genericalias::PyGenericAlias::init_builtin_type() },
            generic_alias_iterator_type: unsafe {
                genericalias::PyGenericAliasIterator::init_builtin_type()
            },
            union_type: unsafe { union_::PyUnion::init_builtin_type() },
            interpolation_type: unsafe { interpolation::PyInterpolation::init_builtin_type() },
            template_type: unsafe { template::PyTemplate::init_builtin_type() },
            template_iter_type: unsafe { template::PyTemplateIter::init_builtin_type() },
            member_descriptor_type: unsafe { descriptor::PyMemberDescriptor::init_builtin_type() },
            wrapper_descriptor_type: unsafe { descriptor::PyWrapper::init_builtin_type() },
            method_wrapper_type: unsafe { descriptor::PyMethodWrapper::init_builtin_type() },

            method_def: unsafe { crate::function::HeapMethodDef::init_builtin_type() },
        };
        (types, hierarchy.empty_tuple)
    }

    /// Fill attributes of builtin types.
    #[cold]
    pub(crate) fn extend(context: &'static Context) {
        // object must be initialized before type to set object.slots.init,
        // which type will inherit via inherit_slots()
        object::init(context);
        type_::init(context);
        list::init(context);
        set::init(context);
        tuple::init(context);
        dict::init(context);
        builtin_func::init(context);
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
        capsule::init(context);
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
        frame_locals_proxy::init(context);
        weakref::init(context);
        weakproxy::init(context);
        singletons::init(context);
        module::init(context);
        namespace::init(context);
        mappingproxy::init(context);
        traceback::init(context);
        genericalias::init(context);
        union_::init(context);
        interpolation::init(context);
        template::init(context);
        descriptor::init(context);
        crate::stdlib::_typing::init(context);

        // RustPython specific
        crate::function::method::init(context);
    }
}
