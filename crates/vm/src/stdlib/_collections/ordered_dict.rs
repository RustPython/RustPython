// OrderedDict implementation
// cspell:ignore odict

#[pymodule(sub)]
pub(super) mod ordered_dict {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{
            IterStatus::Active, PositionIterInternal, PyDict, PyGenericAlias, PyStrRef, PyType,
            PyTypeRef, locked_step,
        },
        common::{ascii, lock::PyMutex},
        convert::ToPyObject,
        dict_inner,
        function::{ArgIterable, KwArgs, OptionalArg, PyComparisonValue},
        iter::PyExactSizeIterator,
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
        module = "_collections",
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
        #[pyarg(positional)]
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
        #[pyarg(positional)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct PopArgs {
        #[pyarg(positional)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct FromKeysArgs {
        #[pyarg(positional)]
        iterable: ArgIterable,
        #[pyarg(any, optional)]
        value: OptionalArg<PyObjectRef>,
    }

    #[pyclass(
        flags(BASETYPE, MAPPING, HAS_DICT),
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
        fn setdefault(&self, args: SetDefaultArgs, vm: &VirtualMachine) -> PyResult {
            self.inner
                ._as_dict_inner()
                .setdefault(vm, &*args.key, || args.default.unwrap_or_none(vm))
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

        #[pymethod]
        fn update(
            &self,
            dict_obj: OptionalArg<PyObjectRef>,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let OptionalArg::Present(dict_obj) = dict_obj {
                self.inner.merge_object(dict_obj, vm)?;
            }
            for (key, value) in kwargs {
                self.inner._as_dict_inner().insert(vm, &key, value)?;
            }
            Ok(())
        }

        #[pymethod]
        fn clear(&self) {
            self.inner._as_dict_inner().clear()
        }

        #[pymethod(name = "__copy__")]
        #[pymethod]
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let new_inner = zelf.inner.copy();
            let new_ref =
                Self { inner: new_inner }.into_ref_with_type(vm, zelf.class().to_owned())?;

            // Copy instance __dict__ if it exists
            if let Some(inst_dict) = zelf.as_object().dict()
                && let Some(new_dict) = new_ref.as_object().dict()
            {
                for (key, value) in inst_dict.items_vec() {
                    new_dict._as_dict_inner().insert(vm, &*key, value)?;
                }
            }

            // Copy slot values using copyreg._slotnames
            if let Ok(copyreg) = vm.import("copyreg", 0)
                && let Ok(slotnames_func) = copyreg.get_attr("_slotnames", vm)
                && let Ok(slot_names) = slotnames_func.call((zelf.class().to_owned(),), vm)
                && let Ok(slot_list) = slot_names.downcast::<crate::builtins::PyList>()
            {
                // Collect slot names to avoid lifetime issues
                let names: Vec<String> = slot_list
                    .borrow_vec()
                    .iter()
                    .filter_map(|name| {
                        name.downcast_ref::<crate::builtins::PyStr>()
                            .map(ToString::to_string)
                    })
                    .filter(|s| s != "__dict__" && s != "__weakref__")
                    .collect();

                for name in names {
                    let interned = vm.ctx.intern_str(name.as_str());
                    if let Ok(value) = zelf.as_object().get_attr(interned, vm) {
                        let _ = new_ref.as_object().set_attr(interned, value, vm);
                    }
                }
            }

            Ok(new_ref)
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
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            // Return (OrderedDict, (list(self.items()),), state)
            // state can be (dict_state, slot_state) tuple or just dict_state
            let items: Vec<PyObjectRef> = zelf
                .inner
                ._as_dict_inner()
                .items()
                .into_iter()
                .map(|(k, v)| vm.new_tuple((k, v)).into())
                .collect();
            let items_list = vm.ctx.new_list(items);

            // Get instance __dict__ if it exists
            let inst_dict = zelf.as_object().dict();
            let dict_state: PyObjectRef = inst_dict
                .filter(|d| d.__len__() > 0)
                .map_or_else(|| vm.ctx.none(), |dict| dict.into());

            // Get slot state using copyreg._slotnames
            let mut slot_state: Option<PyObjectRef> = None;
            if let Ok(copyreg) = vm.import("copyreg", 0)
                && let Ok(slotnames_func) = copyreg.get_attr("_slotnames", vm)
                && let Ok(slot_names) = slotnames_func.call((zelf.class().to_owned(),), vm)
                && let Ok(slot_list) = slot_names.downcast::<crate::builtins::PyList>()
            {
                // Collect slot names to avoid lifetime issues
                let names: Vec<String> = slot_list
                    .borrow_vec()
                    .iter()
                    .filter_map(|name| {
                        name.downcast_ref::<crate::builtins::PyStr>()
                            .map(ToString::to_string)
                    })
                    .filter(|s| s != "__dict__" && s != "__weakref__")
                    .collect();

                let slots_dict = vm.ctx.new_dict();
                for name in names {
                    let interned = vm.ctx.intern_str(name.as_str());
                    if let Ok(value) = zelf.as_object().get_attr(interned, vm) {
                        let _ = slots_dict.set_item(name.as_str(), value, vm);
                    }
                }
                if !slots_dict.is_empty() {
                    slot_state = Some(slots_dict.into());
                }
            }

            // Construct final state
            let state: PyObjectRef = if let Some(slots) = slot_state {
                // Return (dict_state, slot_state) tuple
                vm.new_tuple((dict_state, slots)).into()
            } else {
                dict_state
            };

            vm.new_tuple((zelf.class().to_owned(), vm.new_tuple((items_list,)), state))
                .into()
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
                zelf.inner.merge_object(dict_obj, vm)?;
            }

            // Then add keyword arguments (in order)
            for (key, value) in kwargs {
                zelf.inner._as_dict_inner().insert(vm, &key, value)?;
            }

            Ok(())
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
                let self_items = zelf.inner._as_dict_inner().items();
                let other_items = other_ordered_dict.inner._as_dict_inner().items();

                if self_items.len() != other_items.len() {
                    return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                }

                for ((k1, v1), (k2, v2)) in self_items.iter().zip(other_items.iter()) {
                    // Check keys are equal and in same order
                    if !vm.identical_or_equal(k1, k2)? {
                        return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                    }
                    // Check values are equal
                    if !vm.identical_or_equal(v1, v2)? {
                        return Ok(PyComparisonValue::Implemented(op == PyComparisonOp::Ne));
                    }
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
                        None => Err(vm.new_key_error(needle.to_owned())),
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
                        let a_ordered_dict = a.downcast_ref::<PyOrderedDict>().unwrap();
                        let new_inner = a_ordered_dict.inner.copy();
                        new_inner.merge_object(b.to_pyobject(vm), vm)?;
                        // Preserve the subclass type (use a's type)
                        let result = PyOrderedDict { inner: new_inner }
                            .into_ref_with_type(vm, a.class().to_owned())?;
                        Ok(result.into())
                    } else if b_is_ordered_dict {
                        // This is __ror__: other | OrderedDict
                        // other must be a dict or dict subclass
                        if !a_is_dict {
                            return Ok(vm.ctx.not_implemented());
                        }
                        let b_ordered_dict = b.downcast_ref::<PyOrderedDict>().unwrap();
                        // Create new instance with b's type (preserve subclass)
                        let new_inner = PyDict::default();
                        new_inner.merge_object(a.to_pyobject(vm), vm)?;
                        for (key, value) in b_ordered_dict.inner._as_dict_inner().items() {
                            new_inner._as_dict_inner().insert(vm, &*key, value)?;
                        }
                        let result = PyOrderedDict { inner: new_inner }
                            .into_ref_with_type(vm, b.class().to_owned())?;
                        Ok(result.into())
                    } else {
                        Ok(vm.ctx.not_implemented())
                    }
                }),
                inplace_or: Some(|a, b, vm| {
                    if let Some(a) = a.downcast_ref::<PyOrderedDict>() {
                        a.inner.merge_object(b.to_pyobject(vm), vm)?;
                        Ok(a.to_owned().into())
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

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_keys")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeys {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, Representable))]
    impl PyOrderedDictKeys {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseKeyIterator {
            PyOrderedDictReverseKeyIterator::new(self.ordered_dict.clone())
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
            // Convert both to lists for comparison (like CPython)
            let self_keys: Vec<PyObjectRef> = zelf.ordered_dict.inner._as_dict_inner().keys();
            let other_vec: Result<Vec<PyObjectRef>, _> = other.try_to_value(vm);

            if let Ok(other_keys) = other_vec {
                let other_keys: &Vec<PyObjectRef> = &other_keys;
                self_keys
                    .iter()
                    .richcompare(other_keys.iter(), op, vm)
                    .map(PyComparisonValue::Implemented)
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
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
    #[pyclass(module = "_collections", name = "odict_values")]
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
    #[pyclass(module = "_collections", name = "odict_items")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictItems {
        ordered_dict: PyOrderedDictRef,
    }

    #[pyclass(with(Iterable, Comparable, AsSequence, Representable))]
    impl PyOrderedDictItems {
        #[pymethod]
        fn __reversed__(&self) -> PyOrderedDictReverseItemIterator {
            PyOrderedDictReverseItemIterator::new(self.ordered_dict.clone())
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
            // Convert both to lists for comparison
            let self_items: Vec<PyObjectRef> = zelf
                .ordered_dict
                .inner
                ._as_dict_inner()
                .items()
                .into_iter()
                .map(|(k, v)| vm.new_tuple((k, v)).into())
                .collect();
            let other_vec: Result<Vec<PyObjectRef>, _> = other.try_to_value(vm);

            if let Ok(other_items) = other_vec {
                let other_items: &Vec<PyObjectRef> = &other_items;
                self_items
                    .iter()
                    .richcompare(other_items.iter(), op, vm)
                    .map(PyComparisonValue::Implemented)
            } else {
                Ok(PyComparisonValue::NotImplemented)
            }
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

    #[pyattr]
    #[pyclass(module = "_collections", name = "odict_keyiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictKeyIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictKeyIterator {}
    impl IterNext for PyOrderedDictKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
    #[pyclass(module = "_collections", name = "odict_valueiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictValueIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictValueIterator {}
    impl IterNext for PyOrderedDictValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
    #[pyclass(module = "_collections", name = "odict_itemiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictItemIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictItemIterator {}
    impl IterNext for PyOrderedDictItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().next_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
    #[pyclass(module = "_collections", name = "odict_reverse_keyiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseKeyIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseKeyIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictReverseKeyIterator {}
    impl IterNext for PyOrderedDictReverseKeyIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, _value| key.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
    #[pyclass(module = "_collections", name = "odict_reverse_valueiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseValueIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseValueIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictReverseValueIterator {}
    impl IterNext for PyOrderedDictReverseValueIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |_key, value| value.clone(),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
    #[pyclass(module = "_collections", name = "odict_reverse_itemiterator")]
    #[derive(Debug, PyPayload)]
    struct PyOrderedDictReverseItemIterator {
        size: dict_inner::DictSize,
        internal: PyMutex<PositionIterInternal<PyOrderedDictRef>>,
    }

    impl PyOrderedDictReverseItemIterator {
        fn new(ordered_dict: PyOrderedDictRef) -> Self {
            let size = ordered_dict.inner._as_dict_inner().size();
            let position = size.entries_size.saturating_sub(1);
            Self {
                size,
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
    }

    impl SelfIter for PyOrderedDictReverseItemIterator {}
    impl IterNext for PyOrderedDictReverseItemIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            locked_step(&zelf.internal, |internal| {
                let Active(ordered_dict) = &internal.status else {
                    return (Ok(PyIterReturn::StopIteration(None)), None);
                };
                match ordered_dict.inner._as_dict_inner().prev_entry_checked(
                    internal.position,
                    &zelf.size,
                    |key, value| (key.clone(), value.clone()),
                ) {
                    Err(dict_inner::DictChanged) => (
                        Err(vm.new_runtime_error("dictionary changed size during iteration")),
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
}
