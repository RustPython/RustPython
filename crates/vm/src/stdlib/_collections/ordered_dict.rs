// OrderedDict implementation
// cspell:ignore odict

#[pymodule(sub)]
pub(crate) mod ordered_dict {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        atomic_func,
        builtins::{
            PyDict, PyGenericAlias, PyMappingProxy, PyTuple, PyTypeRef,
            dict::{
                PyDictItems, set_inner_number_or, set_inner_number_subtract, set_inner_number_xor,
                set_item_view_number_xor, set_view_number_and,
            },
            iter::builtins_iter,
        },
        class::StaticType,
        common::{hash::PyHash, lock::PyMutex},
        convert::ToPyObject,
        dict_inner::DictKey,
        function::{ArgIterable, FuncArgs, OptionalArg, PyArithmeticValue, PyComparisonValue},
        object::{Traverse, TraverseFn},
        protocol::{PyIterReturn, PyMappingMethods, PyNumberMethods, PySequenceMethods},
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
    struct PyOrderedDict {
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
            for (k, v1) in a {
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
        #[pyarg(any, optional)]
        last: OptionalArg<bool>,
    }

    #[derive(FromArgs)]
    struct ODictSetDefaultArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        default: OptionalArg<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct ODictMoveToEndArgs {
        #[pyarg(any)]
        key: PyObjectRef,
        #[pyarg(any, optional)]
        last: OptionalArg<bool>,
    }

    #[derive(FromArgs)]
    struct ODictFromKeysArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        value: OptionalArg<PyObjectRef>,
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
    impl PyOrderedDict {
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
        fn copy(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let exact = Self::is_exact(&zelf);
            let od_copy = if exact {
                Self::default().into_pyobject(vm)
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
                    return Err(Self::mutated_error(vm));
                }
                current = next;
            }
            Ok(od_copy)
        }

        #[pymethod]
        fn update(zelf: PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            if args.args.len() > 1 {
                return Err(vm.new_type_error(format!(
                    "update expected at most 1 argument, got {}",
                    args.args.len()
                )));
            }
            if let Some(arg) = args.args.first() {
                odict_update_arg(zelf.as_object(), arg.clone(), vm)?;
            }
            for (key, value) in args.kwargs {
                zelf.as_object().set_item(&key, value, vm)?;
            }
            Ok(())
        }

