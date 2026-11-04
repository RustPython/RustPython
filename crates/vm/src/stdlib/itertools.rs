pub(crate) use decl::module_def;

#[pymodule(name = "itertools")]
mod decl {
    use crate::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, PyWeakRef,
        TryFromBorrowedObject, TryFromObject, VirtualMachine,
        builtins::{PyGenericAlias, PyInt, PyList, PyTuple, PyTupleRef, PyType, PyTypeRef},
        class::PyClassDef,
        common::lock::{PyMutex, PyRwLock},
        convert::ToPyObject,
        function::{FuncArgs, NameIterables, OptionalArg, PosArgs},
        protocol::{PyIter, PyIterReturn, PyNumber},
        raise_if_stop,
        types::{Constructor, IterNext, Iterable, Representable, SelfIter},
    };
    use core::sync::atomic::{AtomicBool, Ordering};
    use crossbeam_utils::atomic::AtomicCell;
    use rustpython_common::wtf8::Wtf8Buf;

    use alloc::fmt;
    use num_traits::ToPrimitive;

    #[pyattr]
    #[pyclass(name = "chain", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsChain {
        source: PyRwLock<Option<PyIter>>,
        active: PyRwLock<Option<PyIter>>,
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE, HAS_DICT))]
    impl PyItertoolsChain {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args =
                crate::types::drop_kwargs_if_init_overridden(&cls, Self::class(&vm.ctx), args);
            if !args.kwargs.is_empty() {
                return Err(
                    vm.new_type_error(format!("{}() takes no keyword arguments", Self::NAME))
                );
            }
            let args_list = PyList::from(args.args);
            Self {
                source: PyRwLock::new(Some(PyIter::try_from_object(
                    vm,
                    args_list.to_pyobject(vm),
                )?)),
                active: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

        #[pyclassmethod]
        fn from_iterable(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            Self {
                source: PyRwLock::new(Some(PyIter::try_from_object(vm, iterable)?)),
                active: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            object: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyGenericAlias> {
            PyGenericAlias::from_args(cls, object, vm)
        }
    }

    impl Constructor for PyItertoolsChain {
        type Args = PosArgs<PyObjectRef, NameIterables>;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl SelfIter for PyItertoolsChain {}

    impl IterNext for PyItertoolsChain {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let Some(source) = zelf.source.read().clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let next = loop {
                let maybe_active = zelf.active.read().clone();
                if let Some(active) = maybe_active {
                    match active.next(vm) {
                        Ok(PyIterReturn::Return(ok)) => {
                            break Ok(PyIterReturn::Return(ok));
                        }
                        Ok(PyIterReturn::StopIteration(_)) => {
                            let _previous = zelf.active.write().take();
                        }
                        Err(err) => {
                            return Err(err);
                        }
                    }
                } else {
                    match source.next(vm) {
                        Ok(PyIterReturn::Return(ok)) => match PyIter::try_from_object(vm, ok) {
                            Ok(iter) => {
                                let _previous = zelf.active.write().replace(iter);
                            }
                            Err(err) => {
                                break Err(err);
                            }
                        },
                        Ok(PyIterReturn::StopIteration(_)) => {
                            break Ok(PyIterReturn::StopIteration(None));
                        }
                        Err(err) => {
                            break Err(err);
                        }
                    }
                }
            };

            if matches!(next, Err(_) | Ok(PyIterReturn::StopIteration(_))) {
                *zelf.source.write() = None;
            };

            next
        }
    }

    #[pyattr]
    #[pyclass(name = "compress", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCompress {
        data: PyIter,
        selectors: PyIter,
    }

    #[derive(FromArgs)]
    struct IterablePosArg {
        #[pyarg(positional)]
        iterable: PyIter,
    }

    #[derive(FromArgs)]
    struct CompressNewArgs {
        #[pyarg(any)]
        data: PyObjectRef,
        #[pyarg(any)]
        selectors: PyObjectRef,
    }

    impl Constructor for PyItertoolsCompress {
        type Args = CompressNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { data, selectors }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let data_iter = data.get_iter(vm)?;
            let selectors_iter = selectors.get_iter(vm)?;
            Ok(Self {
                data: data_iter,
                selectors: selectors_iter,
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsCompress {}

    impl SelfIter for PyItertoolsCompress {}

    impl IterNext for PyItertoolsCompress {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            loop {
                // Match zip(data, selectors): consume data before the selector,
                // and stop as soon as either input is exhausted.
                let data_obj = raise_if_stop!(zelf.data.next(vm)?);
                let sel_obj = raise_if_stop!(zelf.selectors.next(vm)?);
                let selected = sel_obj.try_to_bool(vm)?;
                if selected {
                    return Ok(PyIterReturn::Return(data_obj));
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "count", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCount {
        cur: PyRwLock<PyObjectRef>,
        step: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct CountNewArgs {
        #[pyarg(any, default = 0)]
        start: PyObjectRef,

        #[pyarg(any, default = 1)]
        step: PyObjectRef,
    }

    impl Constructor for PyItertoolsCount {
        type Args = CountNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { start, step }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            if !PyNumber::check(&start) || !PyNumber::check(&step) {
                return Err(vm.new_type_error("a number is required"));
            }

            Ok(Self {
                cur: PyRwLock::new(start),
                step,
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor, Representable), flags(BASETYPE))]
    impl PyItertoolsCount {}

    impl SelfIter for PyItertoolsCount {}

    impl IterNext for PyItertoolsCount {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut cur = zelf.cur.write();
            let step = zelf.step.clone();
            let result = cur.clone();
            *cur = vm._iadd(&cur, step.as_object())?;
            Ok(PyIterReturn::Return(result.to_pyobject(vm)))
        }
    }

    impl Representable for PyItertoolsCount {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let cur_repr = zelf.cur.read().clone().repr(vm)?;
            let step = &zelf.step;
            let mut result = Wtf8Buf::from("count(");
            result.push_wtf8(cur_repr.as_wtf8());
            let step_is_int_one = step.fast_isinstance(vm.ctx.types.int_type)
                && vm.bool_eq(step, vm.ctx.new_int(1).as_object())?;
            if !step_is_int_one {
                result.push_str(", ");
                result.push_wtf8(step.repr(vm)?.as_wtf8());
            }
            result.push_char(')');
            Ok(result)
        }
    }

    #[pyattr]
    #[pyclass(name = "cycle", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCycle {
        iter: PyMutex<Option<PyIter>>,
        saved: PyRwLock<Vec<PyObjectRef>>,
        #[pytraverse(skip)]
        index: AtomicCell<usize>,
    }

    impl Constructor for PyItertoolsCycle {
        type Args = IterablePosArg;
        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            let iter = args.iterable;
            Ok(Self {
                iter: PyMutex::new(Some(iter)),
                saved: PyRwLock::new(Vec::new()),
                index: AtomicCell::new(0),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsCycle {}

    impl SelfIter for PyItertoolsCycle {}

    impl IterNext for PyItertoolsCycle {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let iterator = zelf.iter.lock().clone();
            let restarting = if let Some(iterator) = iterator {
                let next = iterator.next(vm);
                drop(iterator);
                match next? {
                    PyIterReturn::Return(item) => {
                        zelf.saved.write().push(item.clone());
                        return Ok(PyIterReturn::Return(item));
                    }
                    PyIterReturn::StopIteration(value) => {
                        drop(value);
                        let previous = {
                            let mut iterator = zelf.iter.lock();
                            zelf.index.store(0);
                            iterator.take()
                        };
                        // Publish replay mode before releasing the source. Its
                        // finalizer may advance this cycle, but this call still
                        // starts replay at its own index zero.
                        drop(previous);
                        true
                    }
                }
            } else {
                false
            };

            let saved = zelf.saved.read();
            if saved.is_empty() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            // Advance and wrap in a single atomic step. A separate
            // fetch_add followed by a reset lets a second thread observe
            // an index past the end of `saved`.
            let last_index = match zelf.index.fetch_update(|index| {
                let index = if restarting { 0 } else { index };
                let next = index + 1;
                Some(if next < saved.len() { next } else { 0 })
            }) {
                Ok(index) | Err(index) => {
                    if restarting {
                        0
                    } else {
                        index
                    }
                }
            };
            let item = saved[last_index].clone();

            Ok(PyIterReturn::Return(item))
        }
    }

    #[pyattr]
    #[pyclass(name = "repeat", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsRepeat {
        object: PyObjectRef,
        #[pytraverse(skip)]
        times: Option<PyRwLock<usize>>,
    }

    #[derive(FromArgs)]
    struct PyRepeatNewArgs {
        object: PyObjectRef,
        #[pyarg(any, optional)]
        times: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyItertoolsRepeat {
        type Args = PyRepeatNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { object, times }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let times = match times {
                OptionalArg::Present(obj) => {
                    let val = index_to_ssize(&obj, vm)?;
                    // times always >= 0.
                    Some(PyRwLock::new(val.to_usize().unwrap_or(0)))
                }
                OptionalArg::Missing => None,
            };
            Ok(Self { object, times })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor, Representable), flags(BASETYPE))]
    impl Py<PyItertoolsRepeat> {
        #[pymethod]
        fn __length_hint__(&self, vm: &VirtualMachine) -> PyResult<usize> {
            // Return TypeError, length_hint picks this up and returns the default.
            let times = self
                .times
                .as_ref()
                .ok_or_else(|| vm.new_type_error("length of unsized object."))?;
            Ok(*times.read())
        }
    }

    impl SelfIter for PyItertoolsRepeat {}

    impl IterNext for PyItertoolsRepeat {
        fn next(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if let Some(ref times) = zelf.times {
                let mut times = times.write();
                if *times == 0 {
                    return Ok(PyIterReturn::StopIteration(None));
                }
                *times -= 1;
            }
            Ok(PyIterReturn::Return(zelf.object.clone()))
        }
    }

    impl Representable for PyItertoolsRepeat {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            let mut result = Wtf8Buf::from("repeat(");
            result.push_wtf8(zelf.object.repr(vm)?.as_wtf8());
            if let Some(ref times) = zelf.times {
                result.push_str(", ");
                result.push_str(&times.read().to_string());
            }
            result.push_char(')');
            Ok(result)
        }
    }

    #[pyattr]
    #[pyclass(name = "starmap", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsStarmap {
        function: PyObjectRef,
        iterable: PyIter,
    }

    #[derive(FromArgs)]
    struct StarmapNewArgs {
        #[pyarg(positional)]
        function: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsStarmap {
        type Args = StarmapNewArgs;
        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { function, iterable }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self { function, iterable })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsStarmap {}

    impl SelfIter for PyItertoolsStarmap {}

    impl IterNext for PyItertoolsStarmap {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let obj = zelf.iterable.next(vm)?;
            let function = &zelf.function;
            match obj {
                PyIterReturn::Return(obj) => {
                    let args: Vec<_> = obj.try_to_value(vm)?;
                    let original = if obj.class().is(vm.ctx.types.tuple_type) {
                        Some(obj)
                    } else {
                        // Conversion consumes a non-exact tuple before the
                        // function runs, including any reentrant finalizer.
                        drop(obj);
                        None
                    };
                    let result = function.call(args, vm);
                    drop(original);
                    PyIterReturn::from_pyresult(result, vm)
                }
                PyIterReturn::StopIteration(v) => Ok(PyIterReturn::StopIteration(v)),
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "takewhile", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsTakewhile {
        predicate: PyObjectRef,
        iterable: PyIter,
        #[pytraverse(skip)]
        stop_flag: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct TakewhileNewArgs {
        #[pyarg(positional)]
        predicate: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsTakewhile {
        type Args = TakewhileNewArgs;
        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                predicate,
                iterable,
                stop_flag: AtomicCell::new(false),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsTakewhile {}

    impl SelfIter for PyItertoolsTakewhile {}

    impl IterNext for PyItertoolsTakewhile {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.stop_flag.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            // might be StopIteration or anything else, which is propagated upwards
            let obj = raise_if_stop!(zelf.iterable.next(vm)?);
            let predicate = &zelf.predicate;

            let pred_value = predicate.call((obj.clone(),), vm)?;
            let verdict = pred_value.try_to_bool(vm)?;
            drop(pred_value);
            if verdict {
                Ok(PyIterReturn::Return(obj))
            } else {
                drop(obj);
                zelf.stop_flag.store(true);
                Ok(PyIterReturn::StopIteration(None))
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "dropwhile", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsDropwhile {
        predicate: PyObjectRef,
        iterable: PyIter,
        #[pytraverse(skip)]
        start_flag: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct DropwhileNewArgs {
        #[pyarg(positional)]
        predicate: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsDropwhile {
        type Args = DropwhileNewArgs;
        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                predicate,
                iterable,
                start_flag: AtomicCell::new(false),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsDropwhile {}

    impl SelfIter for PyItertoolsDropwhile {}

    impl IterNext for PyItertoolsDropwhile {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            loop {
                let obj = raise_if_stop!(iterable.next(vm)?);
                if zelf.start_flag.load() {
                    return Ok(PyIterReturn::Return(obj));
                }
                let pred_value = predicate.call((obj.clone(),), vm)?;
                let verdict = pred_value.try_to_bool(vm)?;
                drop(pred_value);
                if !verdict {
                    zelf.start_flag.store(true);
                    return Ok(PyIterReturn::Return(obj));
                }
            }
        }
    }

    #[derive(Default, Traverse)]
    struct GroupByState {
        current_value: Option<PyObjectRef>,
        current_key: Option<PyObjectRef>,
        tgtkey: Option<PyObjectRef>,
        #[pytraverse(skip)]
        grouper: Option<PyWeakRef<PyItertoolsGrouper>>,
    }

    impl fmt::Debug for GroupByState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("GroupByState")
                .field("current_value", &self.current_value)
                .field("current_key", &self.current_key)
                .field("tgtkey", &self.tgtkey)
                .finish()
        }
    }

    impl GroupByState {
        fn current_grouper(&self) -> Option<PyRef<PyItertoolsGrouper>> {
            self.grouper.as_ref().and_then(|g| g.upgrade())
        }
    }

    #[pyattr]
    #[pyclass(name = "groupby", traverse)]
    #[derive(PyPayload)]
    struct PyItertoolsGroupBy {
        iterable: PyIter,
        key_func: Option<PyObjectRef>,
        state: PyMutex<GroupByState>,
    }

    impl fmt::Debug for PyItertoolsGroupBy {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PyItertoolsGroupBy")
                .field("iterable", &self.iterable)
                .field("key_func", &self.key_func)
                .field("state", &self.state.lock())
                .finish()
        }
    }

    #[derive(FromArgs)]
    struct GroupByArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        key: Option<PyObjectRef>,
    }

    impl Constructor for PyItertoolsGroupBy {
        type Args = GroupByArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, key }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let iterator = iterable.get_iter(vm)?;
            Ok(Self {
                iterable: iterator,
                key_func: key,
                state: PyMutex::new(GroupByState::default()),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsGroupBy {
        pub(super) fn advance(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<PyIterReturn<(PyObjectRef, PyObjectRef)>> {
            let new_value = raise_if_stop!(self.iterable.next(vm)?);
            let new_key = if let Some(ref kf) = self.key_func {
                kf.call((new_value.clone(),), vm)?
            } else {
                new_value.clone()
            };
            Ok(PyIterReturn::Return((new_value, new_key)))
        }
    }

    impl SelfIter for PyItertoolsGroupBy {}

    impl IterNext for PyItertoolsGroupBy {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            {
                let mut state = zelf.state.lock();
                state.grouper = None;
            }

            loop {
                let (tgtkey, currkey) = {
                    let state = zelf.state.lock();
                    (state.tgtkey.clone(), state.current_key.clone())
                };
                match (tgtkey, currkey) {
                    (_, None) => {}
                    (None, Some(_)) => break,
                    (Some(tgtkey), Some(currkey)) => {
                        if !vm.bool_eq(&tgtkey, &currkey)? {
                            break;
                        }
                    }
                }
                let (value, key) = raise_if_stop!(zelf.advance(vm)?);
                let (_old_value, _old_key) = {
                    let mut state = zelf.state.lock();
                    (
                        state.current_value.replace(value),
                        state.current_key.replace(key),
                    )
                };
            }

            let old_tgtkey = {
                let mut state = zelf.state.lock();
                let Some(currkey) = state.current_key.clone() else {
                    return Ok(PyIterReturn::StopIteration(None));
                };
                state.tgtkey.replace(currkey)
            };
            drop(old_tgtkey);

            let mut state = zelf.state.lock();
            // A target-key finalizer may have reentered and consumed the current item.
            let Some(currkey) = state.current_key.clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let Some(tgtkey) = state.tgtkey.clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let grouper = PyItertoolsGrouper {
                groupby: zelf.to_owned(),
                tgtkey,
            }
            .into_ref(&vm.ctx);

            state.grouper = Some(grouper.downgrade(None, vm).unwrap());
            Ok(PyIterReturn::Return((currkey, grouper).to_pyobject(vm)))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
        tgtkey: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct GrouperArgs {
        #[pyarg(positional)]
        parent: PyRef<PyItertoolsGroupBy>,
        #[pyarg(positional)]
        tgtkey: PyObjectRef,
    }

    impl Constructor for PyItertoolsGrouper {
        type Args = GrouperArgs;

        // The new grouper becomes the parent's current grouper.
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args: Self::Args = args.bind_for(vm, crate::function::Callee::of::<Self>(vm))?;
            let grouper = Self::py_new(&cls, args, vm)?.into_ref_with_type(vm, cls)?;
            grouper.groupby.state.lock().grouper = Some(grouper.downgrade(None, vm)?);
            Ok(grouper.into())
        }

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { parent, tgtkey }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                groupby: parent,
                tgtkey,
            })
        }
    }

    #[pyclass(with(Constructor, IterNext, Iterable), flags(HAS_WEAKREF))]
    impl PyItertoolsGrouper {}

    impl SelfIter for PyItertoolsGrouper {}

    impl IterNext for PyItertoolsGrouper {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let current = zelf.groupby.state.lock().current_grouper();
            if !current.as_ref().is_some_and(|g| zelf.is(g)) {
                return Ok(PyIterReturn::StopIteration(None));
            }
            drop(current);

            if zelf.groupby.state.lock().current_value.is_none() {
                let (value, key) = raise_if_stop!(zelf.groupby.advance(vm)?);
                let (_old_value, _old_key) = {
                    let mut state = zelf.groupby.state.lock();
                    (
                        state.current_value.replace(value),
                        state.current_key.replace(key),
                    )
                };
            }

            let current;
            let currkey = {
                let state = zelf.groupby.state.lock();
                current = state.current_grouper();
                if !current.as_ref().is_some_and(|g| zelf.is(g)) {
                    return Ok(PyIterReturn::StopIteration(None));
                }
                let Some(currkey) = state.current_key.clone() else {
                    return Ok(PyIterReturn::StopIteration(None));
                };
                currkey
            };
            drop(current);
            let tgtkey = zelf.tgtkey.clone();
            if !vm.bool_eq(&tgtkey, &currkey)? {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let current;
            let (value, _old_key) = {
                let mut state = zelf.groupby.state.lock();
                current = state.current_grouper();
                if !current.as_ref().is_some_and(|g| zelf.is(g)) {
                    return Ok(PyIterReturn::StopIteration(None));
                }
                (state.current_value.take(), state.current_key.take())
            };
            drop(current);
            match value {
                Some(v) => Ok(PyIterReturn::Return(v)),
                None => Ok(PyIterReturn::StopIteration(None)),
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "islice", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsIslice {
        iterable: PyMutex<Option<PyIter>>,
        #[pytraverse(skip)]
        cur: AtomicCell<usize>,
        #[pytraverse(skip)]
        next: AtomicCell<usize>,
        #[pytraverse(skip)]
        stop: Option<usize>,
        #[pytraverse(skip)]
        step: usize,
    }

    #[pyclass(with(IterNext, Iterable), flags(BASETYPE))]
    impl PyItertoolsIslice {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args =
                crate::types::drop_kwargs_if_init_overridden(&cls, Self::class(&vm.ctx), args);
            let (iter, a1, a2, a3): (
                PyObjectRef,
                PyObjectRef,
                OptionalArg<PyObjectRef>,
                OptionalArg<PyObjectRef>,
            ) = args.bind_for(vm, Self::NAME)?;

            // CPython clears index conversion errors and reports the appropriate
            // islice ValueError after converting arguments in this order.
            let to_ssize = |obj: &PyObject| isize::try_from_borrowed_object(vm, obj).unwrap_or(-1);
            let mut start = 0;
            let mut stop = -1;
            if let OptionalArg::Present(a2) = &a2 {
                if !vm.is_none(&a1) {
                    start = to_ssize(&a1);
                }
                if !vm.is_none(a2) {
                    stop = to_ssize(a2);
                    if stop == -1 {
                        return Err(vm.new_value_error(
                            "Stop argument for islice() must be None or an integer: 0 <= x <= sys.maxsize.",
                        ));
                    }
                }
            } else if !vm.is_none(&a1) {
                stop = to_ssize(&a1);
                if stop == -1 {
                    return Err(vm.new_value_error(
                        "Stop argument for islice() must be None or an integer: 0 <= x <= sys.maxsize.",
                    ));
                }
            }
            if start < 0 || stop < -1 {
                return Err(vm.new_value_error(
                    "Indices for islice() must be None or an integer: 0 <= x <= sys.maxsize.",
                ));
            }

            let step = if let OptionalArg::Present(a3) = &a3
                && !vm.is_none(a3)
            {
                to_ssize(a3)
            } else {
                1
            };
            if step < 1 {
                return Err(
                    vm.new_value_error("Step for islice() must be a positive integer or None.")
                );
            }
            let start = start as usize;
            let stop = (stop >= 0).then_some(stop as usize);
            let step = step as usize;

            let iter = PyIter::try_from_object(vm, iter)?;

            Self {
                iterable: PyMutex::new(Some(iter)),
                cur: AtomicCell::new(0),
                next: AtomicCell::new(start),
                stop,
                step,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl PyItertoolsIslice {
        fn clear_iterable(&self) {
            let _previous = self.iterable.lock().take();
        }

        fn next_source(&self, iterable: &PyIter, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let result = iterable.next(vm);
            if !matches!(&result, Ok(PyIterReturn::Return(_))) {
                self.clear_iterable();
            }
            result
        }
    }

    impl SelfIter for PyItertoolsIslice {}

    impl IterNext for PyItertoolsIslice {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let Some(iterable) = zelf.iterable.lock().clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let stop = zelf.stop.unwrap_or(usize::MAX);

            while zelf.cur.load() < zelf.next.load() {
                raise_if_stop!(zelf.next_source(&iterable, vm)?);
                zelf.cur.fetch_add(1);
            }
            if zelf.cur.load() >= stop {
                zelf.clear_iterable();
                return Ok(PyIterReturn::StopIteration(None));
            }

            let obj = raise_if_stop!(zelf.next_source(&iterable, vm)?);
            zelf.cur.fetch_add(1);
            let oldnext = zelf.next.load();
            let (newnext, ovf) = oldnext.overflowing_add(zelf.step);
            zelf.next
                .store(if ovf || newnext > stop { stop } else { newnext });
            Ok(PyIterReturn::Return(obj))
        }
    }

    #[pyattr]
    #[pyclass(name = "filterfalse", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsFilterFalse {
        predicate: PyObjectRef,
        iterable: PyIter,
    }

    #[derive(FromArgs)]
    struct FilterFalseNewArgs {
        #[pyarg(positional)]
        function: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsFilterFalse {
        type Args = FilterFalseNewArgs;
        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { function, iterable }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                predicate: function,
                iterable,
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsFilterFalse {}

    impl SelfIter for PyItertoolsFilterFalse {}

    impl IterNext for PyItertoolsFilterFalse {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            loop {
                let obj = raise_if_stop!(iterable.next(vm)?);
                let pred_value = if vm.is_none(predicate) {
                    obj.clone()
                } else {
                    predicate.call((obj.clone(),), vm)?
                };

                let verdict = pred_value.try_to_bool(vm)?;
                if !verdict {
                    return Ok(PyIterReturn::Return(obj));
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "accumulate", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsAccumulate {
        iterable: PyIter,
        bin_op: Option<PyObjectRef>,
        initial: PyRwLock<Option<PyObjectRef>>,
        #[pytraverse(skip)]
        has_initial: bool,
        acc_value: PyRwLock<Option<PyObjectRef>>,
    }

    #[derive(FromArgs)]
    struct AccumulateArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        func: Option<PyObjectRef>,
        #[pyarg(named, optional)]
        initial: Option<PyObjectRef>,
    }

    impl Constructor for PyItertoolsAccumulate {
        type Args = AccumulateArgs;

        fn py_new(_cls: &Py<PyType>, args: AccumulateArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let iterator = args.iterable.get_iter(vm)?;
            Ok(Self {
                iterable: iterator,
                bin_op: args.func,
                has_initial: args.initial.is_some(),
                initial: PyRwLock::new(args.initial),
                acc_value: PyRwLock::new(None),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsAccumulate {}

    impl SelfIter for PyItertoolsAccumulate {}

    impl IterNext for PyItertoolsAccumulate {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let iterable = &zelf.iterable;

            if zelf.has_initial {
                let mut acc = zelf.acc_value.write();
                if acc.is_none() {
                    let initial = zelf.initial.write().take();
                    if let Some(initial) = initial {
                        *acc = Some(initial.clone());
                        return Ok(PyIterReturn::Return(initial));
                    }
                }
            }

            let obj = raise_if_stop!(iterable.next(vm)?);
            // The source callback can reenter and replace the running total.
            let acc_value = zelf.acc_value.read().clone();
            let next_acc_value = match acc_value {
                None => obj,
                Some(value) => match &zelf.bin_op {
                    None => {
                        let result = vm._add(&value, &obj);
                        drop(value);
                        drop(obj);
                        result?
                    }
                    Some(op) => op.call((value, obj), vm)?,
                },
            };
            let _previous = zelf.acc_value.write().replace(next_acc_value.clone());

            Ok(PyIterReturn::Return(next_acc_value))
        }
    }

    #[pyattr]
    #[pyclass(name = "_tee_dataobject", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsTeeData {
        iterable: PyIter,
        values: PyMutex<Vec<PyObjectRef>>,
        next: PyMutex<Option<PyRef<Self>>>,
        #[pytraverse(skip)]
        running: AtomicBool,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PyItertoolsTeeData {
        const BLOCK_SIZE: usize = 64;

        fn new(iterable: PyIter, vm: &VirtualMachine) -> PyRef<Self> {
            Self {
                iterable,
                values: PyMutex::new(Vec::with_capacity(Self::BLOCK_SIZE)),
                next: PyMutex::new(None),
                running: AtomicBool::new(false),
            }
            .into_ref(&vm.ctx)
        }

        fn next_block(&self, vm: &VirtualMachine) -> PyRef<Self> {
            self.next
                .lock()
                .get_or_insert_with(|| Self::new(self.iterable.clone(), vm))
                .clone()
        }
    }

    #[pyattr]
    #[pyclass(name = "_tee", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsTee {
        tee_data: PyMutex<PyRef<PyItertoolsTeeData>>,
        #[pytraverse(skip)]
        index: AtomicCell<usize>,
        #[pytraverse(skip)]
        advancing: AtomicBool,
    }

    #[derive(FromArgs)]
    struct TeeNewArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
    }

    impl Constructor for PyItertoolsTee {
        type Args = TeeNewArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // Keep the source alive until an existing tee's position is copied:
            // its finalizer can advance the iterator returned by __iter__.
            let iterator = args.iterable.get_iter(vm)?;
            // An iterator that is already a tee shares its buffer rather than
            // getting one of its own.
            if let Some(tee) = iterator.as_object().downcast_ref::<Self>() {
                return Ok(tee.__copy__());
            }
            Ok(Self {
                tee_data: PyMutex::new(PyItertoolsTeeData::new(iterator, vm)),
                index: AtomicCell::new(0),
                advancing: AtomicBool::new(false),
            })
        }
    }

    impl PyItertoolsTee {
        fn from_iter(iterator: PyIter, vm: &VirtualMachine) -> PyResult {
            // An iterator is asked for its iterator again, which validates its __iter__.
            let iterator = iterator.get_iter(vm)?;
            let class = Self::class(&vm.ctx);
            if iterator.class().is(class) {
                return vm.call_special_method(&iterator, identifier!(vm, __copy__), ());
            }
            Ok(Self {
                tee_data: PyMutex::new(PyItertoolsTeeData::new(iterator, vm)),
                index: AtomicCell::new(0),
                advancing: AtomicBool::new(false),
            }
            .into_ref_with_type(vm, class.to_owned())?
            .into())
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(HAS_WEAKREF))]
    impl Py<PyItertoolsTee> {
        #[pymethod]
        fn __copy__(&self) -> PyItertoolsTee {
            // A block transition resets the index under this lock, so the pair
            // is a consistent snapshot even while another thread advances us.
            let data = self.tee_data.lock();
            PyItertoolsTee {
                tee_data: PyMutex::new(data.clone()),
                index: AtomicCell::new(self.index.load()),
                advancing: AtomicBool::new(false),
            }
        }
    }

    #[derive(FromArgs)]
    struct TeeArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional, optional, py_default = "2")]
        n: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn tee(args: TeeArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let TeeArgs { iterable, n } = args;
        let n_value = match &n {
            OptionalArg::Present(value) => index_to_ssize(value, vm)?,
            OptionalArg::Missing => 2,
        };
        if n_value < 0 {
            return Err(vm.new_value_error("n cannot be negative"));
        }
        let n = n_value as usize;

        let mut tee_vec: Vec<PyObjectRef> = Vec::new();
        tee_vec
            .try_reserve_exact(n)
            .map_err(|_| vm.no_memory_error())?;
        if n == 0 {
            return Ok(vm.ctx.empty_tuple.clone());
        }

        let copyable = PyItertoolsTee::from_iter(iterable.get_iter(vm)?, vm)?;
        for _ in 0..n {
            tee_vec.push(vm.call_special_method(&copyable, identifier!(vm, __copy__), ())?);
        }

        Ok(PyTuple::new_ref(tee_vec, &vm.ctx))
    }
    impl SelfIter for PyItertoolsTee {}
    impl IterNext for PyItertoolsTee {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut current = zelf.tee_data.lock();
            // Reading the index and moving it on is one step: two callers that
            // read the same index hand out the same value twice and leave the
            // buffer to be filled out of order.
            // The block lock serializes claims; an atomic exchange is not needed.
            if zelf.advancing.load(Ordering::Acquire) {
                drop(current);
                return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
            }
            zelf.advancing.store(true, Ordering::Relaxed);
            scopeguard::defer! { zelf.advancing.store(false, Ordering::Release) }
            if zelf.index.load() == PyItertoolsTeeData::BLOCK_SIZE {
                let data = current.clone();
                drop(current);
                let next = data.next_block(vm);
                current = zelf.tee_data.lock();
                let previous = core::mem::replace(&mut *current, next);
                zelf.index.store(0);
                drop(current);
                // Releasing a consumed block may run item finalizers. No cursor
                // or buffer lock may be held when the last owner lets it go.
                drop(data);
                drop(previous);
                current = zelf.tee_data.lock();
            }

            // Cached reads neither call Python nor release the block, so they
            // can update the cursor without cloning its data reference.
            let index = zelf.index.load();
            let cached = {
                let values = current.values.lock();
                if let Some(value) = values.get(index) {
                    Some(value.clone())
                } else {
                    // Claim the source while the cache is locked: another tee
                    // cannot fill this slot between the check and the claim.
                    if current.running.load(Ordering::Acquire) {
                        drop(values);
                        drop(current);
                        return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
                    }
                    current.running.store(true, Ordering::Relaxed);
                    None
                }
            };
            if let Some(value) = cached {
                zelf.index.store(index + 1);
                return Ok(PyIterReturn::Return(value));
            }

            let data = current.clone();
            drop(current);
            // Prevent concurrent/reentrant calls to iterable.next(). The claim
            // covers caching the value as well: released any earlier, a second
            // tee at the same index fetches a value of its own and one of the
            // two is dropped without ever reaching a caller.
            scopeguard::defer! { data.running.store(false, Ordering::Release) }
            let value = raise_if_stop!(data.iterable.next(vm)?);
            data.values.lock().push(value.clone());
            zelf.index.store(index + 1);
            Ok(PyIterReturn::Return(value))
        }
    }

    #[pyattr]
    #[pyclass(name = "product", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsProduct {
        pools: Vec<PyTupleRef>,
        #[pytraverse(skip)]
        idxs: PyRwLock<Vec<usize>>,
        #[pytraverse(skip)]
        stop: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct ProductArgs {
        #[pyarg(named, default = 1)]
        repeat: isize,
    }

    impl Constructor for PyItertoolsProduct {
        type Args = (PosArgs<PyObjectRef, NameIterables>, ProductArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (iterables, args): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let repeat = args.repeat;
            if repeat < 0 {
                return Err(vm.new_value_error("repeat argument cannot be negative"));
            }
            let repeat = repeat as usize;

            // The count is settled before the arguments are read, the way
            // `product_new()` settles it before it calls `PySequence_Tuple()`
            // on any of them, so a repeat too large to serve does not run their
            // code first.
            let npools = iterables
                .iter()
                .len()
                .checked_mul(repeat)
                .filter(|n| *n <= isize::MAX as usize / size_of::<usize>())
                .ok_or_else(|| vm.new_overflow_error("repeat argument too large"))?;

            let mut single = Vec::new();
            if repeat != 0 {
                for arg in iterables.iter() {
                    let pool = if let Some(tuple) = arg.downcast_ref_if_exact::<PyTuple>(vm) {
                        tuple.to_owned()
                    } else {
                        PyTuple::new_ref(arg.try_to_value(vm)?, &vm.ctx)
                    };
                    single.push(pool);
                }
            }

            let mut pools: Vec<PyTupleRef> = Vec::new();
            pools
                .try_reserve_exact(npools)
                .map_err(|_| vm.no_memory_error())?;
            // Filled by index, the way `product_new()` fills a tuple of
            // `npools`. Repeating the arguments `repeat` times instead walks
            // that many steps even when there are no arguments to repeat, so
            // `product(repeat=2**62)` would spin rather than answer `[()]`.
            pools.extend((0..npools).map(|i| single[i % single.len()].clone()));

            let mut idxs = Vec::new();
            idxs.try_reserve_exact(npools)
                .map_err(|_| vm.no_memory_error())?;
            idxs.resize(npools, 0);

            let empty = pools.iter().any(|pool| pool.is_empty());

            Ok(Self {
                pools,
                idxs: PyRwLock::new(idxs),
                stop: AtomicCell::new(empty),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsProduct {
        fn update_idxs(&self, idxs: &mut [usize]) {
            for (index, pool) in idxs.iter_mut().zip(&self.pools).rev() {
                *index += 1;
                if *index < pool.len() {
                    return;
                }
                *index = 0;
            }
            self.stop.store(true);
        }
    }

    impl SelfIter for PyItertoolsProduct {}
    impl IterNext for PyItertoolsProduct {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut idxs = zelf.idxs.write();
            // stop signal
            if zelf.stop.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let pools = &zelf.pools;

            let res = vm.ctx.new_tuple(
                pools
                    .iter()
                    .zip(idxs.iter())
                    .map(|(pool, idx)| pool[*idx].clone())
                    .collect(),
            );

            zelf.update_idxs(&mut idxs);

            Ok(PyIterReturn::Return(res.into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCombinations {
        pool: Vec<PyObjectRef>,
        #[pytraverse(skip)]
        indices: PyRwLock<Vec<usize>>,
        result: PyRwLock<Option<Vec<PyObjectRef>>>,
        #[pytraverse(skip)]
        r: AtomicCell<usize>,
        #[pytraverse(skip)]
        exhausted: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct CombinationsNewArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any)]
        r: PyObjectRef,
    }

    fn index_to_ssize(obj: &PyObject, vm: &VirtualMachine) -> PyResult<isize> {
        let index;
        let int = if let Some(int) = obj.downcast_ref::<PyInt>() {
            int
        } else {
            index = obj.try_index(vm)?;
            &index
        };
        int.as_bigint()
            .to_isize()
            .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to C ssize_t"))
    }

    fn combinations_r(r: &PyObject, vm: &VirtualMachine) -> PyResult<usize> {
        let value = index_to_ssize(r, vm)?;
        if value < 0 {
            return Err(vm.new_value_error("r cannot be negative"));
        }
        Ok(value as usize)
    }

    impl Constructor for PyItertoolsCombinations {
        type Args = CombinationsNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let r_value = combinations_r(&r, vm)?;
            let pool: Vec<_> = iterable.try_to_value(vm)?;
            let r = r_value;
            let n = pool.len();

            let mut indices = Vec::new();
            indices
                .try_reserve_exact(r)
                .map_err(|_| vm.no_memory_error())?;
            indices.extend(0..r);

            Ok(Self {
                pool,
                indices: PyRwLock::new(indices),
                result: PyRwLock::new(None),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsCombinations {}

    impl SelfIter for PyItertoolsCombinations {}
    impl IterNext for PyItertoolsCombinations {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // stop signal
            if zelf.exhausted.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if r == 0 {
                zelf.exhausted.store(true);
                return Ok(PyIterReturn::Return(vm.new_tuple(()).into()));
            }

            let mut result_lock = zelf.result.write();
            let result = if let Some(ref mut result) = *result_lock {
                let mut indices = zelf.indices.write();

                // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
                let mut idx = r as isize - 1;
                while idx >= 0 && indices[idx as usize] == idx as usize + n - r {
                    idx -= 1;
                }

                // If no suitable index is found, then the indices are all at
                // their maximum value and we're done.
                if idx < 0 {
                    zelf.exhausted.store(true);
                    return Ok(PyIterReturn::StopIteration(None));
                }

                // Increment the current index which we know is not at its
                // maximum.  Then move back to the right setting each index
                // to its lowest possible value (one higher than the index
                // to its left -- this maintains the sort order invariant).
                indices[idx as usize] += 1;
                for j in idx as usize + 1..r {
                    indices[j] = indices[j - 1] + 1;
                }

                // Update the result tuple for the new indices
                // starting with i, the leftmost index that changed
                for i in idx as usize..r {
                    let index = indices[i];
                    let elem = &zelf.pool[index];
                    elem.clone_into(&mut result[i]);
                }

                result.to_vec()
            } else {
                let res = zelf.pool[0..r].to_vec();
                *result_lock = Some(res.clone());
                res
            };

            Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations_with_replacement", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCombinationsWithReplacement {
        pool: Vec<PyObjectRef>,
        #[pytraverse(skip)]
        indices: PyRwLock<Vec<usize>>,
        #[pytraverse(skip)]
        r: AtomicCell<usize>,
        #[pytraverse(skip)]
        exhausted: AtomicCell<bool>,
    }

    impl Constructor for PyItertoolsCombinationsWithReplacement {
        type Args = CombinationsNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let r_value = combinations_r(&r, vm)?;
            let pool: Vec<_> = iterable.try_to_value(vm)?;
            let r = r_value;
            let n = pool.len();

            let mut indices = Vec::new();
            indices
                .try_reserve_exact(r)
                .map_err(|_| vm.no_memory_error())?;
            indices.resize(r, 0);

            Ok(Self {
                pool,
                indices: PyRwLock::new(indices),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(n == 0 && r > 0),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsCombinationsWithReplacement {}

    impl SelfIter for PyItertoolsCombinationsWithReplacement {}

    impl IterNext for PyItertoolsCombinationsWithReplacement {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // stop signal
            if zelf.exhausted.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if r == 0 {
                zelf.exhausted.store(true);
                return Ok(PyIterReturn::Return(vm.new_tuple(()).into()));
            }

            let mut indices = zelf.indices.write();

            let res = vm
                .ctx
                .new_tuple(indices.iter().map(|&i| zelf.pool[i].clone()).collect());

            // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
            let mut idx = r as isize - 1;
            while idx >= 0 && indices[idx as usize] == n - 1 {
                idx -= 1;
            }

            // If no suitable index is found, then the indices are all at
            // their maximum value and we're done.
            if idx < 0 {
                zelf.exhausted.store(true);
            } else {
                let index = indices[idx as usize] + 1;

                // Increment the current index which we know is not at its
                // maximum. Then set all to the right to the same value.
                for j in idx as usize..r {
                    indices[j] = index;
                }
            }

            Ok(PyIterReturn::Return(res.into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "permutations", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsPermutations {
        pool: Vec<PyObjectRef>, // Collected input iterable
        #[pytraverse(skip)]
        indices: PyRwLock<Vec<usize>>, // One index per element in pool
        #[pytraverse(skip)]
        cycles: PyRwLock<Vec<usize>>, // One rollover counter per element in the result
        #[pytraverse(skip)]
        result: PyRwLock<Option<Vec<usize>>>, // Indexes of the most recently returned result
        #[pytraverse(skip)]
        r: AtomicCell<usize>, // Size of result tuple
        #[pytraverse(skip)]
        exhausted: AtomicCell<bool>, // Set when the iterator is exhausted
    }

    #[derive(FromArgs)]
    struct PermutationsNewArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        r: Option<PyObjectRef>,
    }

    impl Constructor for PyItertoolsPermutations {
        type Args = PermutationsNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let pool: Vec<_> = iterable.try_to_value(vm)?;

            let n = pool.len();
            // If r is not provided, r == n. If provided, r must be a positive integer, or None.
            // If None, it behaves the same as if it was not provided.
            let r = match &r {
                Some(r) => {
                    let value = r
                        .downcast_ref::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("Expected int as r"))?
                        .as_bigint()
                        .to_isize()
                        .ok_or_else(|| {
                            vm.new_overflow_error("Python int too large to convert to C ssize_t")
                        })?;
                    if value < 0 {
                        return Err(vm.new_value_error("r must be non-negative"));
                    }
                    value as usize
                }
                None => n,
            };

            Ok(Self {
                pool,
                indices: PyRwLock::new((0..n).collect()),
                cycles: PyRwLock::new((0..r.min(n)).map(|i| n - i).collect()),
                result: PyRwLock::new(None),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsPermutations {}

    impl SelfIter for PyItertoolsPermutations {}

    impl IterNext for PyItertoolsPermutations {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // stop signal
            if zelf.exhausted.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let n = zelf.pool.len();
            let r = zelf.r.load();

            if n == 0 {
                zelf.exhausted.store(true);
                return Ok(PyIterReturn::Return(vm.new_tuple(()).into()));
            }

            let mut result = zelf.result.write();

            if let Some(ref mut result) = *result {
                let mut indices = zelf.indices.write();
                let mut cycles = zelf.cycles.write();
                let mut sentinel = false;

                // Decrement rightmost cycle, moving leftward upon zero rollover
                for i in (0..r).rev() {
                    cycles[i] -= 1;

                    if cycles[i] == 0 {
                        // rotation: indices[i:] = indices[i+1:] + indices[i:i+1]
                        let index = indices[i];
                        for j in i..n - 1 {
                            indices[j] = indices[j + 1];
                        }
                        indices[n - 1] = index;
                        cycles[i] = n - i;
                    } else {
                        let j = cycles[i];
                        indices.swap(i, n - j);

                        for k in i..r {
                            // start with i, the leftmost element that changed
                            // yield tuple(pool[k] for k in indices[:r])
                            result[k] = indices[k];
                        }
                        sentinel = true;
                        break;
                    }
                }
                if !sentinel {
                    zelf.exhausted.store(true);
                    return Ok(PyIterReturn::StopIteration(None));
                }
            } else {
                // On the first pass, initialize result tuple using the indices
                *result = Some((0..r).collect());
            }

            Ok(PyIterReturn::Return(
                vm.ctx
                    .new_tuple(
                        result
                            .as_ref()
                            .unwrap()
                            .iter()
                            .map(|&i| zelf.pool[i].clone())
                            .collect(),
                    )
                    .into(),
            ))
        }
    }

    #[derive(FromArgs)]
    struct ZipLongestArgs {
        #[pyarg(named, optional)]
        fillvalue: Option<PyObjectRef>,
    }

    impl Constructor for PyItertoolsZipLongest {
        type Args = (PosArgs<PyIter, NameIterables>, ZipLongestArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (iterators, args): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let fillvalue = args.fillvalue.unwrap_or_else(|| vm.ctx.none());
            let iterators = iterators.into_vec();
            let num_active = iterators.len();
            let mut slots = Vec::new();
            slots
                .try_reserve_exact(iterators.len())
                .map_err(|_| vm.no_memory_error())?;
            slots.extend(iterators.into_iter().map(Some));
            Ok(Self {
                state: PyMutex::new(ZipLongestState {
                    iterators: slots,
                    num_active,
                }),
                fillvalue: PyRwLock::new(fillvalue),
            })
        }
    }

    #[derive(Debug, Traverse)]
    struct ZipLongestState {
        iterators: Vec<Option<PyIter>>,
        #[pytraverse(skip)]
        num_active: usize,
    }

    #[pyattr]
    #[pyclass(name = "zip_longest", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsZipLongest {
        state: PyMutex<ZipLongestState>,
        fillvalue: PyRwLock<PyObjectRef>,
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsZipLongest {}

    impl SelfIter for PyItertoolsZipLongest {}

    impl IterNext for PyItertoolsZipLongest {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let len = {
                let state = zelf.state.lock();
                if state.num_active == 0 {
                    return Ok(PyIterReturn::StopIteration(None));
                }
                state.iterators.len()
            };
            let mut result: Vec<PyObjectRef> = Vec::new();
            for idx in 0..len {
                let iterator = zelf.state.lock().iterators[idx].clone();
                let next_obj = if let Some(iterator) = iterator {
                    let next = iterator.next(vm);
                    drop(iterator);
                    match next {
                        Ok(PyIterReturn::Return(obj)) => obj,
                        Ok(PyIterReturn::StopIteration(value)) => {
                            drop(value);
                            let (terminal, previous) = {
                                let mut state = zelf.state.lock();
                                // A reentrant or competing call may already
                                // have exhausted this slot or the whole zip.
                                if state.num_active > 0 && state.iterators[idx].is_some() {
                                    state.num_active -= 1;
                                }
                                let terminal = state.num_active == 0;
                                let previous = if terminal {
                                    None
                                } else {
                                    state.iterators[idx].take()
                                };
                                (terminal, previous)
                            };
                            if terminal {
                                // The final exhausted source remains owned,
                                // unlike exhausted sources in earlier slots.
                                return Ok(PyIterReturn::StopIteration(None));
                            }
                            drop(previous);
                            zelf.fillvalue.read().clone()
                        }
                        Err(err) => {
                            zelf.state.lock().num_active = 0;
                            return Err(err);
                        }
                    }
                } else {
                    zelf.fillvalue.read().clone()
                };
                result.push(next_obj);
            }
            Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "pairwise", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsPairwise {
        iterator: PyMutex<Option<PyIter>>,
        old: PyRwLock<Option<PyObjectRef>>,
        #[pytraverse(skip)]
        terminal: AtomicBool,
    }

    impl Constructor for PyItertoolsPairwise {
        type Args = IterablePosArg;

        const DROP_KWARGS_WHEN_INIT_OVERRIDDEN: bool = true;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            let iterator = args.iterable;
            Ok(Self {
                iterator: PyMutex::new(Some(iterator)),
                old: PyRwLock::new(None),
                terminal: AtomicBool::new(false),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsPairwise {}

    impl SelfIter for PyItertoolsPairwise {}

    impl PyItertoolsPairwise {
        fn next_item(&self, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let Some(iterator) = self.iterator.lock().clone() else {
                return Ok(PyIterReturn::StopIteration(None));
            };
            let next = iterator.next(vm);
            drop(iterator);
            next
        }

        fn clear_iterator(&self) {
            self.terminal.store(true, Ordering::Release);
            let previous = self.iterator.lock().take();
            drop(previous);
        }

        fn clear_old(&self) {
            let previous = self.old.write().take();
            drop(previous);
        }
    }

    impl IterNext for PyItertoolsPairwise {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.terminal.load(Ordering::Acquire) {
                return Ok(PyIterReturn::StopIteration(None));
            }
            let old_clone = {
                let guard = zelf.old.read();
                guard.clone()
            };
            let old = match old_clone {
                None => {
                    let next = zelf.next_item(vm);
                    let previous = {
                        let mut old = zelf.old.write();
                        match &next {
                            Ok(PyIterReturn::Return(obj)) => old.replace(obj.clone()),
                            _ => old.take(),
                        }
                    };
                    // The first source callback can publish an old item by
                    // reentering. Replace it before clearing a failed source.
                    drop(previous);
                    match next {
                        Ok(PyIterReturn::Return(obj)) => {
                            if zelf.terminal.load(Ordering::Acquire) {
                                zelf.clear_old();
                                return Ok(PyIterReturn::StopIteration(None));
                            }
                            obj
                        }
                        result => {
                            zelf.clear_iterator();
                            return result;
                        }
                    }
                }
                Some(obj) => obj,
            };

            let new = match zelf.next_item(vm) {
                Ok(PyIterReturn::Return(obj)) => obj,
                result => {
                    // For the second input, release the source before both
                    // the stored previous item and this call's owned copy.
                    zelf.clear_iterator();
                    zelf.clear_old();
                    drop(old);
                    return result;
                }
            };
            let previous = zelf.old.write().replace(new.clone());
            drop(previous);

            Ok(PyIterReturn::Return(vm.new_tuple((old, new)).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "batched", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsBatched {
        #[pytraverse(skip)]
        exhausted: AtomicCell<bool>,
        iterable: PyIter,
        #[pytraverse(skip)]
        n: AtomicCell<usize>,
        #[pytraverse(skip)]
        strict: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct BatchedNewArgs {
        #[pyarg(any)]
        iterable: PyObjectRef,
        #[pyarg(any)]
        n: PyObjectRef,
        #[pyarg(named, optional, py_default = "False")]
        strict: Option<PyObjectRef>,
    }

    impl Constructor for PyItertoolsBatched {
        type Args = BatchedNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args {
                iterable,
                n,
                strict,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let n_value = index_to_ssize(&n, vm)?;
            let strict_value = match &strict {
                Some(value) => value.try_to_bool(vm)?,
                None => false,
            };
            if n_value < 1 {
                return Err(vm.new_value_error("n must be at least one"));
            }
            let iterable = PyIter::try_from_object(vm, iterable)?;

            Ok(Self {
                iterable,
                n: AtomicCell::new(n_value as usize),
                exhausted: AtomicCell::new(false),
                strict: AtomicCell::new(strict_value),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE, HAS_DICT))]
    impl PyItertoolsBatched {}

    impl SelfIter for PyItertoolsBatched {}

    impl IterNext for PyItertoolsBatched {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.exhausted.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }
            let mut result: Vec<PyObjectRef> = Vec::new();
            let n = zelf.n.load();
            for _ in 0..n {
                match zelf.iterable.next(vm) {
                    Ok(PyIterReturn::Return(obj)) => {
                        result.push(obj);
                    }
                    Ok(PyIterReturn::StopIteration(value)) => {
                        drop(value);
                        if result.is_empty() || zelf.strict.load() {
                            zelf.exhausted.store(true);
                        }
                        break;
                    }
                    Err(err) => {
                        zelf.exhausted.store(true);
                        return Err(err);
                    }
                }
            }
            let res_len = result.len();
            match res_len {
                0 => Ok(PyIterReturn::StopIteration(None)),
                _ => {
                    if zelf.strict.load() && res_len != n {
                        drop(result);
                        Err(vm.new_value_error("batched(): incomplete batch"))
                    } else {
                        Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
                    }
                }
            }
        }
    }
}
