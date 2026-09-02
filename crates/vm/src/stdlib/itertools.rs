pub(crate) use decl::module_def;

#[pymodule(name = "itertools")]
mod decl {
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, PyWeakRef, VirtualMachine,
        builtins::{
            PyGenericAlias, PyInt, PyIntRef, PyList, PyTuple, PyTupleRef, PyType, PyTypeRef, int,
        },
        class::PyClassDef,
        common::lock::{PyMutex, PyRwLock, PyRwLockWriteGuard},
        convert::ToPyObject,
        function::{FuncArgs, OptionalArg, OptionalOption, PosArgs},
        protocol::{PyIter, PyIterReturn, PyNumber},
        raise_if_stop,
        stdlib::sys,
        types::{Constructor, IterNext, Iterable, Representable, SelfIter},
    };
    use core::sync::atomic::{AtomicBool, Ordering};
    use crossbeam_utils::atomic::AtomicCell;
    use malachite_bigint::BigInt;
    use num_traits::One;
    use rustpython_common::wtf8::Wtf8Buf;

    use alloc::fmt;
    use num_traits::{Signed, ToPrimitive};

    #[pyattr]
    #[pyclass(name = "chain", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsChain {
        source: PyRwLock<Option<PyIter>>,
        active: PyRwLock<Option<PyIter>>,
    }

    #[pyclass(with(IterNext, Iterable), flags(BASETYPE, HAS_DICT))]
    impl PyItertoolsChain {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args_list = PyList::from(args.args);
            Self {
                source: PyRwLock::new(Some(args_list.to_pyobject(vm).get_iter(vm)?)),
                active: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }

        #[pyclassmethod]
        fn from_iterable(
            cls: PyTypeRef,
            source: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            Self {
                source: PyRwLock::new(Some(source.get_iter(vm)?)),
                active: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
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
                            *zelf.active.write() = None;
                        }
                        Err(err) => {
                            break Err(err);
                        }
                    }
                } else {
                    match source.next(vm) {
                        Ok(PyIterReturn::Return(ok)) => match ok.get_iter(vm) {
                            Ok(iter) => {
                                *zelf.active.write() = Some(iter);
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
    struct CompressNewArgs {
        #[pyarg(any)]
        data: PyIter,
        #[pyarg(any)]
        selectors: PyIter,
    }

    impl Constructor for PyItertoolsCompress {
        type Args = CompressNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { data, selectors }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self { data, selectors })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(BASETYPE))]
    impl PyItertoolsCompress {}

    impl SelfIter for PyItertoolsCompress {}

    impl IterNext for PyItertoolsCompress {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            loop {
                let sel_obj = raise_if_stop!(zelf.selectors.next(vm)?);
                let verdict = sel_obj.clone().try_to_bool(vm)?;
                let data_obj = zelf.data.next(vm)?;

                if verdict {
                    return Ok(data_obj);
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
        #[pyarg(any, optional)]
        start: OptionalArg<PyObjectRef>,

        #[pyarg(any, optional)]
        step: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyItertoolsCount {
        type Args = CountNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { start, step }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let start = start.into_option().unwrap_or_else(|| vm.new_pyobj(0));
            let step = step.into_option().unwrap_or_else(|| vm.new_pyobj(1));
            if !PyNumber::check(&start) || !PyNumber::check(&step) {
                return Err(vm.new_type_error("a number is required"));
            }

            Ok(Self {
                cur: PyRwLock::new(start),
                step,
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor, Representable))]
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
        iter: PyIter,
        saved: PyRwLock<Vec<PyObjectRef>>,
        #[pytraverse(skip)]
        index: AtomicCell<usize>,
    }

    impl Constructor for PyItertoolsCycle {
        type Args = PyIter;

        fn py_new(_cls: &Py<PyType>, iter: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                iter,
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
            let item = if let PyIterReturn::Return(item) = zelf.iter.next(vm)? {
                zelf.saved.write().push(item.clone());
                item
            } else {
                let saved = zelf.saved.read();
                if saved.is_empty() {
                    return Ok(PyIterReturn::StopIteration(None));
                }

                // Advance and wrap in a single atomic step. A separate
                // fetch_add followed by a reset lets a second thread observe
                // an index past the end of `saved`.
                let last_index = match zelf.index.fetch_update(|index| {
                    let next = index + 1;
                    Some(if next < saved.len() { next } else { 0 })
                }) {
                    Ok(index) | Err(index) => index,
                };

                saved[last_index].clone()
            };

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
            let times = match times.into_option() {
                Some(obj) => {
                    let int = obj.try_index(vm)?;
                    let val: isize = int.try_to_primitive(vm)?;
                    // times always >= 0.
                    Some(PyRwLock::new(val.to_usize().unwrap_or(0)))
                }
                None => None,
            };
            Ok(Self { object, times })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor, Representable), flags(BASETYPE))]
    impl PyItertoolsRepeat {
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
                    PyIterReturn::from_pyresult(function.call(args, vm), vm)
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

            let verdict = predicate.call((obj.clone(),), vm)?;
            let verdict = verdict.try_to_bool(vm)?;
            if verdict {
                Ok(PyIterReturn::Return(obj))
            } else {
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

            if !zelf.start_flag.load() {
                loop {
                    let obj = raise_if_stop!(iterable.next(vm)?);
                    let pred_value = predicate.call((obj.clone(),), vm)?;
                    if !pred_value.try_to_bool(vm)? {
                        zelf.start_flag.store(true);
                        return Ok(PyIterReturn::Return(obj));
                    }
                }
            }
            iterable.next(vm)
        }
    }

    #[derive(Default, Traverse)]
    struct GroupByState {
        current_value: Option<PyObjectRef>,
        current_key: Option<PyObjectRef>,
        #[pytraverse(skip)]
        next_group: bool,
        #[pytraverse(skip)]
        grouper: Option<PyWeakRef<PyItertoolsGrouper>>,
    }

    impl fmt::Debug for GroupByState {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("GroupByState")
                .field("current_value", &self.current_value)
                .field("current_key", &self.current_key)
                .field("next_group", &self.next_group)
                .finish()
        }
    }

    impl GroupByState {
        fn is_current(&self, grouper: &Py<PyItertoolsGrouper>) -> bool {
            self.grouper
                .as_ref()
                .and_then(|g| g.upgrade())
                .is_some_and(|current_grouper| grouper.is(&current_grouper))
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
        iterable: PyIter,
        #[pyarg(any, optional)]
        key: OptionalOption<PyObjectRef>,
    }

    impl Constructor for PyItertoolsGroupBy {
        type Args = GroupByArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, key }: Self::Args,
            _vm: &VirtualMachine,
        ) -> PyResult<Self> {
            Ok(Self {
                iterable,
                key_func: key.flatten(),
                state: PyMutex::new(GroupByState::default()),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
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
            let mut state = zelf.state.lock();
            state.grouper = None;

            if !state.next_group {
                // FIXME: unnecessary clone. current_key always exist until assigning new
                let current_key = state.current_key.clone();
                drop(state);

                let (value, key) = if let Some(old_key) = current_key {
                    loop {
                        let (value, new_key) = raise_if_stop!(zelf.advance(vm)?);
                        if !vm.bool_eq(&new_key, &old_key)? {
                            break (value, new_key);
                        }
                    }
                } else {
                    raise_if_stop!(zelf.advance(vm)?)
                };

                state = zelf.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
            }

            state.next_group = false;

            let grouper = PyItertoolsGrouper {
                groupby: zelf.to_owned(),
            }
            .into_ref(&vm.ctx);

            state.grouper = Some(grouper.downgrade(None, vm).unwrap());
            Ok(PyIterReturn::Return(
                (state.current_key.as_ref().unwrap().clone(), grouper).to_pyobject(vm),
            ))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
    }

    #[pyclass(with(IterNext, Iterable), flags(HAS_WEAKREF))]
    impl PyItertoolsGrouper {}

    impl SelfIter for PyItertoolsGrouper {}

    impl IterNext for PyItertoolsGrouper {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let old_key = {
                let mut state = zelf.groupby.state.lock();

                if !state.is_current(zelf) {
                    return Ok(PyIterReturn::StopIteration(None));
                }

                // check to see if the value has already been retrieved from the iterator
                if let Some(val) = state.current_value.take() {
                    return Ok(PyIterReturn::Return(val));
                }

                state.current_key.as_ref().unwrap().clone()
            };
            let (value, key) = raise_if_stop!(zelf.groupby.advance(vm)?);
            if vm.bool_eq(&key, &old_key)? {
                Ok(PyIterReturn::Return(value))
            } else {
                let mut state = zelf.groupby.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
                state.next_group = true;
                state.grouper = None;
                Ok(PyIterReturn::StopIteration(None))
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "islice", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsIslice {
        iterable: PyIter,
        #[pytraverse(skip)]
        cur: AtomicCell<usize>,
        #[pytraverse(skip)]
        next: AtomicCell<usize>,
        #[pytraverse(skip)]
        stop: Option<usize>,
        #[pytraverse(skip)]
        step: usize,
    }

    // Restrict obj to ints with value 0 <= val <= sys.maxsize
    // On failure (out of range, non-int object) a ValueError is raised.
    fn pyobject_to_opt_usize(
        obj: PyObjectRef,
        name: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let is_int = obj.fast_isinstance(vm.ctx.types.int_type);
        if is_int {
            let value = int::get_value(&obj).to_usize();
            if let Some(value) = value {
                // Only succeeds for values for which 0 <= value <= sys.maxsize
                if value <= sys::MAXSIZE as usize {
                    return Ok(value);
                }
            }
        }
        // We don't have an int or value was < 0 or > sys.maxsize
        Err(vm.new_value_error(format!(
            "{name} argument for islice() must be None or an integer: 0 <= x <= sys.maxsize."
        )))
    }

    #[pyclass(with(IterNext, Iterable), flags(BASETYPE))]
    impl PyItertoolsIslice {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (iter, start, stop, step) = match args.args.len() {
                0 | 1 => {
                    return Err(vm.new_arity_type_error(Self::NAME, 2..=4, args.args.len()));
                }
                2 => {
                    let (iter, stop): (PyObjectRef, PyObjectRef) = args.bind_for(vm, Self::NAME)?;
                    (iter, 0usize, stop, 1usize)
                }
                _ => {
                    let (iter, start, stop, step) = if args.args.len() == 3 {
                        let (iter, start, stop): (PyObjectRef, PyObjectRef, PyObjectRef) =
                            args.bind_for(vm, Self::NAME)?;
                        (iter, start, stop, 1usize)
                    } else {
                        let (iter, start, stop, step): (
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                        ) = args.bind_for(vm, Self::NAME)?;

                        let step = if !vm.is_none(&step) {
                            pyobject_to_opt_usize(step, "Step", vm)?
                        } else {
                            1usize
                        };
                        (iter, start, stop, step)
                    };
                    let start = if !vm.is_none(&start) {
                        pyobject_to_opt_usize(start, "Start", vm)?
                    } else {
                        0usize
                    };

                    (iter, start, stop, step)
                }
            };

            let stop = if !vm.is_none(&stop) {
                Some(pyobject_to_opt_usize(stop, "Stop", vm)?)
            } else {
                None
            };

            let iter = iter.get_iter(vm)?;

            Self {
                iterable: iter,
                cur: AtomicCell::new(0),
                next: AtomicCell::new(start),
                stop,
                step,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl SelfIter for PyItertoolsIslice {}

    impl IterNext for PyItertoolsIslice {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            while zelf.cur.load() < zelf.next.load() {
                zelf.iterable.next(vm)?;
                zelf.cur.fetch_add(1);
            }

            if let Some(stop) = zelf.stop
                && zelf.cur.load() >= stop
            {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let obj = raise_if_stop!(zelf.iterable.next(vm)?);
            zelf.cur.fetch_add(1);

            // TODO is this overflow check required? attempts to copy CPython.
            let (next, ovf) = zelf.next.load().overflowing_add(zelf.step);
            zelf.next.store(if ovf { zelf.stop.unwrap() } else { next });

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
        predicate: PyObjectRef,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsFilterFalse {
        type Args = FilterFalseNewArgs;

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

                if !pred_value.try_to_bool(vm)? {
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
        initial: Option<PyObjectRef>,
        acc_value: PyRwLock<Option<PyObjectRef>>,
    }

    #[derive(FromArgs)]
    struct AccumulateArgs {
        iterable: PyIter,
        #[pyarg(any, optional)]
        func: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        initial: OptionalOption<PyObjectRef>,
    }

    impl Constructor for PyItertoolsAccumulate {
        type Args = AccumulateArgs;

        fn py_new(_cls: &Py<PyType>, args: AccumulateArgs, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                iterable: args.iterable,
                bin_op: args.func.flatten(),
                initial: args.initial.flatten(),
                acc_value: PyRwLock::new(None),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyItertoolsAccumulate {}

    impl SelfIter for PyItertoolsAccumulate {}

    impl IterNext for PyItertoolsAccumulate {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let iterable = &zelf.iterable;

            let acc_value = zelf.acc_value.read().clone();

            let next_acc_value = match acc_value {
                None => match &zelf.initial {
                    None => raise_if_stop!(iterable.next(vm)?),
                    Some(obj) => obj.clone(),
                },
                Some(value) => {
                    let obj = raise_if_stop!(iterable.next(vm)?);
                    match &zelf.bin_op {
                        None => vm._add(&value, &obj)?,
                        Some(op) => op.call((value, obj), vm)?,
                    }
                }
            };
            *zelf.acc_value.write() = Some(next_acc_value.clone());

            Ok(PyIterReturn::Return(next_acc_value))
        }
    }

    #[pyattr]
    #[pyclass(name = "_tee_dataobject", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsTeeData {
        iterable: PyIter,
        values: PyMutex<Vec<PyObjectRef>>,
        #[pytraverse(skip)]
        running: AtomicBool,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PyItertoolsTeeData {
        fn new(iterable: PyIter, vm: &VirtualMachine) -> PyRef<Self> {
            Self {
                iterable,
                values: PyMutex::new(vec![]),
                running: AtomicBool::new(false),
            }
            .into_ref(&vm.ctx)
        }

        fn get_item(&self, vm: &VirtualMachine, index: usize) -> PyResult<PyIterReturn> {
            // Return cached value if available
            {
                let Some(values) = self.values.try_lock() else {
                    return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
                };
                if index < values.len() {
                    return Ok(PyIterReturn::Return(values[index].clone()));
                }
            }
            // Prevent concurrent/reentrant calls to iterable.next(). The claim
            // covers caching the value as well: released any earlier, a second
            // tee at the same index fetches a value of its own and one of the
            // two is dropped without ever reaching a caller.
            if self.running.swap(true, Ordering::Acquire) {
                return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
            }
            scopeguard::defer! { self.running.store(false, Ordering::Release) }
            let obj = raise_if_stop!(self.iterable.next(vm)?);
            let Some(mut values) = self.values.try_lock() else {
                return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
            };
            if values.len() == index {
                values.push(obj);
            }
            Ok(PyIterReturn::Return(values[index].clone()))
        }
    }

    #[pyattr]
    #[pyclass(name = "_tee", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsTee {
        tee_data: PyRef<PyItertoolsTeeData>,
        #[pytraverse(skip)]
        index: AtomicCell<usize>,
        #[pytraverse(skip)]
        advancing: AtomicBool,
    }

    impl Constructor for PyItertoolsTee {
        type Args = PyIter;

        fn py_new(_cls: &Py<PyType>, iterator: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // An iterator that is already a tee shares its buffer rather than
            // getting one of its own.
            if let Some(tee) = iterator.as_object().downcast_ref::<Self>() {
                return Ok(tee.__copy__());
            }
            Ok(Self {
                tee_data: PyItertoolsTeeData::new(iterator, vm),
                index: AtomicCell::new(0),
                advancing: AtomicBool::new(false),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor), flags(HAS_WEAKREF))]
    impl PyItertoolsTee {
        fn from_iter(iterator: PyIter, vm: &VirtualMachine) -> PyResult {
            let class = Self::class(&vm.ctx);
            if iterator.class().is(class) {
                return vm.call_special_method(&iterator, identifier!(vm, __copy__), ());
            }
            Ok(Self {
                tee_data: PyItertoolsTeeData::new(iterator, vm),
                index: AtomicCell::new(0),
                advancing: AtomicBool::new(false),
            }
            .into_ref_with_type(vm, class.to_owned())?
            .into())
        }

        #[pymethod]
        fn __copy__(&self) -> Self {
            Self {
                tee_data: self.tee_data.clone(),
                index: AtomicCell::new(self.index.load()),
                advancing: AtomicBool::new(false),
            }
        }
    }

    #[pyfunction]
    fn tee(iterable: PyIter, n: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let n = n.unwrap_or(2);
        if n < 0 {
            return Err(vm.new_value_error("n must be >= 0"));
        }
        let n = n as usize;

        // Only an iterator that cannot copy itself needs a tee to buffer it.
        let copyable = if iterable.class().has_attr(identifier!(vm, __copy__)) {
            iterable.into()
        } else {
            PyItertoolsTee::from_iter(iterable, vm)?
        };

        let mut tee_vec: Vec<PyObjectRef> = Vec::new();
        tee_vec
            .try_reserve_exact(n)
            .map_err(|_| vm.new_memory_error(""))?;
        for _ in 0..n {
            tee_vec.push(vm.call_special_method(&copyable, identifier!(vm, __copy__), ())?);
        }

        Ok(PyTuple::new_ref(tee_vec, &vm.ctx))
    }
    impl SelfIter for PyItertoolsTee {}
    impl IterNext for PyItertoolsTee {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // Reading the index and moving it on is one step: two callers that
            // read the same index hand out the same value twice and leave the
            // buffer to be filled out of order.
            if zelf.advancing.swap(true, Ordering::Acquire) {
                return Err(vm.new_runtime_error("cannot re-enter the tee iterator"));
            }
            scopeguard::defer! { zelf.advancing.store(false, Ordering::Release) }
            let index = zelf.index.load();
            let value = raise_if_stop!(zelf.tee_data.get_item(vm, index)?);
            zelf.index.store(index + 1);
            Ok(PyIterReturn::Return(value))
        }
    }

    #[pyattr]
    #[pyclass(name = "product", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsProduct {
        pools: Vec<Vec<PyObjectRef>>,
        #[pytraverse(skip)]
        idxs: PyRwLock<Vec<usize>>,
        #[pytraverse(skip)]
        cur: AtomicCell<usize>,
        #[pytraverse(skip)]
        stop: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct ProductArgs {
        #[pyarg(named, optional)]
        repeat: OptionalArg<isize>,
    }

    impl Constructor for PyItertoolsProduct {
        type Args = (PosArgs<PyObjectRef>, ProductArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (iterables, args): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let repeat = args.repeat.unwrap_or(1);
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

            let mut single: Vec<Vec<PyObjectRef>> = Vec::new();
            for arg in iterables.iter() {
                single.push(arg.try_to_value(vm)?);
            }

            let mut pools: Vec<Vec<PyObjectRef>> = Vec::new();
            pools
                .try_reserve_exact(npools)
                .map_err(|_| vm.new_memory_error(""))?;
            // Filled by index, the way `product_new()` fills a tuple of
            // `npools`. Repeating the arguments `repeat` times instead walks
            // that many steps even when there are no arguments to repeat, so
            // `product(repeat=2**62)` would spin rather than answer `[()]`.
            pools.extend((0..npools).map(|i| single[i % single.len()].clone()));

            let mut idxs = Vec::new();
            idxs.try_reserve_exact(npools)
                .map_err(|_| vm.new_memory_error(""))?;
            idxs.resize(npools, 0);

            let l = pools.len();

            Ok(Self {
                pools,
                idxs: PyRwLock::new(idxs),
                cur: AtomicCell::new(l.wrapping_sub(1)),
                stop: AtomicCell::new(false),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyItertoolsProduct {
        fn update_idxs(&self, mut idxs: PyRwLockWriteGuard<'_, Vec<usize>>) {
            if idxs.is_empty() {
                self.stop.store(true);
                return;
            }

            let cur = self.cur.load();
            let lst_idx = &self.pools[cur].len() - 1;

            if idxs[cur] == lst_idx {
                if cur == 0 {
                    self.stop.store(true);
                    return;
                }
                idxs[cur] = 0;
                self.cur.fetch_sub(1);
                self.update_idxs(idxs);
            } else {
                idxs[cur] += 1;
                self.cur.store(idxs.len() - 1);
            }
        }
    }

    impl SelfIter for PyItertoolsProduct {}
    impl IterNext for PyItertoolsProduct {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            // stop signal
            if zelf.stop.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            let pools = &zelf.pools;

            for p in pools {
                if p.is_empty() {
                    return Ok(PyIterReturn::StopIteration(None));
                }
            }

            let idxs = zelf.idxs.write();
            let res = vm.ctx.new_tuple(
                pools
                    .iter()
                    .zip(idxs.iter())
                    .map(|(pool, idx)| pool[*idx].clone())
                    .collect(),
            );

            zelf.update_idxs(idxs);

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
        r: PyIntRef,
    }

    impl Constructor for PyItertoolsCombinations {
        type Args = CombinationsNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let pool: Vec<_> = iterable.try_to_value(vm)?;

            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative"));
            }
            let r = r.to_isize().ok_or_else(|| {
                vm.new_overflow_error("Python int too large to convert to C ssize_t")
            })? as usize;

            let n = pool.len();

            let mut indices = Vec::new();
            indices
                .try_reserve_exact(r)
                .map_err(|_| vm.new_memory_error(""))?;
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

    #[pyclass(with(IterNext, Iterable, Constructor))]
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
            let pool: Vec<_> = iterable.try_to_value(vm)?;
            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative"));
            }
            let r = r.to_isize().ok_or_else(|| {
                vm.new_overflow_error("Python int too large to convert to C ssize_t")
            })? as usize;

            let n = pool.len();

            let mut indices = Vec::new();
            indices
                .try_reserve_exact(r)
                .map_err(|_| vm.new_memory_error(""))?;
            indices.resize(r, 0);

            Ok(Self {
                pool,
                indices: PyRwLock::new(indices),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(n == 0 && r > 0),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
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
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional, optional)]
        r: OptionalOption<PyObjectRef>,
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
            let r = match r.flatten() {
                Some(r) => {
                    let val = r
                        .downcast_ref::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("Expected int as r"))?
                        .as_bigint();

                    if val.is_negative() {
                        return Err(vm.new_value_error("r must be non-negative"));
                    }
                    val.to_isize().ok_or_else(|| {
                        vm.new_overflow_error("Python int too large to convert to C ssize_t")
                    })? as usize
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

    #[pyclass(with(IterNext, Iterable, Constructor))]
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
        fillvalue: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyItertoolsZipLongest {
        type Args = (PosArgs<PyIter>, ZipLongestArgs);

        fn py_new(
            _cls: &Py<PyType>,
            (iterators, args): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let fillvalue = args.fillvalue.unwrap_or_none(vm);
            let iterators = iterators.into_vec();
            Ok(Self {
                iterators,
                fillvalue: PyRwLock::new(fillvalue),
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "zip_longest", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsZipLongest {
        iterators: Vec<PyIter>,
        fillvalue: PyRwLock<PyObjectRef>,
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyItertoolsZipLongest {}

    impl SelfIter for PyItertoolsZipLongest {}

    impl IterNext for PyItertoolsZipLongest {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.iterators.is_empty() {
                return Ok(PyIterReturn::StopIteration(None));
            }
            let mut result: Vec<PyObjectRef> = Vec::new();
            let mut num_active = zelf.iterators.len();

            for idx in 0..zelf.iterators.len() {
                let next_obj = match zelf.iterators[idx].next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => {
                        num_active -= 1;
                        if num_active == 0 {
                            return Ok(PyIterReturn::StopIteration(v));
                        }
                        zelf.fillvalue.read().clone()
                    }
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
        iterator: PyIter,
        old: PyRwLock<Option<PyObjectRef>>,
    }

    impl Constructor for PyItertoolsPairwise {
        type Args = PyIter;

        fn py_new(_cls: &Py<PyType>, iterator: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                iterator,
                old: PyRwLock::new(None),
            })
        }
    }

    #[pyclass(with(IterNext, Iterable, Constructor))]
    impl PyItertoolsPairwise {}

    impl SelfIter for PyItertoolsPairwise {}

    impl IterNext for PyItertoolsPairwise {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let old_clone = {
                let guard = zelf.old.read();
                guard.clone()
            };
            let old = match old_clone {
                None => match zelf.iterator.next(vm)? {
                    PyIterReturn::Return(obj) => {
                        // Needed for when we reenter
                        *zelf.old.write() = Some(obj.clone());
                        obj
                    }
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                },
                Some(obj) => obj,
            };

            let new = raise_if_stop!(zelf.iterator.next(vm)?);
            *zelf.old.write() = Some(new.clone());

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
        #[pyarg(positional)]
        iterable_ref: PyObjectRef,
        #[pyarg(positional)]
        n: PyIntRef,
        #[pyarg(named, default = false)]
        strict: bool,
    }

    impl Constructor for PyItertoolsBatched {
        type Args = BatchedNewArgs;

        fn py_new(
            _cls: &Py<PyType>,
            Self::Args {
                iterable_ref,
                n,
                strict,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let n = n.as_bigint();
            if n.lt(&BigInt::one()) {
                return Err(vm.new_value_error("n must be at least one"));
            }
            let n = n
                .to_usize()
                .ok_or_else(|| vm.new_overflow_error("Python int too large to convert to usize"))?;
            let iterable = iterable_ref.get_iter(vm)?;

            Ok(Self {
                iterable,
                n: AtomicCell::new(n),
                exhausted: AtomicCell::new(false),
                strict: AtomicCell::new(strict),
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
                match zelf.iterable.next(vm)? {
                    PyIterReturn::Return(obj) => {
                        result.push(obj);
                    }
                    PyIterReturn::StopIteration(_) => {
                        zelf.exhausted.store(true);
                        break;
                    }
                }
            }
            let res_len = result.len();
            match res_len {
                0 => Ok(PyIterReturn::StopIteration(None)),
                _ => {
                    if zelf.strict.load() && res_len != n {
                        Err(vm.new_value_error("batched(): incomplete batch"))
                    } else {
                        Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
                    }
                }
            }
        }
    }
}
