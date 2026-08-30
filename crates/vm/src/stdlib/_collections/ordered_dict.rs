// OrderedDict implementation
// cspell:ignore odict

#[pymodule(sub)]
pub(crate) mod ordered_dict {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{
            IterStatus::Active,
            PositionIterInternal, PyDict, PyGenericAlias, PyMappingProxy, PyStrRef, PyTuple,
            PyTupleRef, PyType, PyTypeRef,
            dict::{
                PyDictItems, set_inner_number_or, set_inner_number_subtract, set_inner_number_xor,
                set_item_view_number_xor, set_view_number_and,
            },
            iter::builtins_iter,
            locked_step,
        },
        common::{ascii, lock::PyMutex},
        dict_inner,
        function::{ArgIterable, IntoFuncArgs, KwArgs, OptionalArg, PyComparisonValue},
        object::{Traverse, TraverseFn},
        protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        types::{
            AsMapping, AsNumber, AsSequence, Callable, Comparable, Constructor, DefaultConstructor,
            Initializer, IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
        },
    };

    // ============================================================================

    #[pyattr]
    #[pyclass(
        module = "collections",
        name = "OrderedDict",
        base = PyDict,
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug, Default)]
    struct PyOrderedDict {
        inner: PyDict,
    }

    type PyOrderedDictRef = PyRef<PyOrderedDict>;

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyOrderedDict {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.inner.traverse(tracer_fn);
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            Traverse::clear(&mut self.inner, out);
        }
    }

    #[derive(FromArgs)]
    struct MoveToEndArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct PopItemArgs {
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct SetDefaultArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct PopArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct FromKeysArgs {
        #[pyarg(any)]
        iterable: ArgIterable,
        #[pyarg(any, optional)]
        value: OptionalArg<PyObjectRef>,
    }

    #[pyclass(
        flags(BASETYPE, MAPPING, HAS_DICT, HAS_WEAKREF),
        with(
            Constructor,
            Initializer,
            Comparable,
            Iterable,
            AsMapping,
            AsSequence,
            AsNumber,
            Representable
        )
    )]
    impl PyOrderedDict {
        /// Move an existing element to the end (or beginning if last is false).
        #[pymethod]
        fn move_to_end(&self, args: MoveToEndArgs, vm: &VirtualMachine) -> PyResult<()> {
            let entries = self.inner._as_dict_inner();
            if entries.move_to_end(vm, &*args.key, args.last)? {
                Ok(())
            } else {
                Err(vm.new_key_error(args.key))
            }
        }

        /// Remove and return a (key, value) pair from the dictionary.
        /// Pairs are returned in LIFO order if last is true or FIFO order if false.
        #[pymethod]
        fn popitem(
            &self,
            args: PopItemArgs,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let entries = self.inner._as_dict_inner();
            let result = if args.last {
                entries.pop_back() // LIFO - existing method
            } else {
                entries.pop_front() // FIFO - new method
            };
            result.ok_or_else(|| {
                let err_msg = vm.ctx.new_str(ascii!("dictionary is empty")).into();
                vm.new_key_error(err_msg)
            })
        }

        #[pymethod]
        fn setdefault(zelf: PyRef<Self>, args: SetDefaultArgs, vm: &VirtualMachine) -> PyResult {
            if let Some(value) = zelf.inner._as_dict_inner().get(vm, &*args.key)? {
                return Ok(value);
            }
            let value = args.default.unwrap_or_none(vm);
            zelf.as_object().set_item(&*args.key, value.clone(), vm)?;
            Ok(value)
        }

        #[pymethod]
        fn pop(&self, args: PopArgs, vm: &VirtualMachine) -> PyResult {
            match self.inner._as_dict_inner().pop(vm, &*args.key)? {
                Some(value) => Ok(value),
                None => args.default.ok_or_else(|| vm.new_key_error(args.key)),
            }
        }

        #[pymethod]
        fn get(
            &self,
            key: PyObjectRef,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            match self.inner._as_dict_inner().get(vm, &*key)? {
                Some(value) => Ok(value),
                None => Ok(default.unwrap_or_none(vm)),
            }
        }

        fn merge_object(
            target: &PyObject,
            other: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match other.get_attr(vm.ctx.intern_str("keys"), vm) {
                Ok(keys_method) => {
                    let keys = keys_method.call((), vm)?.get_iter(vm)?;
                    while let PyIterReturn::Return(key) = keys.next(vm)? {
                        let value = other.get_item(&*key, vm)?;
                        target.set_item(&*key, value, vm)?;
                    }
                }
                Err(exc) if exc.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                    let iter = other.get_iter(vm)?;
                    for (index, element) in iter.iter::<PyObjectRef>(vm)?.enumerate() {
                        let (key, value) = PyDict::update_sequence_pair(element?, index, vm)?;
                        target.set_item(&*key, value, vm)?;
                    }
                }
                Err(exc) => return Err(exc),
            }
            Ok(())
        }

        fn merge_kwargs(target: &PyObject, kwargs: KwArgs, vm: &VirtualMachine) -> PyResult<()> {
            for (key, value) in kwargs {
                target.set_item(&key, value, vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn update(
            zelf: PyRef<Self>,
            dict_obj: OptionalArg<PyObjectRef>,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let OptionalArg::Present(dict_obj) = dict_obj {
                Self::merge_object(zelf.as_object(), dict_obj, vm)?;
            }
            Self::merge_kwargs(zelf.as_object(), kwargs, vm)
        }

        #[pymethod]
        fn clear(&self) {
            self.inner._as_dict_inner().clear()
        }

        #[pymethod]
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let copied = PyType::call(zelf.class(), ().into_args(vm), vm)?;
            let entries = zelf.inner._as_dict_inner();
            let (size, mutation_version, keys) = entries.keys_versioned_snapshot();
            for key in keys {
                let value = zelf.as_object().get_item(&*key, vm)?;
                if entries.has_changed_size_or_version(&size, mutation_version) {
                    return Err(vm.new_runtime_error("OrderedDict mutated during iteration"));
                }
                copied.set_item(&*key, value, vm)?;
                if entries.has_changed_size_or_version(&size, mutation_version) {
                    return Err(vm.new_runtime_error("OrderedDict mutated during iteration"));
                }
            }
            Ok(copied)
        }

        #[pyclassmethod]
        fn fromkeys(class: PyTypeRef, args: FromKeysArgs, vm: &VirtualMachine) -> PyResult {
            let value = args.value.unwrap_or_none(vm);
            let d = PyType::call(&class, ().into(), vm)?;
            match d.downcast_exact::<Self>(vm) {
                Ok(ordered_dict) => {
                    for key in args.iterable.iter(vm)? {
                        let key: PyObjectRef = key?;
                        ordered_dict
                            .inner
                            ._as_dict_inner()
                            .insert(vm, &*key, value.clone())?;
                    }
                    Ok(ordered_dict.into_pyref().into())
                }
                Err(pyobj) => {
                    for key in args.iterable.iter(vm)? {
                        let key: PyObjectRef = key?;
                        pyobj.set_item(&*key, value.clone(), vm)?;
                    }
                    Ok(pyobj)
                }
            }
        }

        #[pymethod]
        fn __sizeof__(&self) -> usize {
            // Add overhead for OrderedDict's conceptual linked-list structure
            // In CPython, each entry has an additional _ODictNode with prev/next pointers
            let base_size = core::mem::size_of::<Self>() + self.inner._as_dict_inner().sizeof();
            // Add overhead: 2 pointers (prev, next) per entry + head/tail pointers
            let num_entries = self.inner._as_dict_inner().len();
            let pointer_size = core::mem::size_of::<usize>();
            let linked_list_overhead = 2 * pointer_size + num_entries * 2 * pointer_size;
            base_size + linked_list_overhead
        }

        /// Return a reverse iterator over the dict keys.
        #[pymethod]
        fn __reversed__(zelf: PyRef<Self>) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(zelf)
        }

        /// Return the OrderedDict keys view.
        #[pymethod]
        fn keys(zelf: PyRef<Self>) -> PyOrderedDictKeys {
            PyOrderedDictKeys { ordered_dict: zelf }
        }

        /// Return the OrderedDict values view.
        #[pymethod]
        fn values(zelf: PyRef<Self>) -> PyOrderedDictValues {
            PyOrderedDictValues { ordered_dict: zelf }
        }

        /// Return the OrderedDict items view.
        #[pymethod]
        fn items(zelf: PyRef<Self>) -> PyOrderedDictItems {
            PyOrderedDictItems { ordered_dict: zelf }
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let state = vm.call_method(zelf.as_object(), "__getstate__", ())?;
            let items = PyOrderedDictItemIterator::new(zelf.clone()).into_pyobject(vm);
            Ok(vm
                .new_tuple((
                    zelf.class().to_owned(),
                    vm.ctx.empty_tuple.clone(),
                    state,
                    vm.ctx.none(),
                    items,
                ))
                .into())
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl DefaultConstructor for PyOrderedDict {}

    impl Initializer for PyOrderedDict {
        type Args = (OptionalArg<PyObjectRef>, KwArgs);

        fn init(
            zelf: PyRef<Self>,
            (dict_obj, kwargs): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Do NOT clear existing data - just merge/update
            // This matches CPython behavior where __init__ updates existing dict
            // rather than replacing it

            // First add positional argument
            if let OptionalArg::Present(dict_obj) = dict_obj {
                Self::merge_object(zelf.as_object(), dict_obj, vm)?;
            }

            // Then add keyword arguments (in order)
            Self::merge_kwargs(zelf.as_object(), kwargs, vm)
        }
    }

    impl Comparable for PyOrderedDict {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            // Check for identity optimization
            if let Some(res) = op.identical_optimization(zelf, other) {
                return Ok(res.into());
            }

            // Order-sensitive comparison when comparing two OrderedDicts
            if let Some(other_ordered_dict) = other.downcast_ref::<Self>()
                && (op == PyComparisonOp::Eq || op == PyComparisonOp::Ne)
            {
                let self_entries = zelf.inner._as_dict_inner();
                let other_entries = other_ordered_dict.inner._as_dict_inner();
                let (self_size, self_version) = self_entries.versioned_size();
                let (other_size, other_version) = other_entries.versioned_size();
                let self_dict = zelf
                    .as_object()
                    .downcast_ref::<PyDict>()
                    .expect("OrderedDict must retain its dict base payload");
                let other_dict = other_ordered_dict
                    .as_object()
                    .downcast_ref::<PyDict>()
                    .expect("OrderedDict must retain its dict base payload");
                let mapping_equal = self_dict
                    .inner_cmp(other_dict, PyComparisonOp::Eq, true, vm)?
                    .unwrap();
                if !mapping_equal {
                    return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                }
                if self_entries.has_changed_size_or_version(&self_size, self_version)
                    || other_entries.has_changed_size_or_version(&other_size, other_version)
                {
                    return Err(vm.new_runtime_error("OrderedDict mutated during iteration"));
                }

                let mut self_position = 0;
                let mut other_position = 0;
                loop {
                    let self_key = self_entries
                        .next_entry_version_checked(
                            self_position,
                            &self_size,
                            self_version,
                            |key, _| key.clone(),
                        )
                        .map_err(|_| {
                            vm.new_runtime_error("OrderedDict mutated during iteration")
                        })?;
                    let other_key = other_entries
                        .next_entry_version_checked(
                            other_position,
                            &other_size,
                            other_version,
                            |key, _| key.clone(),
                        )
                        .map_err(|_| {
                            vm.new_runtime_error("OrderedDict mutated during iteration")
                        })?;
                    let (
                        Some((next_self_position, self_key)),
                        Some((next_other_position, other_key)),
                    ) = (self_key, other_key)
                    else {
                        break;
                    };
                    let keys_equal = vm.identical_or_equal(&self_key, &other_key)?;
                    if self_entries.has_changed_size_or_version(&self_size, self_version)
                        || other_entries.has_changed_size_or_version(&other_size, other_version)
                    {
                        return Err(vm.new_runtime_error("OrderedDict mutated during iteration"));
                    }
                    if !keys_equal {
                        return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                    }
                    self_position = next_self_position;
                    other_position = next_other_position;
                }
                return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Eq));
            }

            // Fall back to dict comparison (order-insensitive) for other types
            if let Some(other_dict) = other.downcast_ref::<PyDict>() {
                op.eq_only(|| {
                    let self_entries = zelf.inner._as_dict_inner();
                    let other_entries = other_dict._as_dict_inner();

                    if self_entries.len() != other_entries.len() {
                        return Ok(PyComparisonValue::Implemented(false));
                    }

                    for (k, v1) in self_entries.items() {
                        match other_entries.get(vm, &*k)? {
                            Some(v2) => {
                                if !vm.identical_or_equal(&v1, &v2)? {
                                    return Ok(PyComparisonValue::Implemented(false));
                                }
                            }
                            None => return Ok(PyComparisonValue::Implemented(false)),
                        }
                    }
                    Ok(PyComparisonValue::Implemented(true))
                })
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
        }
    }

    impl Iterable for PyOrderedDict {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictKeyIterator::new(zelf).into_pyobject(vm))
        }
    }

    impl AsMapping for PyOrderedDict {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyOrderedDict::mapping_downcast(mapping)
                    .inner
                    ._as_dict_inner()
                    .len())),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    match zelf.inner._as_dict_inner().get(vm, needle)? {
                        Some(value) => Ok(value),
                        None => match vm
                            .get_method(zelf.as_object().to_owned(), identifier!(vm, __missing__))
                        {
                            Some(method) => method?.call((needle.to_owned(),), vm),
                            None => Err(vm.new_key_error(needle.to_owned())),
                        },
                    }
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    if let Some(value) = value {
                        zelf.inner._as_dict_inner().insert(vm, needle, value)
                    } else {
                        zelf.inner._as_dict_inner().delete(vm, needle)
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyOrderedDict {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                contains: atomic_func!(|seq, target, vm| PyOrderedDict::sequence_downcast(seq)
                    .inner
                    ._as_dict_inner()
                    .contains(vm, target)),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl AsNumber for PyOrderedDict {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                // Handle both __or__ and __ror__ in the same function
                // This function is used for both `or` and `right_or` slots via copy_from
                or: Some(|a, b, vm| {
                    let a_is_ordered_dict = a.downcast_ref::<PyOrderedDict>().is_some();
                    let b_is_ordered_dict = b.downcast_ref::<PyOrderedDict>().is_some();
                    let a_is_dict = a.class().fast_issubclass(vm.ctx.types.dict_type);
                    let b_is_dict = b.class().fast_issubclass(vm.ctx.types.dict_type);

                    if a_is_ordered_dict {
                        // This is __or__: OrderedDict | other
                        // other must be a dict or dict subclass
                        if !b_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let result = PyType::call(a.class(), (a.to_owned(),).into_args(vm), vm)?;
                        PyOrderedDict::merge_object(&result, b.to_owned(), vm)?;
                        Ok(result)
                    } else if b_is_ordered_dict {
                        // This is __ror__: other | OrderedDict
                        // other must be a dict or dict subclass
                        if !a_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let result = PyType::call(b.class(), (a.to_owned(),).into_args(vm), vm)?;
                        PyOrderedDict::merge_object(&result, b.to_owned(), vm)?;
                        Ok(result)
                    } else {
                        Ok(vm.ctx.not_implemented())
                    }
                }),
                inplace_or: Some(|a, b, vm| {
                    if a.downcast_ref::<PyOrderedDict>().is_some() {
                        PyOrderedDict::merge_object(a, b.to_owned(), vm)?;
                        Ok(a.to_owned())
                    } else {
                        Ok(vm.ctx.not_implemented())
                    }
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Representable for PyOrderedDict {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let class = zelf.class();
            let class_name = class.name();

            if zelf.inner._as_dict_inner().len() == 0 {
                return Ok(format!("{class_name}()"));
            }

            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts = Vec::with_capacity(zelf.inner._as_dict_inner().len());
                for (key, value) in zelf.inner._as_dict_inner().items() {
                    let key_repr: PyStrRef = key.repr(vm)?;
                    let value_repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(format!("{key_repr}: {value_repr}"));
                }
                Ok(format!("{class_name}({{{}}})", str_parts.join(", ")))
            } else {
                // Recursion detected - return just "..." as CPython does
                Ok("...".to_owned())
            }
        }
    }

    // ============================================================================
    // OrderedDict Views
    // ============================================================================

    fn is_set_like_view(other: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
        if other.fast_isinstance(vm.ctx.types.set_type)
            || other.fast_isinstance(vm.ctx.types.frozenset_type)
            || other.class().is(vm.ctx.types.dict_keys_type)
            || other.class().is(vm.ctx.types.dict_items_type)
            || other.downcast_ref::<PyOrderedDictKeys>().is_some()
            || other.downcast_ref::<PyOrderedDictItems>().is_some()
        {
            return Ok(true);
        }
        let abc = vm.import("_collections_abc", 0)?;
        let set_abc = abc.get_attr("Set", vm)?;
        other.is_instance(&set_abc, vm)
    }

    fn set_like_view_cmp(
        self_len: usize,
        self_items: Vec<PyObjectRef>,
        self_contains: impl Fn(&PyObject, &VirtualMachine) -> PyResult<bool>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if op == PyComparisonOp::Ne {
            return set_like_view_cmp(
                self_len,
                self_items,
                self_contains,
                other,
                PyComparisonOp::Eq,
                vm,
            )
            .map(|result| result.map(|equal| !equal));
        }
        if !is_set_like_view(other, vm)? {
            return Ok(PyComparisonValue::NotImplemented);
        }

        let other_len = other.length(vm)?;
        if !op.eval_ord(self_len.cmp(&other_len)) {
            return Ok(PyComparisonValue::Implemented(false));
        }

        let self_is_subset = matches!(
            op,
            PyComparisonOp::Eq | PyComparisonOp::Lt | PyComparisonOp::Le
        );
        if self_is_subset {
            for item in self_items {
                if !other.sequence_unchecked().contains(&item, vm)? {
                    return Ok(PyComparisonValue::Implemented(false));
                }
            }
        } else {
            let iter = other.get_iter(vm)?;
            for item in iter.iter::<PyObjectRef>(vm)? {
                let item = item?;
                if !self_contains(&item, vm)? {
                    return Ok(PyComparisonValue::Implemented(false));
                }
            }
        }
        Ok(PyComparisonValue::Implemented(true))
    }

    fn ordered_items_contains(
        ordered_dict: &PyOrderedDict,
        needle: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let Some(needle) = needle.downcast_ref::<PyTuple>() else {
            return Ok(false);
        };
        if needle.len() != 2 {
            return Ok(false);
        }
        let Some(value) = ordered_dict.inner._as_dict_inner().get(vm, &*needle[0])? else {
            return Ok(false);
        };
        vm.identical_or_equal(&value, &needle[1])
    }

    fn ordered_view_number_and(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
        let a_is_view = a.downcast_ref::<PyOrderedDictKeys>().is_some()
            || a.downcast_ref::<PyOrderedDictItems>().is_some();
        let (view, other) = if a_is_view { (a, b) } else { (b, a) };
        set_view_number_and(view, other, vm)
    }

    fn ordered_item_view_number_xor(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
        let is_item_view = |obj: &PyObject| {
            obj.downcast_ref::<PyOrderedDictItems>().is_some()
                || obj.downcast_ref::<PyDictItems>().is_some()
        };
        set_item_view_number_xor(a, b, is_item_view(a) && is_item_view(b), vm)
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_keys", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeys {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, AsNumber, Representable))]
    impl PyOrderedDictKeys {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(self.ordered_dict.clone())
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.ordered_dict.as_object().to_owned(), vm)
        }

        #[pymethod]
        fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
            for item in other.iter(vm)? {
                if self
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .contains(vm, &*item?)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }

    impl AsSequence for PyOrderedDictKeys {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictKeys::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                contains: atomic_func!(|seq, target, vm| PyOrderedDictKeys::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .contains(vm, target)),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictKeys {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictKeyIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Comparable for PyOrderedDictKeys {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let entries = zelf.ordered_dict.inner._as_dict_inner();
            set_like_view_cmp(
                entries.len(),
                entries.keys(),
                |needle, vm| entries.contains(vm, needle),
                other,
                op,
                vm,
            )
        }
    }

    impl AsNumber for PyOrderedDictKeys {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                subtract: Some(set_inner_number_subtract),
                and: Some(ordered_view_number_and),
                xor: Some(set_inner_number_xor),
                or: Some(set_inner_number_or),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Representable for PyOrderedDictKeys {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for key in zelf.ordered_dict.inner._as_dict_inner().keys() {
                    let repr: PyStrRef = key.repr(vm)?;
                    str_parts.push(repr.to_string());
                }
                Ok(format!("odict_keys([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_keys(...)".to_owned())
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_values", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictValues {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, AsSequence, Representable))]
    impl PyOrderedDictValues {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseValueIterator {
            PyOrderedDictReverseValueIterator::new(self.ordered_dict.clone())
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.ordered_dict.as_object().to_owned(), vm)
        }
    }

    impl AsSequence for PyOrderedDictValues {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictValues::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictValues {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictValueIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Representable for PyOrderedDictValues {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for value in zelf.ordered_dict.inner._as_dict_inner().values() {
                    let repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(repr.to_string());
                }
                Ok(format!("odict_values([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_values(...)".to_owned())
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_items", traverse)]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyOrderedDictItems {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, AsNumber, Representable))]
    impl PyOrderedDictItems {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseItemIterator {
            PyOrderedDictReverseItemIterator::new(self.ordered_dict.clone())
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.ordered_dict.as_object().to_owned(), vm)
        }

        #[pymethod]
        fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
            for item in other.iter(vm)? {
                let item = item?;
                if ordered_items_contains(&self.ordered_dict, &item, vm)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }

    impl AsSequence for PyOrderedDictItems {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictItems::sequence_downcast(seq)
                    .ordered_dict
                    .inner
                    ._as_dict_inner()
                    .len())),
                contains: atomic_func!(|seq, target, vm| {
                    let zelf = PyOrderedDictItems::sequence_downcast(seq);
                    ordered_items_contains(&zelf.ordered_dict, target, vm)
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictItems {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyOrderedDictItemIterator::new(zelf.ordered_dict.clone()).into_pyobject(vm))
        }
    }

    impl Comparable for PyOrderedDictItems {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            let entries = zelf.ordered_dict.inner._as_dict_inner();
            let self_items = entries
                .items()
                .into_iter()
                .map(|(k, v)| vm.new_tuple((k, v)).into())
                .collect();
            set_like_view_cmp(
                entries.len(),
                self_items,
                |needle, vm| ordered_items_contains(&zelf.ordered_dict, needle, vm),
                other,
                op,
                vm,
            )
        }
    }

    impl AsNumber for PyOrderedDictItems {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                subtract: Some(set_inner_number_subtract),
                and: Some(ordered_view_number_and),
                xor: Some(ordered_item_view_number_xor),
                or: Some(set_inner_number_or),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Representable for PyOrderedDictItems {
        #[inline]
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let mut str_parts =
                    Vec::with_capacity(zelf.ordered_dict.inner._as_dict_inner().len());
                for (key, value) in zelf.ordered_dict.inner._as_dict_inner().items() {
                    let key_repr: PyStrRef = key.repr(vm)?;
                    let value_repr: PyStrRef = value.repr(vm)?;
                    str_parts.push(format!("({key_repr}, {value_repr})"));
                }
                Ok(format!("odict_items([{}])", str_parts.join(", ")))
            } else {
                Ok("odict_items(...)".to_owned())
            }
        }
    }

    // ============================================================================
    // OrderedDict Iterators
    // ============================================================================

    fn reduce_forward_iterator(
        internal: &PyMutex<PositionIterInternal<PyOrderedDictRef>>,
        size: &dict_inner::DictSize,
        mutation_version: usize,
        project: impl Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let internal = internal.lock();
        let mut result = Vec::new();
        if let Active(ordered_dict) = &internal.status {
            let entries = ordered_dict.inner._as_dict_inner();
            let remaining = entries
                .remaining_items_version_checked(internal.position, size, mutation_version, false)
                .map_err(|_| vm.new_runtime_error("OrderedDict mutated during iteration"))?;
            for (key, value) in remaining {
                result.push(project(vm, key, value));
            }
        }
        Ok(vm.new_tuple((builtins_iter(vm), (vm.ctx.new_list(result),))))
    }

    fn reduce_reverse_iterator(
        internal: &PyMutex<PositionIterInternal<PyOrderedDictRef>>,
        size: &dict_inner::DictSize,
        mutation_version: usize,
        project: impl Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let internal = internal.lock();
        let mut result = Vec::new();
        if let Active(ordered_dict) = &internal.status {
            let entries = ordered_dict.inner._as_dict_inner();
            let remaining = entries
                .remaining_items_version_checked(internal.position, size, mutation_version, true)
                .map_err(|_| vm.new_runtime_error("OrderedDict mutated during iteration"))?;
            for (key, value) in remaining {
                result.push(project(vm, key, value));
            }
        }
        Ok(vm.new_tuple((builtins_iter(vm), (vm.ctx.new_list(result),))))
    }

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_keyiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeyIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictKeyIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_forward_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |_vm, key, _value| key,
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictKeyIterator {}
    impl IterNext for PyOrderedDictKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.next_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, key))) => {
                        internal.position = position;
                        (Ok(PyIterReturn::Return(key)), None)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_valueiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictValueIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictValueIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_forward_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |_vm, _key, value| value,
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictValueIterator {}
    impl IterNext for PyOrderedDictValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.next_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, value))) => {
                        internal.position = position;
                        (Ok(PyIterReturn::Return(value)), None)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_itemiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictItemIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, 0)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictItemIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal.lock().length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_forward_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |vm, key, value| vm.new_tuple((key, value)).into(),
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictItemIterator {}
    impl IterNext for PyOrderedDictItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.next_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, (key, value)))) => {
                        internal.position = position;
                        (
                            Ok(PyIterReturn::Return(vm.new_tuple((key, value)).into())),
                            None,
                        )
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    // Reverse iterators

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_reverse_keyiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseKeyIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseKeyIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_reverse_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |_vm, key, _value| key,
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictReverseKeyIterator {}
    impl IterNext for PyOrderedDictReverseKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.prev_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, key))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (Ok(PyIterReturn::Return(key)), released)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_reverse_valueiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseValueIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseValueIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_reverse_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |_vm, _key, value| value,
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictReverseValueIterator {}
    impl IterNext for PyOrderedDictReverseValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.prev_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, value))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (Ok(PyIterReturn::Return(value)), released)
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(
        module = "_collections",
        name = "odict_reverse_itemiterator",
        traverse = "manual"
    )]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseItemIterator {
        size: dict_inner::DictSize,
        mutation_version: usize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let (size, mutation_version) = ordered_dict.inner._as_dict_inner().versioned_size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
                mutation_version,
                internal: PyMutex::new(PositionIterInternal::new(ordered_dict, position)),
            }
        }
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyOrderedDictReverseItemIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.internal
                .lock()
                .rev_length_hint(|_| self.size.entries_size)
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            reduce_reverse_iterator(
                &self.internal,
                &self.size,
                self.mutation_version,
                |vm, key, value| vm.new_tuple((key, value)).into(),
                vm,
            )
        }
    }

    impl SelfIter for PyOrderedDictReverseItemIterator {}
    impl IterNext for PyOrderedDictReverseItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                let entries = ordered_dict.inner._as_dict_inner();
                match entries.prev_entry_version_checked(
                    internal.position,
                    &zelf.size,
                    zelf.mutation_version,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("OrderedDict mutated during iteration")),
                        internal.exhaust(),
                    ),
                    Ok(Some((position, (key, value)))) => {
                        let released = if position == 0 {
                            internal.exhaust()
                        } else {
                            internal.position = position - 1;
                            None
                        };
                        (
                            Ok(PyIterReturn::Return(vm.new_tuple((key, value)).into())),
                            released,
                        )
                    }
                    Ok(None) => (Ok(PyIterReturn::StopIteration(None)), internal.exhaust()),
                }
            })
        }
    }

    macro_rules! impl_ordered_iterator_traverse {
        ($($iterator:ty),* $(,)?) => {
            $(
                // SAFETY: the iterator owns exactly the OrderedDict reference
                // stored in its PositionIterInternal and visits it once.
                unsafe impl Traverse for $iterator {
                    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
                        self.internal.traverse(tracer_fn);
                    }
                }
            )*
        };
    }

    impl_ordered_iterator_traverse!(
        PyOrderedDictKeyIterator,
        PyOrderedDictValueIterator,
        PyOrderedDictItemIterator,
        PyOrderedDictReverseKeyIterator,
        PyOrderedDictReverseValueIterator,
        PyOrderedDictReverseItemIterator,
    );
}
