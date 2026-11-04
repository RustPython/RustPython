// OrderedDict implementation
// cspell:ignore odict

#[pymodule(sub, name = "_collections")]
pub(crate) mod ordered_dict {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject,
        VirtualMachine, atomic_func,
        builtins::{
            PyAnyDict, PyDict, PyGenericAlias, PyTypeRef,
            dict::{PyDictItems, PyDictKeys, PyDictValues},
            iter::builtins_iter,
        },
        class::StaticType,
        common::{hash::PyHash, lock::PyMutex},
        convert::ToPyObject,
        dict_inner::DictKey,
        function::{KwArgs, OptionalArg, PyArithmeticValue, PyComparisonValue},
        object::{Traverse, TraverseFn},
        protocol::{PyIter, PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
        recursion::ReprGuard,
        types::{
            AsMapping, AsNumber, AsSequence, Comparable, Constructor, DefaultConstructor,
            Initializer, IterNext, Iterable, PyComparisonOp, Representable, SelfIter,
        },
    };
    use std::collections::HashMap;

    const ODICT_ITER_REVERSED: u8 = 1;
    const ODICT_ITER_KEYS: u8 = 2;
    const ODICT_ITER_VALUES: u8 = 4;
    const ODICT_ITER_ITEMS: u8 = ODICT_ITER_KEYS | ODICT_ITER_VALUES;

    #[derive(Debug)]
    struct ODictNode {
        key: PyObjectRef,
        hash: PyHash,
        prev: Option<usize>,
        next: Option<usize>,
    }

    #[derive(Debug, Default)]
    struct ODictLinks {
        first: Option<usize>,
        last: Option<usize>,
        nodes: Vec<Option<ODictNode>>,
        free: Vec<usize>,
        by_hash: HashMap<PyHash, Vec<usize>>,
        state: usize,
    }

    impl ODictLinks {
        fn alloc_node(&mut self, node: ODictNode) -> usize {
            if let Some(idx) = self.free.pop() {
                self.nodes[idx] = Some(node);
                idx
            } else {
                let idx = self.nodes.len();
                self.nodes.push(Some(node));
                idx
            }
        }

        fn add_tail(&mut self, key: PyObjectRef, hash: PyHash) {
            let prev = self.last;
            let idx = self.alloc_node(ODictNode {
                key,
                hash,
                prev,
                next: None,
            });
            if let Some(prev) = prev {
                if let Some(node) = self.nodes[prev].as_mut() {
                    node.next = Some(idx);
                }
            } else {
                self.first = Some(idx);
            }
            self.last = Some(idx);
            self.by_hash.entry(hash).or_default().push(idx);
            self.state += 1;
        }

        fn unlink(&mut self, idx: usize) -> Option<ODictNode> {
            let node = self.nodes[idx].take()?;
            self.free.push(idx);
            if let Some(prev) = node.prev {
                if let Some(prev_node) = self.nodes[prev].as_mut() {
                    prev_node.next = node.next;
                }
            } else {
                self.first = node.next;
            }
            if let Some(next) = node.next {
                if let Some(next_node) = self.nodes[next].as_mut() {
                    next_node.prev = node.prev;
                }
            } else {
                self.last = node.prev;
            }
            if let Some(bucket) = self.by_hash.get_mut(&node.hash) {
                bucket.retain(|&i| i != idx);
            }
            self.state += 1;
            Some(node)
        }

        fn key_at(&self, idx: usize) -> Option<PyObjectRef> {
            self.nodes[idx].as_ref().map(|n| n.key.clone())
        }

        fn next_after(&self, key: &PyObject, reversed: bool) -> Option<PyObjectRef> {
            let mut idx = if reversed { self.last } else { self.first };
            while let Some(i) = idx {
                let Some(node) = self.nodes[i].as_ref() else {
                    break;
                };
                if node.key.is(key) {
                    let nxt = if reversed { node.prev } else { node.next };
                    return nxt.and_then(|j| self.key_at(j));
                }
                idx = if reversed { node.prev } else { node.next };
            }
            None
        }
    }