        #[pymethod]
        fn pop(zelf: PyRef<Self>, args: ODictPopArgs, vm: &VirtualMachine) -> PyResult {
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
            zelf: PyRef<Self>,
            args: ODictPopItemArgs,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let last = args.last.unwrap_or(true);
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
            zelf: PyRef<Self>,
            args: ODictSetDefaultArgs,
            vm: &VirtualMachine,
        ) -> PyResult {
            let key = args.key;
            let default = args.default.unwrap_or_none(vm);
            if Self::is_exact(&zelf) {
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
            let last = args.last.unwrap_or(true);
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
        fn keys(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            PyODictKeys { od: zelf }.to_pyobject(vm)
        }

        #[pymethod]
        fn values(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            PyODictValues { od: zelf }.to_pyobject(vm)
        }

        #[pymethod]
        fn items(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            PyOrderedDictItems { od: zelf }.to_pyobject(vm)
        }

        #[pymethod]
        fn __reversed__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyObjectRef {
            Self::iter_kind(zelf, ODICT_ITER_KEYS | ODICT_ITER_REVERSED, vm)
        }

        #[pymethod]
        fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let items = zelf.as_object().get_attr("items", vm)?.call((), vm)?;
            let items_iter = items.get_iter(vm)?;
            let state = vm.call_method(zelf.as_object(), "__getstate__", ())?;
            let state = match state.downcast_ref::<PyDict>() {
                Some(d) if d.__len__() == 0 => vm.ctx.none(),
                _ => state,
            };
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
            let value = args.value.unwrap_or_none(vm);
            let inst = cls.as_object().call((), vm)?;
            let iter = args.iterable.get_iter(vm)?;
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
        type Args = FuncArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            if args.args.len() > 1 {
                return Err(vm.new_type_error(format!(
                    "OrderedDict expected at most 1 argument, got {}",
                    args.args.len()
                )));
            }
            Self::update(zelf, args, vm)
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
                    if !a.fast_isinstance(vm.ctx.types.dict_type)
                        || !b.fast_isinstance(vm.ctx.types.dict_type)
                    {
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
        if let Some(dict) = arg.downcast_ref_if_exact::<PyDict>(vm) {
            for (key, value) in dict {
                zelf.set_item(&*key, value, vm)?;
            }
            return Ok(());
        }
        match arg.get_attr("keys", vm) {
            Ok(keys_fn) => {
                let keys = keys_fn.call((), vm)?;
                let iter = keys.get_iter(vm)?;
                for key in iter.iter::<PyObjectRef>(vm)? {
                    let key = key?;
                    let value = arg.get_item(&*key, vm)?;
                    zelf.set_item(&*key, value, vm)?;
                }
                return Ok(());
            }
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {}
            Err(e) => return Err(e),
        }
        let iter = arg.get_iter(vm)?;
        for (index, element) in iter.iter::<PyObjectRef>(vm)?.enumerate() {
            let (key, value) = PyDict::update_sequence_pair(element?, index, vm)?;
            zelf.set_item(&*key, value, vm)?;
        }
        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "odict_iterator", traverse = "manual")]
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
    impl PyODictIter {
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
            let Some(od) = self.od() else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            let mut current = self.current.lock().clone();
            let reversed = self.kind & ODICT_ITER_REVERSED != 0;
            while let Some(key) = current {
                out.push(self.project(key.clone(), vm)?);
                current = od.links.lock().next_after(&key, reversed);
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

    fn odict_view_eq(
        od: &PyRef<PyOrderedDict>,
        kind: u8,
        other: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        if kind == ODICT_ITER_VALUES {
            return Ok(PyArithmeticValue::NotImplemented);
        }
        let set_like = other.fast_isinstance(vm.ctx.types.set_type)
            || other.fast_isinstance(vm.ctx.types.frozenset_type)
            || other.class().is(vm.ctx.types.dict_keys_type)
            || other.class().is(vm.ctx.types.dict_items_type)
            || other.downcast_ref::<PyODictKeys>().is_some()
            || other.downcast_ref::<PyOrderedDictItems>().is_some();
        if !set_like {
            return Ok(PyArithmeticValue::NotImplemented);
        }
        let Ok(other_len) = other.length(vm) else {
            return Ok(PyArithmeticValue::NotImplemented);
        };
        if od.dict.__len__() != other_len {
            return Ok(PyArithmeticValue::Implemented(false));
        }
        let iter_obj = PyODictIter::new(od.clone(), kind).to_pyobject(vm);
        let a_iter = iter_obj.get_iter(vm)?;
        for item in a_iter.iter::<PyObjectRef>(vm)? {
            let item = item?;
            if !other.sequence_unchecked().contains(&item, vm)? {
                return Ok(PyArithmeticValue::Implemented(false));
            }
        }
        Ok(PyArithmeticValue::Implemented(true))
    }

    fn ordered_view_number_and(a: &PyObject, b: &PyObject, vm: &VirtualMachine) -> PyResult {
        let a_is_view = a.downcast_ref::<PyODictKeys>().is_some()
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
    #[pyclass(name = "odict_keys", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyODictKeys {
        od: PyRef<PyOrderedDict>,
    }

    #[pyclass(with(Iterable, Comparable, AsMapping, AsSequence, AsNumber))]
    impl PyODictKeys {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(self.od.clone(), ODICT_ITER_KEYS | ODICT_ITER_REVERSED).to_pyobject(vm)
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.od.as_object().to_owned(), vm)
        }

        #[pymethod]
        fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
            for item in other.iter(vm)? {
                if self.od.dict.inner_getitem_opt(&*item?, vm)?.is_some() {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }

    impl AsMapping for PyODictKeys {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyODictKeys::mapping_downcast(mapping)
                    .od
                    .dict
                    .__len__())),
                ..PyMappingMethods::NOT_IMPLEMENTED
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyODictKeys {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyODictKeys::sequence_downcast(seq)
                    .od
                    .dict
                    .__len__())),
                contains: atomic_func!(|seq, target, vm| {
                    PyODictKeys::sequence_downcast(seq)
                        .od
                        .dict
                        .inner_getitem_opt(target, vm)
                        .map(|v| v.is_some())
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyODictKeys {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od.clone(), ODICT_ITER_KEYS).to_pyobject(vm))
        }
    }