    #[pyattr]
    #[pyclass(
        module = "collections",
        name = "OrderedDict",
        base = PyDict,
        unhashable = true,
        traverse = "manual"
    )]
    #[derive(Debug)]
    pub(crate) struct PyOrderedDict {
        dict: PyDict,
        links: PyMutex<ODictLinks>,
    }

    impl Default for PyOrderedDict {
        fn default() -> Self {
            Self {
                dict: PyDict::default(),
                links: PyMutex::new(ODictLinks::default()),
            }
        }
    }

    // SAFETY: Traverse visits each owned Python reference at most once.
    unsafe impl Traverse for PyOrderedDict {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.dict.traverse(tracer_fn);
            if let Some(links) = self.links.try_lock() {
                for node in links.nodes.iter().flatten() {
                    node.key.traverse(tracer_fn);
                }
            }
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            Traverse::clear(&mut self.dict, out);
            let links = self.links.get_mut();
            for node in links.nodes.drain(..).flatten() {
                out.push(node.key);
            }
            links.first = None;
            links.last = None;
            links.free.clear();
            links.by_hash.clear();
            links.state += 1;
        }
    }

    impl PyOrderedDict {
        fn is_exact(zelf: &Py<Self>) -> bool {
            zelf.class().is(Self::static_type())
        }

        fn mutated_error(vm: &VirtualMachine) -> crate::builtins::PyBaseExceptionRef {
            vm.new_runtime_error("OrderedDict mutated during iteration")
        }

        fn find_node(
            &self,
            key: &PyObject,
            hash: PyHash,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let candidates: Vec<(usize, PyObjectRef)>;
            {
                let links = self.links.lock();
                let Some(idxs) = links.by_hash.get(&hash) else {
                    return Ok(None);
                };
                for &i in idxs {
                    if let Some(node) = links.nodes[i].as_ref()
                        && node.key.is(key)
                    {
                        return Ok(Some(i));
                    }
                }
                candidates = idxs
                    .iter()
                    .filter_map(|&i| links.nodes[i].as_ref().map(|n| (i, n.key.clone())))
                    .collect();
            }
            for (i, k) in candidates {
                if vm.bool_eq(&k, key)? {
                    return Ok(Some(i));
                }
            }
            Ok(None)
        }

        fn setitem_impl(
            &self,
            key: PyObjectRef,
            value: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let hash = key.key_hash(vm)?;
            self.dict.inner_setitem(&*key, value, vm)?;
            if self.find_node(&key, hash, vm)?.is_none() {
                self.links.lock().add_tail(key, hash);
            }
            Ok(())
        }

        fn delitem_impl(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let hash = key.key_hash(vm)?;
            let Some(idx) = self.find_node(&key, hash, vm)? else {
                return Err(vm.new_key_error(key));
            };
            self.links.lock().unlink(idx);
            self.dict.inner_delitem(&*key, vm)
        }

        fn dicts_equal(a: &PyDict, b: &PyDict, vm: &VirtualMachine) -> PyResult<bool> {
            if a.__len__() != b.__len__() {
                return Ok(false);
            }
            let mut pos = 0;
            while let Some((next, k, v1)) = a.next_entry(pos) {
                pos = next;
                match b.inner_getitem_opt(&*k, vm)? {
                    Some(v2) if v1.is(&v2) || vm.bool_eq(&v1, &v2)? => {}
                    _ => return Ok(false),
                }
            }
            Ok(true)
        }

        fn keys_equal(&self, other: &Self, vm: &VirtualMachine) -> PyResult<bool> {
            if core::ptr::eq(self, other) {
                return Ok(true);
            }
            let (state_a, state_b, mut key_a, mut key_b) = {
                let la = self.links.lock();
                let lb = other.links.lock();
                (
                    la.state,
                    lb.state,
                    la.first.and_then(|i| la.key_at(i)),
                    lb.first.and_then(|i| lb.key_at(i)),
                )
            };
            loop {
                match (key_a, key_b) {
                    (None, None) => return Ok(true),
                    (None, Some(_)) | (Some(_), None) => return Ok(false),
                    (Some(a), Some(b)) => {
                        let eq = vm.bool_eq(&a, &b)?;
                        let (next_a, next_b, mutated) = {
                            let la = self.links.lock();
                            let lb = other.links.lock();
                            if la.state != state_a || lb.state != state_b {
                                (None, None, true)
                            } else if !eq {
                                return Ok(false);
                            } else {
                                (la.next_after(&a, false), lb.next_after(&b, false), false)
                            }
                        };
                        if mutated {
                            return Err(Self::mutated_error(vm));
                        }
                        key_a = next_a;
                        key_b = next_b;
                    }
                }
            }
        }

        fn first_or_last_key(&self, last: bool) -> Option<PyObjectRef> {
            let links = self.links.lock();
            let idx = if last { links.last } else { links.first }?;
            links.key_at(idx)
        }

        fn iter_kind(zelf: PyRef<Self>, kind: u8, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(zelf, kind).to_pyobject(vm)
        }
    }

    #[derive(FromArgs)]
    struct ODictPopArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ODictPopItemArgs {
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct ODictSetDefaultArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ODictMoveToEndArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, default = true)]
        last: bool,
    }

    #[derive(FromArgs)]
    struct ODictFromKeysArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        value: Option<PyObjectRef>,
    }

    #[pyclass(
        with(
            AsMapping,
            AsNumber,
            AsSequence,
            Comparable,
            Constructor,
            Initializer,
            Iterable,
            Representable
        ),
        flags(BASETYPE, MAPPING, HAS_DICT, HAS_WEAKREF)
    )]
    impl Py<PyOrderedDict> {
        #[pymethod]
        fn clear(&self) {
            self.dict.clear();
            let mut links = self.links.lock();
            *links = ODictLinks {
                state: links.state.wrapping_add(1),
                ..ODictLinks::default()
            };
        }

        #[pymethod]
        fn copy(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyResult {
            let exact = PyOrderedDict::is_exact(&zelf);
            let od_copy = if exact {
                PyOrderedDict::default().into_pyobject(vm)
            } else {
                let cls = zelf.class().to_owned();
                cls.as_object().call((), vm)?
            };
            let state = zelf.links.lock().state;
            let mut current = zelf.first_or_last_key(false);
            while let Some(key) = current {
                let value = if exact {
                    zelf.dict
                        .inner_getitem_opt(&*key, vm)?
                        .ok_or_else(|| vm.new_key_error(key.clone()))?
                } else {
                    zelf.as_object().get_item(&*key, vm)?
                };
                od_copy.set_item(&*key, value, vm)?;
                let (next, mutated) = {
                    let links = zelf.links.lock();
                    if links.state != state {
                        (None, true)
                    } else {
                        (links.next_after(&key, false), false)
                    }
                };
                if mutated {
                    return Err(PyOrderedDict::mutated_error(vm));
                }
                current = next;
            }
            Ok(od_copy)
        }

        #[pymethod]
        fn update(
            zelf: PyRef<PyOrderedDict>,
            other: OptionalArg<PyObjectRef>,
            kwargs: KwArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if let OptionalArg::Present(other) = other {
                odict_update_arg(zelf.as_object(), other, vm)?;
            }
            for (key, value) in kwargs {
                zelf.as_object().set_item(&key, value, vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn pop(zelf: PyRef<PyOrderedDict>, args: ODictPopArgs, vm: &VirtualMachine) -> PyResult {
            let key = args.key;
            let hash = key.key_hash(vm)?;
            match zelf.find_node(&key, hash, vm)? {
                Some(idx) => {
                    zelf.links.lock().unlink(idx);
                    let value = zelf
                        .dict
                        .inner_getitem_opt(&*key, vm)?
                        .ok_or_else(|| vm.new_key_error(key.clone()))?;
                    zelf.dict.inner_delitem(&*key, vm)?;
                    Ok(value)
                }
                None => args.default.ok_or_else(|| vm.new_key_error(key)),
            }
        }

        #[pymethod]
        fn popitem(
            zelf: PyRef<PyOrderedDict>,
            args: ODictPopItemArgs,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let last = args.last;
            let Some(key) = zelf.first_or_last_key(last) else {
                return Err(vm.new_key_error(vm.ctx.new_str("dictionary is empty").into()));
            };
            let hash = key.key_hash(vm)?;
            if let Some(idx) = zelf.find_node(&key, hash, vm)? {
                zelf.links.lock().unlink(idx);
            }
            let value = zelf
                .dict
                .inner_getitem_opt(&*key, vm)?
                .ok_or_else(|| vm.new_key_error(key.clone()))?;
            zelf.dict.inner_delitem(&*key, vm)?;
            Ok((key, value))
        }

        #[pymethod]
        fn setdefault(
            zelf: PyRef<PyOrderedDict>,
            args: ODictSetDefaultArgs,
            vm: &VirtualMachine,
        ) -> PyResult {
            let key = args.key;
            let default = args.default.unwrap_or_else(|| vm.ctx.none());
            if PyOrderedDict::is_exact(&zelf) {
                if let Some(value) = zelf.dict.inner_getitem_opt(&*key, vm)? {
                    return Ok(value);
                }
                zelf.setitem_impl(key, default.clone(), vm)?;
                Ok(default)
            } else if zelf.as_object().sequence_unchecked().contains(&key, vm)? {
                zelf.as_object().get_item(&*key, vm)
            } else {
                zelf.as_object().set_item(&*key, default.clone(), vm)?;
                Ok(default)
            }
        }

        #[pymethod]
        fn move_to_end(&self, args: ODictMoveToEndArgs, vm: &VirtualMachine) -> PyResult<()> {
            let key = args.key;
            let last = args.last;
            let hash = key.key_hash(vm)?;
            let Some(idx) = self.find_node(&key, hash, vm)? else {
                return Err(vm.new_key_error(key));
            };
            let mut links = self.links.lock();
            let already = if last {
                links.last == Some(idx)
            } else {
                links.first == Some(idx)
            };
            if already {
                return Ok(());
            }
            let Some(node) = links.unlink(idx) else {
                return Err(vm.new_key_error(key));
            };
            if last {
                links.add_tail(node.key, node.hash);
            } else {
                let next = links.first;
                let new_idx = links.alloc_node(ODictNode {
                    key: node.key,
                    hash: node.hash,
                    prev: None,
                    next,
                });
                if let Some(next) = next {
                    if let Some(next_node) = links.nodes[next].as_mut() {
                        next_node.prev = Some(new_idx);
                    }
                } else {
                    links.last = Some(new_idx);
                }
                links.first = Some(new_idx);
                links.by_hash.entry(node.hash).or_default().push(new_idx);
                links.state += 1;
            }
            Ok(())
        }

        #[pymethod]
        fn keys(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyObjectRef {
            PyODictKeys::new(zelf).to_pyobject(vm)
        }

        #[pymethod]
        fn values(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyObjectRef {
            PyODictValues::new(zelf).to_pyobject(vm)
        }

        #[pymethod]
        fn items(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyObjectRef {
            PyOrderedDictItems::new(zelf).to_pyobject(vm)
        }

        #[pymethod]
        fn __reversed__(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyObjectRef {
            PyOrderedDict::iter_kind(zelf, ODICT_ITER_KEYS | ODICT_ITER_REVERSED, vm)
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<PyOrderedDict>, vm: &VirtualMachine) -> PyResult {
            // State hooks run before items(), and their result is preserved even
            // when a subclass deliberately returns an empty dictionary.
            let state = vm.call_method(zelf.as_object(), "__getstate__", ())?;
            let items = zelf.as_object().get_attr("items", vm)?.call((), vm)?;
            let items_iter = PyIter::try_from_object(vm, items)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    zelf.class().to_owned().into(),
                    vm.ctx.empty_tuple.clone().into(),
                    state,
                    vm.ctx.none(),
                    items_iter.into(),
                ])
                .into())
        }

        #[pymethod]
        fn __sizeof__(&self) -> usize {
            self.dict.__sizeof__()
                + core::mem::size_of::<ODictLinks>()
                + self.links.lock().nodes.capacity() * core::mem::size_of::<Option<ODictNode>>()
        }

        #[pyclassmethod]
        fn fromkeys(cls: PyTypeRef, args: ODictFromKeysArgs, vm: &VirtualMachine) -> PyResult {
            let value = args.value.unwrap_or_else(|| vm.ctx.none());
            let inst = cls.as_object().call((), vm)?;
            let iter = PyIter::try_from_object(vm, args.iterable)?;
            for key in iter.iter::<PyObjectRef>(vm)? {
                inst.set_item(&*key?, value.clone(), vm)?;
            }
            Ok(inst)
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

        fn init(zelf: &Py<Self>, (other, kwargs): Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            Py::<Self>::update(zelf.to_owned(), other, kwargs, vm)
        }
    }

    impl Representable for PyOrderedDict {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            if zelf.dict.__len__() == 0 {
                return Ok(format!("{}()", zelf.class().name()));
            }
            let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) else {
                return Ok("...".to_owned());
            };
            let tmp = PyDict::default();
            let mut current = zelf.first_or_last_key(false);
            while let Some(key) = current {
                let value = zelf
                    .dict
                    .inner_getitem_opt(&*key, vm)?
                    .ok_or_else(|| vm.new_key_error(key.clone()))?;
                tmp.inner_setitem(&*key, value, vm)?;
                current = zelf.links.lock().next_after(&key, false);
            }
            let dcopy = tmp.into_ref(&vm.ctx);
            let dict_repr = dcopy.as_object().repr(vm)?;
            Ok(format!("{}({dict_repr})", zelf.class().name()))
        }
    }

    impl AsMapping for PyOrderedDict {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyOrderedDict::mapping_downcast(mapping)
                    .dict
                    .__len__())),
                subscript: atomic_func!(|mapping, needle, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    if let Some(value) = zelf.dict.inner_getitem_opt(needle, vm)? {
                        return Ok(value);
                    }
                    if let Some(missing) =
                        vm.get_method(zelf.to_owned().into(), identifier!(vm, __missing__))
                    {
                        return missing?.call((needle.to_owned(),), vm);
                    }
                    Err(vm.new_key_error(needle.to_owned()))
                }),
                ass_subscript: atomic_func!(|mapping, needle, value, vm| {
                    let zelf = PyOrderedDict::mapping_downcast(mapping);
                    if let Some(value) = value {
                        zelf.setitem_impl(needle.to_owned(), value, vm)
                    } else {
                        zelf.delitem_impl(needle.to_owned(), vm)
                    }
                }),
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyOrderedDict {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                contains: atomic_func!(|seq, target, vm| {
                    PyOrderedDict::sequence_downcast(seq)
                        .dict
                        .inner_getitem_opt(target, vm)
                        .map(|v| v.is_some())
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl AsNumber for PyOrderedDict {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                or: Some(|a, b, vm| {
                    if PyAnyDict::from_object(a).is_none() || PyAnyDict::from_object(b).is_none() {
                        return Ok(vm.ctx.not_implemented());
                    }
                    let cls = if let Some(od) = a.downcast_ref::<PyOrderedDict>() {
                        od.class().to_owned()
                    } else if let Some(od) = b.downcast_ref::<PyOrderedDict>() {
                        od.class().to_owned()
                    } else {
                        return Ok(vm.ctx.not_implemented());
                    };
                    let new = cls.as_object().call((a.to_owned(),), vm)?;
                    odict_update_arg(&new, b.to_owned(), vm)?;
                    Ok(new)
                }),
                inplace_or: Some(|a, b, vm| {
                    let Some(od) = a.downcast_ref::<PyOrderedDict>() else {
                        return Ok(vm.ctx.not_implemented());
                    };
                    odict_update_arg(od.as_object(), b.to_owned(), vm)?;
                    Ok(od.as_object().to_owned())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for PyOrderedDict {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            if !matches!(op, PyComparisonOp::Eq | PyComparisonOp::Ne) {
                return Ok(PyArithmeticValue::NotImplemented);
            }
            if !other.fast_isinstance(vm.ctx.types.dict_type) {
                return Ok(PyArithmeticValue::NotImplemented);
            }
            let dict_eq = if let Some(other_od) = other.downcast_ref::<Self>() {
                let eq = Self::dicts_equal(&zelf.dict, &other_od.dict, vm)?;
                if (op == PyComparisonOp::Eq && !eq) || (op == PyComparisonOp::Ne && !eq) {
                    return Ok(PyArithmeticValue::Implemented(op == PyComparisonOp::Ne));
                }
                let keys_eq = zelf.keys_equal(other_od, vm)?;
                return Ok(PyArithmeticValue::Implemented(
                    if op == PyComparisonOp::Eq {
                        keys_eq
                    } else {
                        !keys_eq
                    },
                ));
            } else if let Some(other_d) = other.downcast_ref::<PyDict>() {
                Self::dicts_equal(&zelf.dict, other_d, vm)?
            } else {
                return Ok(PyArithmeticValue::NotImplemented);
            };
            Ok(PyArithmeticValue::Implemented(
                if op == PyComparisonOp::Eq {
                    dict_eq
                } else {
                    !dict_eq
                },
            ))
        }
    }

    impl Iterable for PyOrderedDict {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(Self::iter_kind(zelf, ODICT_ITER_KEYS, vm))
        }
    }

    fn odict_update_arg(zelf: &PyObject, arg: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if (arg.class().is(vm.ctx.types.dict_type) || arg.class().is(vm.ctx.types.frozendict_type))
            && let Some(dict) = PyAnyDict::from_object(&arg)
        {
            // A destination's __setitem__ can mutate the source. CPython takes
            // an items snapshot before calling any of those overrides.
            let items = dict.items_vec();
            let result = items
                .iter()
                .try_for_each(|(key, value)| zelf.set_item(&**key, value.clone(), vm));
            // The snapshot is a list of tuples in CPython: release entries
            // backwards, and each value before its key, after all callbacks.
            for (key, value) in items.into_iter().rev() {
                drop(value);
                drop(key);
            }
            return result;
        }
        if let Some(keys_fn) = vm.get_attribute_opt(&arg, "keys")? {
            let keys = keys_fn.call((), vm)?;
            let iter = PyIter::try_from_object(vm, keys)?;
            for key in iter.iter::<PyObjectRef>(vm)? {
                let key = key?;
                let value = arg.get_item(&*key, vm)?;
                zelf.set_item(&*key, value, vm)?;
            }
            return Ok(());
        }
        let pairs = match vm.get_attribute_opt(&arg, "items")? {
            Some(items_fn) => items_fn.call((), vm)?,
            None => arg,
        };
        odict_add_pairs(zelf, pairs, vm)
    }

    fn odict_add_pairs(zelf: &PyObject, pairs: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // An iterator need not retain its iterable. Keep the container alive
        // until all pair callbacks and the outer iterator have finished.
        let result = (|| {
            let iter = PyIter::try_from_object(vm, pairs.clone())?;
            for pair in iter.iter::<PyObjectRef>(vm)? {
                let pair = pair?;
                let mut pair_iter = None;
                let mut key = None;
                let mut value = None;
                let result = (|| {
                    let iterator = pair_iter.insert(PyIter::try_from_object(vm, pair.clone())?);
                    // Unlike dict.update(), OrderedDict unpacks at most three
                    // values and never consults a pair's length hint.
                    let key = key.insert(match iterator.next(vm)? {
                        PyIterReturn::Return(key) => key,
                        PyIterReturn::StopIteration(_) => {
                            return Err(vm.new_value_error("need more than 0 values to unpack"));
                        }
                    });
                    let value = value.insert(match iterator.next(vm)? {
                        PyIterReturn::Return(value) => value,
                        PyIterReturn::StopIteration(_) => {
                            return Err(vm.new_value_error("need more than 1 value to unpack"));
                        }
                    });
                    if let PyIterReturn::Return(unexpected) = iterator.next(vm)? {
                        drop(unexpected);
                        return Err(vm.new_value_error("too many values to unpack (expected 2)"));
                    }
                    zelf.set_item(&**key, value.clone(), vm)
                })();
                // Finalizers may call back into the destination. Match the pair,
                // iterator, key, value release order on both success and failure.
                drop(pair);
                drop(pair_iter);
                drop(key);
                drop(value);
                result?;
            }
            Ok(())
        })();
        drop(pairs);
        result
    }

    #[pyattr]
    #[pyclass(name = "odict_iterator", module = "builtins", traverse = "manual")]
    #[derive(Debug, PyPayload)]
    struct PyODictIter {
        od: Option<PyRef<PyOrderedDict>>,
        kind: u8,
        size: usize,
        state: usize,
        current: PyMutex<Option<PyObjectRef>>,
    }

    impl PyODictIter {
        fn new(od: PyRef<PyOrderedDict>, kind: u8) -> Self {
            let reversed = kind & ODICT_ITER_REVERSED != 0;
            let (state, current) = {
                let links = od.links.lock();
                let idx = if reversed { links.last } else { links.first };
                (links.state, idx.and_then(|i| links.key_at(i)))
            };
            Self {
                size: od.dict.__len__(),
                state,
                current: PyMutex::new(current),
                od: Some(od),
                kind,
            }
        }

        fn od(&self) -> Option<&Py<PyOrderedDict>> {
            self.od.as_deref()
        }

        fn project(&self, key: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let Some(od) = self.od() else {
                return Err(vm.new_runtime_error("OrderedDict iterator is cleared"));
            };
            let want_key = self.kind & ODICT_ITER_KEYS != 0;
            let want_value = self.kind & ODICT_ITER_VALUES != 0;
            if want_key && want_value {
                let value = od
                    .dict
                    .inner_getitem_opt(&*key, vm)?
                    .ok_or_else(|| vm.new_key_error(key.clone()))?;
                Ok(vm.ctx.new_tuple(vec![key, value]).into())
            } else if want_value {
                od.dict
                    .inner_getitem_opt(&*key, vm)?
                    .ok_or_else(|| vm.new_key_error(key))
            } else {
                Ok(key)
            }
        }
    }

    // SAFETY: visits each owned reference at most once.
    unsafe impl Traverse for PyODictIter {
        fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
            self.od.traverse(tracer_fn);
            if let Some(cur) = self.current.try_lock()
                && let Some(key) = cur.as_ref()
            {
                key.traverse(tracer_fn);
            }
        }

        fn clear(&mut self, out: &mut Vec<PyObjectRef>) {
            if let Some(key) = self.current.get_mut().take() {
                out.push(key);
            }
            if let Some(od) = self.od.take() {
                out.push(od.into());
            }
        }
    }

    #[pyclass(with(IterNext, Iterable), flags(DISALLOW_INSTANTIATION))]
    impl Py<PyODictIter> {
        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
            let remaining = self.collect_remaining(vm)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    builtins_iter(vm)?,
                    vm.ctx
                        .new_tuple(vec![vm.ctx.new_list(remaining).into()])
                        .into(),
                ])
                .into())
        }
    }

    impl PyODictIter {
        fn collect_remaining(&self, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
            // Reduction consumes a copy through the normal iterator path so
            // mutation checks and value lookup callbacks match next(). The
            // original iterator must retain its position, including on error.
            let copy = Self {
                od: self.od.clone(),
                kind: self.kind,
                size: self.size,
                state: self.state,
                current: PyMutex::new(self.current.lock().clone()),
            }
            .into_ref(&vm.ctx);
            let mut out = Vec::new();
            while let PyIterReturn::Return(value) = Self::next(&copy, vm)? {
                out.push(value);
            }
            Ok(out)
        }
    }

    impl SelfIter for PyODictIter {}

    impl IterNext for PyODictIter {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let Some(od) = zelf.od() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let mut current_guard = zelf.current.lock();
            let Some(key) = current_guard.clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let links = od.links.lock();
            if links.state != zelf.state {
                return Err(PyOrderedDict::mutated_error(vm));
            }
            if od.dict.__len__() != zelf.size {
                return Err(vm.new_runtime_error("OrderedDict changed size during iteration"));
            }
            let reversed = zelf.kind & ODICT_ITER_REVERSED != 0;
            let next = links.next_after(&key, reversed);
            drop(links);
            *current_guard = next;
            drop(current_guard);
            Ok(PyIterReturn::Return(zelf.project(key, vm)?))
        }
    }

    #[pyattr]
    #[pyclass(name = "odict_keys", module = false, base = PyDictKeys, traverse)]
    #[derive(Debug)]
    struct PyODictKeys {
        view: PyDictKeys,
    }

    impl PyODictKeys {
        fn new(od: PyRef<PyOrderedDict>) -> Self {
            let dict = PyAnyDict::from_object(od.as_object())
                .expect("OrderedDict has a dict payload")
                .to_owned();
            Self {
                view: PyDictKeys::new(dict),
            }
        }

        fn od(&self) -> &Py<PyOrderedDict> {
            self.view
                .dict
                .as_object()
                .downcast_ref::<PyOrderedDict>()
                .expect("ordered dict views own an OrderedDict")
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Iterable))]
    impl Py<PyODictKeys> {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(self.od().to_owned(), ODICT_ITER_KEYS | ODICT_ITER_REVERSED)
                .to_pyobject(vm)
        }
    }

    impl Iterable for PyODictKeys {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od().to_owned(), ODICT_ITER_KEYS).to_pyobject(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "odict_values", module = false, base = PyDictValues, traverse)]
    #[derive(Debug)]
    struct PyODictValues {
        view: PyDictValues,
    }

    impl PyODictValues {
        fn new(od: PyRef<PyOrderedDict>) -> Self {
            let dict = PyAnyDict::from_object(od.as_object())
                .expect("OrderedDict has a dict payload")
                .to_owned();
            Self {
                view: PyDictValues::new(dict),
            }
        }

        fn od(&self) -> &Py<PyOrderedDict> {
            self.view
                .dict
                .as_object()
                .downcast_ref::<PyOrderedDict>()
                .expect("ordered dict views own an OrderedDict")
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Iterable))]
    impl Py<PyODictValues> {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(
                self.od().to_owned(),
                ODICT_ITER_VALUES | ODICT_ITER_REVERSED,
            )
            .to_pyobject(vm)
        }
    }

    impl Iterable for PyODictValues {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od().to_owned(), ODICT_ITER_VALUES).to_pyobject(vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "odict_items", module = false, base = PyDictItems, traverse)]
    #[derive(Debug)]
    struct PyOrderedDictItems {
        view: PyDictItems,
    }

    impl PyOrderedDictItems {
        fn new(od: PyRef<PyOrderedDict>) -> Self {
            let dict = PyAnyDict::from_object(od.as_object())
                .expect("OrderedDict has a dict payload")
                .to_owned();
            Self {
                view: PyDictItems::new(dict),
            }
        }

        fn od(&self) -> &Py<PyOrderedDict> {
            self.view
                .dict
                .as_object()
                .downcast_ref::<PyOrderedDict>()
                .expect("ordered dict views own an OrderedDict")
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION), with(Iterable))]
    impl Py<PyOrderedDictItems> {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(self.od().to_owned(), ODICT_ITER_ITEMS | ODICT_ITER_REVERSED)
                .to_pyobject(vm)
        }
    }

    impl Iterable for PyOrderedDictItems {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od().to_owned(), ODICT_ITER_ITEMS).to_pyobject(vm))
        }
    }
}