    impl Comparable for PyODictKeys {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| odict_view_eq(&zelf.od, ODICT_ITER_KEYS, other, vm))
        }
    }

    impl AsNumber for PyODictKeys {
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

    #[pyattr]
    #[pyclass(name = "odict_values", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyODictValues {
        od: PyRef<PyOrderedDict>,
    }

    #[pyclass(with(Iterable, Comparable, AsMapping))]
    impl PyODictValues {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(self.od.clone(), ODICT_ITER_VALUES | ODICT_ITER_REVERSED)
                .to_pyobject(vm)
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.od.as_object().to_owned(), vm)
        }
    }

    impl AsMapping for PyODictValues {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyODictValues::mapping_downcast(mapping)
                    .od
                    .dict
                    .__len__())),
                ..PyMappingMethods::NOT_IMPLEMENTED
            };
            &AS_MAPPING
        }
    }

    impl Iterable for PyODictValues {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od.clone(), ODICT_ITER_VALUES).to_pyobject(vm))
        }
    }

    impl Comparable for PyODictValues {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| odict_view_eq(&zelf.od, ODICT_ITER_VALUES, other, vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "odict_items", traverse)]
    #[derive(Debug, PyPayload)]
    pub(crate) struct PyOrderedDictItems {
        od: PyRef<PyOrderedDict>,
    }

    #[pyclass(with(Iterable, Comparable, AsMapping, AsSequence, AsNumber))]
    impl PyOrderedDictItems {
        #[pymethod]
        fn __reversed__(&self, vm: &VirtualMachine) -> PyObjectRef {
            PyODictIter::new(self.od.clone(), ODICT_ITER_ITEMS | ODICT_ITER_REVERSED)
                .to_pyobject(vm)
        }

        #[pygetset]
        fn mapping(&self, vm: &VirtualMachine) -> PyResult<PyMappingProxy> {
            PyMappingProxy::from_object(self.od.as_object().to_owned(), vm)
        }

        #[pymethod]
        fn isdisjoint(&self, other: ArgIterable, vm: &VirtualMachine) -> PyResult<bool> {
            for item in other.iter(vm)? {
                let item = item?;
                let Some(needle) = item.downcast_ref::<PyTuple>() else {
                    continue;
                };
                if needle.len() != 2 {
                    continue;
                }
                let Some(found) = self.od.dict.inner_getitem_opt(&*needle[0], vm)? else {
                    continue;
                };
                if vm.identical_or_equal(&found, &needle[1])? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }

    impl AsMapping for PyOrderedDictItems {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: atomic_func!(|mapping, _vm| Ok(PyOrderedDictItems::mapping_downcast(
                    mapping
                )
                .od
                .dict
                .__len__())),
                ..PyMappingMethods::NOT_IMPLEMENTED
            };
            &AS_MAPPING
        }
    }

    impl AsSequence for PyOrderedDictItems {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| Ok(PyOrderedDictItems::sequence_downcast(seq)
                    .od
                    .dict
                    .__len__())),
                contains: atomic_func!(|seq, target, vm| {
                    let needle: &Py<PyTuple> = match target.downcast_ref() {
                        Some(needle) => needle,
                        None => return Ok(false),
                    };
                    if needle.len() != 2 {
                        return Ok(false);
                    }
                    let zelf = PyOrderedDictItems::sequence_downcast(seq);
                    let key = &needle[0];
                    let Some(found) = zelf.od.dict.inner_getitem_opt(&**key, vm)? else {
                        return Ok(false);
                    };
                    vm.identical_or_equal(&found, &needle[1])
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    impl Iterable for PyOrderedDictItems {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            Ok(PyODictIter::new(zelf.od.clone(), ODICT_ITER_ITEMS).to_pyobject(vm))
        }
    }

    impl Comparable for PyOrderedDictItems {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            op.eq_only(|| odict_view_eq(&zelf.od, ODICT_ITER_ITEMS, other, vm))
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
}
