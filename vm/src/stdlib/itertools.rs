pub(crate) use decl::make_module;

#[pymodule(name = "itertools")]
mod decl {
    use crate::common::{
        lock::{PyMutex, PyRwLock, PyRwLockWriteGuard},
        rc::PyRc,
    };
    use crate::{
        builtins::{int, PyGenericAlias, PyInt, PyIntRef, PyTuple, PyTupleRef, PyTypeRef},
        function::{ArgCallable, FuncArgs, IntoPyObject, OptionalArg, OptionalOption, PosArgs},
        protocol::{PyIter, PyIterReturn},
        stdlib::sys,
        types::{Constructor, IterNext, IterNextIterable},
        IdProtocol, PyObjectRef, PyObjectView, PyRef, PyResult, PyValue, PyWeakRef, TypeProtocol,
        VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use num_bigint::BigInt;
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use std::fmt;

    #[pyattr]
    #[pyclass(name = "chain")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsChain {
        iterables: Vec<PyObjectRef>,
        cur_idx: AtomicCell<usize>,
        cached_iter: PyRwLock<Option<PyIter>>,
    }

    #[pyimpl(with(IterNext))]
    impl PyItertoolsChain {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            PyItertoolsChain {
                iterables: args.args,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }

        #[pyclassmethod]
        fn from_iterable(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            PyItertoolsChain {
                iterables: vm.extract_elements(&iterable)?,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }
    }
    impl IterNextIterable for PyItertoolsChain {}
    impl IterNext for PyItertoolsChain {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            loop {
                let pos = zelf.cur_idx.load();
                if pos >= zelf.iterables.len() {
                    break;
                }
                let cur_iter = if zelf.cached_iter.read().is_none() {
                    // We need to call "get_iter" outside of the lock.
                    let iter = zelf.iterables[pos].clone().get_iter(vm)?;
                    *zelf.cached_iter.write() = Some(iter.clone());
                    iter
                } else if let Some(cached_iter) = (*zelf.cached_iter.read()).clone() {
                    cached_iter
                } else {
                    // Someone changed cached iter to None since we checked.
                    continue;
                };

                // We need to call "next" outside of the lock.
                match cur_iter.next(vm) {
                    Ok(PyIterReturn::Return(ok)) => return Ok(PyIterReturn::Return(ok)),
                    Ok(PyIterReturn::StopIteration(_)) => {
                        zelf.cur_idx.fetch_add(1);
                        *zelf.cached_iter.write() = None;
                    }
                    Err(err) => {
                        return Err(err);
                    }
                }
            }

            Ok(PyIterReturn::StopIteration(None))
        }
    }

    #[pyattr]
    #[pyclass(name = "compress")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCompress {
        data: PyIter,
        selector: PyIter,
    }

    #[derive(FromArgs)]
    struct CompressNewArgs {
        #[pyarg(positional)]
        data: PyIter,
        #[pyarg(positional)]
        selector: PyIter,
    }

    impl Constructor for PyItertoolsCompress {
        type Args = CompressNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { data, selector }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsCompress { data, selector }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsCompress {}

    impl IterNextIterable for PyItertoolsCompress {}
    impl IterNext for PyItertoolsCompress {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            loop {
                let sel_obj = match zelf.selector.next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                };
                let verdict = sel_obj.clone().try_to_bool(vm)?;
                let data_obj = zelf.data.next(vm)?;

                if verdict {
                    return Ok(data_obj);
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "count")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCount {
        cur: PyRwLock<BigInt>,
        step: BigInt,
    }

    #[derive(FromArgs)]
    struct CountNewArgs {
        #[pyarg(positional, optional)]
        start: OptionalArg<PyIntRef>,

        #[pyarg(positional, optional)]
        step: OptionalArg<PyIntRef>,
    }

    impl Constructor for PyItertoolsCount {
        type Args = CountNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { start, step }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let start = match start.into_option() {
                Some(int) => int.as_bigint().clone(),
                None => BigInt::zero(),
            };
            let step = match step.into_option() {
                Some(int) => int.as_bigint().clone(),
                None => BigInt::one(),
            };

            PyItertoolsCount {
                cur: PyRwLock::new(start),
                step,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsCount {}
    impl IterNextIterable for PyItertoolsCount {}
    impl IterNext for PyItertoolsCount {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut cur = zelf.cur.write();
            let result = cur.clone();
            *cur += &zelf.step;
            Ok(PyIterReturn::Return(result.into_pyobject(vm)))
        }
    }

    #[pyattr]
    #[pyclass(name = "cycle")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCycle {
        iter: PyIter,
        saved: PyRwLock<Vec<PyObjectRef>>,
        index: AtomicCell<usize>,
    }

    impl Constructor for PyItertoolsCycle {
        type Args = PyIter;

        fn py_new(cls: PyTypeRef, iter: Self::Args, vm: &VirtualMachine) -> PyResult {
            Self {
                iter,
                saved: PyRwLock::new(Vec::new()),
                index: AtomicCell::new(0),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsCycle {}
    impl IterNextIterable for PyItertoolsCycle {}
    impl IterNext for PyItertoolsCycle {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let item = if let PyIterReturn::Return(item) = zelf.iter.next(vm)? {
                zelf.saved.write().push(item.clone());
                item
            } else {
                let saved = zelf.saved.read();
                if saved.len() == 0 {
                    return Ok(PyIterReturn::StopIteration(None));
                }

                let last_index = zelf.index.fetch_add(1);

                if last_index >= saved.len() - 1 {
                    zelf.index.store(0);
                }

                saved[last_index].clone()
            };

            Ok(PyIterReturn::Return(item))
        }
    }

    #[pyattr]
    #[pyclass(name = "repeat")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsRepeat {
        object: PyObjectRef,
        times: Option<PyRwLock<usize>>,
    }

    #[derive(FromArgs)]
    struct PyRepeatNewArgs {
        object: PyObjectRef,
        #[pyarg(any, optional)]
        times: OptionalArg<PyIntRef>,
    }

    impl Constructor for PyItertoolsRepeat {
        type Args = PyRepeatNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { object, times }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let times = match times.into_option() {
                Some(int) => {
                    let val: isize = int.try_to_primitive(vm)?;
                    // times always >= 0.
                    Some(PyRwLock::new(val.to_usize().unwrap_or(0)))
                }
                None => None,
            };
            PyItertoolsRepeat { object, times }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsRepeat {
        #[pymethod(magic)]
        fn length_hint(&self, vm: &VirtualMachine) -> PyResult<usize> {
            // Return TypeError, length_hint picks this up and returns the default.
            let times = self
                .times
                .as_ref()
                .ok_or_else(|| vm.new_type_error("length of unsized object.".to_owned()))?;
            Ok(*times.read())
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let cls = zelf.clone_class().into_pyobject(vm);
            Ok(match zelf.times {
                Some(ref times) => vm.new_tuple((cls, (zelf.object.clone(), *times.read()))),
                None => vm.new_tuple((cls, (zelf.object.clone(),))),
            })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let mut fmt = format!("{}", &self.object.repr(vm)?);
            if let Some(ref times) = self.times {
                fmt.push_str(&format!(", {}", times.read()));
            }
            Ok(format!("repeat({})", fmt))
        }
    }

    impl IterNextIterable for PyItertoolsRepeat {}
    impl IterNext for PyItertoolsRepeat {
        fn next(zelf: &PyObjectView<Self>, _vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

    #[pyattr]
    #[pyclass(name = "starmap")]
    #[derive(Debug, PyValue)]
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
            cls: PyTypeRef,
            Self::Args { function, iterable }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsStarmap { function, iterable }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsStarmap {}
    impl IterNextIterable for PyItertoolsStarmap {}
    impl IterNext for PyItertoolsStarmap {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let obj = zelf.iterable.next(vm)?;
            let function = &zelf.function;
            match obj {
                PyIterReturn::Return(obj) => {
                    PyIterReturn::from_pyresult(vm.invoke(function, vm.extract_elements(&obj)?), vm)
                }
                PyIterReturn::StopIteration(v) => Ok(PyIterReturn::StopIteration(v)),
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "takewhile")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsTakewhile {
        predicate: PyObjectRef,
        iterable: PyIter,
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
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsTakewhile {
                predicate,
                iterable,
                stop_flag: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsTakewhile {}
    impl IterNextIterable for PyItertoolsTakewhile {}
    impl IterNext for PyItertoolsTakewhile {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.stop_flag.load() {
                return Ok(PyIterReturn::StopIteration(None));
            }

            // might be StopIteration or anything else, which is propagated upwards
            let obj = match zelf.iterable.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            let predicate = &zelf.predicate;

            let verdict = vm.invoke(predicate, (obj.clone(),))?;
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
    #[pyclass(name = "dropwhile")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsDropwhile {
        predicate: ArgCallable,
        iterable: PyIter,
        start_flag: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct DropwhileNewArgs {
        #[pyarg(positional)]
        predicate: ArgCallable,
        #[pyarg(positional)]
        iterable: PyIter,
    }

    impl Constructor for PyItertoolsDropwhile {
        type Args = DropwhileNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsDropwhile {
                predicate,
                iterable,
                start_flag: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsDropwhile {}
    impl IterNextIterable for PyItertoolsDropwhile {}
    impl IterNext for PyItertoolsDropwhile {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            if !zelf.start_flag.load() {
                loop {
                    let obj = match iterable.next(vm)? {
                        PyIterReturn::Return(obj) => obj,
                        PyIterReturn::StopIteration(v) => {
                            return Ok(PyIterReturn::StopIteration(v))
                        }
                    };
                    let pred = predicate.clone();
                    let pred_value = vm.invoke(&pred, (obj.clone(),))?;
                    if !pred_value.try_to_bool(vm)? {
                        zelf.start_flag.store(true);
                        return Ok(PyIterReturn::Return(obj));
                    }
                }
            }
            iterable.next(vm)
        }
    }

    struct GroupByState {
        current_value: Option<PyObjectRef>,
        current_key: Option<PyObjectRef>,
        next_group: bool,
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
        fn is_current(&self, grouper: &PyObjectView<PyItertoolsGrouper>) -> bool {
            self.grouper
                .as_ref()
                .and_then(|g| g.upgrade())
                .map_or(false, |ref current_grouper| grouper.is(current_grouper))
        }
    }

    #[pyattr]
    #[pyclass(name = "groupby")]
    #[derive(PyValue)]
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
            cls: PyTypeRef,
            Self::Args { iterable, key }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsGroupBy {
                iterable,
                key_func: key.flatten(),
                state: PyMutex::new(GroupByState {
                    current_key: None,
                    current_value: None,
                    next_group: false,
                    grouper: None,
                }),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsGroupBy {
        pub(super) fn advance(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<PyIterReturn<(PyObjectRef, PyObjectRef)>> {
            let new_value = match self.iterable.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            let new_key = if let Some(ref kf) = self.key_func {
                vm.invoke(kf, (new_value.clone(),))?
            } else {
                new_value.clone()
            };
            Ok(PyIterReturn::Return((new_value, new_key)))
        }
    }
    impl IterNextIterable for PyItertoolsGroupBy {}
    impl IterNext for PyItertoolsGroupBy {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut state = zelf.state.lock();
            state.grouper = None;

            if !state.next_group {
                // FIXME: unnecessary clone. current_key always exist until assigning new
                let current_key = state.current_key.clone();
                drop(state);

                let (value, key) = if let Some(old_key) = current_key {
                    loop {
                        let (value, new_key) = match zelf.advance(vm)? {
                            PyIterReturn::Return(obj) => obj,
                            PyIterReturn::StopIteration(v) => {
                                return Ok(PyIterReturn::StopIteration(v))
                            }
                        };
                        if !vm.bool_eq(&new_key, &old_key)? {
                            break (value, new_key);
                        }
                    }
                } else {
                    match zelf.advance(vm)? {
                        PyIterReturn::Return(obj) => obj,
                        PyIterReturn::StopIteration(v) => {
                            return Ok(PyIterReturn::StopIteration(v))
                        }
                    }
                };

                state = zelf.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
            }

            state.next_group = false;

            let grouper = PyItertoolsGrouper {
                groupby: zelf.to_owned(),
            }
            .into_ref(vm);

            state.grouper = Some(grouper.downgrade(None, vm).unwrap());
            Ok(PyIterReturn::Return(
                (state.current_key.as_ref().unwrap().clone(), grouper).into_pyobject(vm),
            ))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
    }

    #[pyimpl(with(IterNext))]
    impl PyItertoolsGrouper {}
    impl IterNextIterable for PyItertoolsGrouper {}
    impl IterNext for PyItertoolsGrouper {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
            let (value, key) = match zelf.groupby.advance(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
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
    #[pyclass(name = "islice")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsIslice {
        iterable: PyIter,
        cur: AtomicCell<usize>,
        next: AtomicCell<usize>,
        stop: Option<usize>,
        step: usize,
    }

    // Restrict obj to ints with value 0 <= val <= sys.maxsize
    // On failure (out of range, non-int object) a ValueError is raised.
    fn pyobject_to_opt_usize(
        obj: PyObjectRef,
        name: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let is_int = obj.isinstance(&vm.ctx.types.int_type);
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
        return Err(vm.new_value_error(format!(
            "{} argument for islice() must be None or an integer: 0 <= x <= sys.maxsize.",
            name
        )));
    }

    #[pyimpl(with(IterNext))]
    impl PyItertoolsIslice {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let (iter, start, stop, step) = match args.args.len() {
                0 | 1 => {
                    return Err(vm.new_type_error(format!(
                        "islice expected at least 2 arguments, got {}",
                        args.args.len()
                    )));
                }
                2 => {
                    let (iter, stop): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                    (iter, 0usize, stop, 1usize)
                }
                _ => {
                    let (iter, start, stop, step) = if args.args.len() == 3 {
                        let (iter, start, stop): (PyObjectRef, PyObjectRef, PyObjectRef) =
                            args.bind(vm)?;
                        (iter, start, stop, 1usize)
                    } else {
                        let (iter, start, stop, step): (
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                            PyObjectRef,
                        ) = args.bind(vm)?;

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

            PyItertoolsIslice {
                iterable: iter,
                cur: AtomicCell::new(0),
                next: AtomicCell::new(start),
                stop,
                step,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    impl IterNextIterable for PyItertoolsIslice {}
    impl IterNext for PyItertoolsIslice {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            while zelf.cur.load() < zelf.next.load() {
                zelf.iterable.next(vm)?;
                zelf.cur.fetch_add(1);
            }

            if let Some(stop) = zelf.stop {
                if zelf.cur.load() >= stop {
                    return Ok(PyIterReturn::StopIteration(None));
                }
            }

            let obj = match zelf.iterable.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            zelf.cur.fetch_add(1);

            // TODO is this overflow check required? attempts to copy CPython.
            let (next, ovf) = zelf.next.load().overflowing_add(zelf.step);
            zelf.next.store(if ovf { zelf.stop.unwrap() } else { next });

            Ok(PyIterReturn::Return(obj))
        }
    }

    #[pyattr]
    #[pyclass(name = "filterfalse")]
    #[derive(Debug, PyValue)]
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
            cls: PyTypeRef,
            Self::Args {
                predicate,
                iterable,
            }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsFilterFalse {
                predicate,
                iterable,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsFilterFalse {}
    impl IterNextIterable for PyItertoolsFilterFalse {}
    impl IterNext for PyItertoolsFilterFalse {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let predicate = &zelf.predicate;
            let iterable = &zelf.iterable;

            loop {
                let obj = match iterable.next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                };
                let pred_value = if vm.is_none(predicate) {
                    obj.clone()
                } else {
                    vm.invoke(predicate, (obj.clone(),))?
                };

                if !pred_value.try_to_bool(vm)? {
                    return Ok(PyIterReturn::Return(obj));
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "accumulate")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsAccumulate {
        iterable: PyIter,
        binop: Option<PyObjectRef>,
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

        fn py_new(cls: PyTypeRef, args: AccumulateArgs, vm: &VirtualMachine) -> PyResult {
            PyItertoolsAccumulate {
                iterable: args.iterable,
                binop: args.func.flatten(),
                initial: args.initial.flatten(),
                acc_value: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsAccumulate {}

    impl IterNextIterable for PyItertoolsAccumulate {}
    impl IterNext for PyItertoolsAccumulate {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let iterable = &zelf.iterable;

            let acc_value = zelf.acc_value.read().clone();

            let next_acc_value = match acc_value {
                None => match &zelf.initial {
                    None => match iterable.next(vm)? {
                        PyIterReturn::Return(obj) => obj,
                        PyIterReturn::StopIteration(v) => {
                            return Ok(PyIterReturn::StopIteration(v))
                        }
                    },
                    Some(obj) => obj.clone(),
                },
                Some(value) => {
                    let obj = match iterable.next(vm)? {
                        PyIterReturn::Return(obj) => obj,
                        PyIterReturn::StopIteration(v) => {
                            return Ok(PyIterReturn::StopIteration(v))
                        }
                    };
                    match &zelf.binop {
                        None => vm._add(&value, &obj)?,
                        Some(op) => vm.invoke(op, (value, obj))?,
                    }
                }
            };
            *zelf.acc_value.write() = Some(next_acc_value.clone());

            Ok(PyIterReturn::Return(next_acc_value))
        }
    }

    #[derive(Debug)]
    struct PyItertoolsTeeData {
        iterable: PyIter,
        values: PyRwLock<Vec<PyObjectRef>>,
    }

    impl PyItertoolsTeeData {
        fn new(iterable: PyIter, _vm: &VirtualMachine) -> PyResult<PyRc<PyItertoolsTeeData>> {
            Ok(PyRc::new(PyItertoolsTeeData {
                iterable,
                values: PyRwLock::new(vec![]),
            }))
        }

        fn get_item(&self, vm: &VirtualMachine, index: usize) -> PyResult<PyIterReturn> {
            if self.values.read().len() == index {
                let result = match self.iterable.next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                };
                self.values.write().push(result);
            }
            Ok(PyIterReturn::Return(self.values.read()[index].clone()))
        }
    }

    #[pyattr]
    #[pyclass(name = "tee")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsTee {
        tee_data: PyRc<PyItertoolsTeeData>,
        index: AtomicCell<usize>,
    }

    #[derive(FromArgs)]
    struct TeeNewArgs {
        #[pyarg(positional)]
        iterable: PyIter,
        #[pyarg(positional, optional)]
        n: OptionalArg<usize>,
    }

    impl Constructor for PyItertoolsTee {
        type Args = TeeNewArgs;

        // TODO: make tee() a function, rename this class to itertools._tee and make
        // teedata a python class
        #[allow(clippy::new_ret_no_self)]
        fn py_new(
            _cls: PyTypeRef,
            Self::Args { iterable, n }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let n = n.unwrap_or(2);

            let copyable = if iterable.class().has_attr("__copy__") {
                vm.call_method(&iterable, "__copy__", ())?
            } else {
                PyItertoolsTee::from_iter(iterable, vm)?
            };

            let mut tee_vec: Vec<PyObjectRef> = Vec::with_capacity(n);
            for _ in 0..n {
                tee_vec.push(vm.call_method(&copyable, "__copy__", ())?);
            }

            Ok(PyTuple::new_ref(tee_vec, &vm.ctx).into())
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsTee {
        fn from_iter(iterator: PyIter, vm: &VirtualMachine) -> PyResult {
            let class = PyItertoolsTee::class(vm);
            if iterator.class().is(PyItertoolsTee::class(vm)) {
                return vm.call_method(&iterator, "__copy__", ());
            }
            Ok(PyItertoolsTee {
                tee_data: PyItertoolsTeeData::new(iterator, vm)?,
                index: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, class.clone())?
            .into())
        }

        #[pymethod(magic)]
        fn copy(&self) -> Self {
            Self {
                tee_data: PyRc::clone(&self.tee_data),
                index: AtomicCell::new(self.index.load()),
            }
        }
    }
    impl IterNextIterable for PyItertoolsTee {}
    impl IterNext for PyItertoolsTee {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let value = match zelf.tee_data.get_item(vm, zelf.index.load())? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            zelf.index.fetch_add(1);
            Ok(PyIterReturn::Return(value))
        }
    }

    #[pyattr]
    #[pyclass(name = "product")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsProduct {
        pools: Vec<Vec<PyObjectRef>>,
        idxs: PyRwLock<Vec<usize>>,
        cur: AtomicCell<usize>,
        stop: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct ProductArgs {
        #[pyarg(named, optional)]
        repeat: OptionalArg<usize>,
    }

    impl Constructor for PyItertoolsProduct {
        type Args = (PosArgs<PyObjectRef>, ProductArgs);

        fn py_new(cls: PyTypeRef, (iterables, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            let repeat = args.repeat.unwrap_or(1);
            let mut pools = Vec::new();
            for arg in iterables.iter() {
                pools.push(vm.extract_elements(arg)?);
            }
            let pools = std::iter::repeat(pools)
                .take(repeat)
                .flatten()
                .collect::<Vec<Vec<PyObjectRef>>>();

            let l = pools.len();

            PyItertoolsProduct {
                pools,
                idxs: PyRwLock::new(vec![0; l]),
                cur: AtomicCell::new(l.wrapping_sub(1)),
                stop: AtomicCell::new(false),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsProduct {
        fn update_idxs(&self, mut idxs: PyRwLockWriteGuard<'_, Vec<usize>>) {
            if idxs.len() == 0 {
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
    impl IterNextIterable for PyItertoolsProduct {}
    impl IterNext for PyItertoolsProduct {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[pyclass(name = "combinations")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCombinations {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    #[derive(FromArgs)]
    struct CombinationsNewArgs {
        #[pyarg(positional)]
        iterable: PyObjectRef,
        #[pyarg(positional)]
        r: PyIntRef,
    }

    impl Constructor for PyItertoolsCombinations {
        type Args = CombinationsNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;

            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative".to_owned()));
            }
            let r = r.to_usize().unwrap();

            let n = pool.len();

            PyItertoolsCombinations {
                pool,
                indices: PyRwLock::new((0..r).collect()),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsCombinations {}
    impl IterNextIterable for PyItertoolsCombinations {}
    impl IterNext for PyItertoolsCombinations {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

            let res = vm.ctx.new_tuple(
                zelf.indices
                    .read()
                    .iter()
                    .map(|&i| zelf.pool[i].clone())
                    .collect(),
            );

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
            } else {
                // Increment the current index which we know is not at its
                // maximum.  Then move back to the right setting each index
                // to its lowest possible value (one higher than the index
                // to its left -- this maintains the sort order invariant).
                indices[idx as usize] += 1;
                for j in idx as usize + 1..r {
                    indices[j] = indices[j - 1] + 1;
                }
            }

            Ok(PyIterReturn::Return(res.into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations_with_replacement")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsCombinationsWithReplacement {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    impl Constructor for PyItertoolsCombinationsWithReplacement {
        type Args = CombinationsNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;
            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative".to_owned()));
            }
            let r = r.to_usize().unwrap();

            let n = pool.len();

            PyItertoolsCombinationsWithReplacement {
                pool,
                indices: PyRwLock::new(vec![0; r]),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(n == 0 && r > 0),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsCombinationsWithReplacement {}

    impl IterNextIterable for PyItertoolsCombinationsWithReplacement {}
    impl IterNext for PyItertoolsCombinationsWithReplacement {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
                    indices[j as usize] = index as usize;
                }
            }

            Ok(PyIterReturn::Return(res.into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "permutations")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsPermutations {
        pool: Vec<PyObjectRef>,               // Collected input iterable
        indices: PyRwLock<Vec<usize>>,        // One index per element in pool
        cycles: PyRwLock<Vec<usize>>,         // One rollover counter per element in the result
        result: PyRwLock<Option<Vec<usize>>>, // Indexes of the most recently returned result
        r: AtomicCell<usize>,                 // Size of result tuple
        exhausted: AtomicCell<bool>,          // Set when the iterator is exhausted
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
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool = vm.extract_elements(&iterable)?;

            let n = pool.len();
            // If r is not provided, r == n. If provided, r must be a positive integer, or None.
            // If None, it behaves the same as if it was not provided.
            let r = match r.flatten() {
                Some(r) => {
                    let val = r
                        .payload::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("Expected int as r".to_owned()))?
                        .as_bigint();

                    if val.is_negative() {
                        return Err(vm.new_value_error("r must be non-negative".to_owned()));
                    }
                    val.to_usize().unwrap()
                }
                None => n,
            };

            PyItertoolsPermutations {
                pool,
                indices: PyRwLock::new((0..n).collect()),
                cycles: PyRwLock::new((0..r.min(n)).map(|i| n - i).collect()),
                result: PyRwLock::new(None),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsPermutations {}
    impl IterNextIterable for PyItertoolsPermutations {}
    impl IterNext for PyItertoolsPermutations {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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

        fn py_new(cls: PyTypeRef, (iterators, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            let fillvalue = args.fillvalue.unwrap_or_none(vm);
            let iterators = iterators.into_vec();
            PyItertoolsZipLongest {
                iterators,
                fillvalue,
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyattr]
    #[pyclass(name = "zip_longest")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsZipLongest {
        iterators: Vec<PyIter>,
        fillvalue: PyObjectRef,
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsZipLongest {}
    impl IterNextIterable for PyItertoolsZipLongest {}
    impl IterNext for PyItertoolsZipLongest {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            if zelf.iterators.is_empty() {
                return Ok(PyIterReturn::StopIteration(None));
            }
            let mut result: Vec<PyObjectRef> = Vec::new();
            let mut numactive = zelf.iterators.len();

            for idx in 0..zelf.iterators.len() {
                let next_obj = match zelf.iterators[idx].next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => {
                        numactive -= 1;
                        if numactive == 0 {
                            return Ok(PyIterReturn::StopIteration(v));
                        }
                        zelf.fillvalue.clone()
                    }
                };
                result.push(next_obj);
            }
            Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "pairwise")]
    #[derive(Debug, PyValue)]
    struct PyItertoolsPairwise {
        iterator: PyIter,
        old: PyRwLock<Option<PyObjectRef>>,
    }

    impl Constructor for PyItertoolsPairwise {
        type Args = PyIter;

        fn py_new(cls: PyTypeRef, iterator: Self::Args, vm: &VirtualMachine) -> PyResult {
            PyItertoolsPairwise {
                iterator,
                old: PyRwLock::new(None),
            }
            .into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(IterNext, Constructor))]
    impl PyItertoolsPairwise {}
    impl IterNextIterable for PyItertoolsPairwise {}
    impl IterNext for PyItertoolsPairwise {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let old = match zelf.old.read().clone() {
                None => match zelf.iterator.next(vm)? {
                    PyIterReturn::Return(obj) => obj,
                    PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
                },
                Some(obj) => obj,
            };
            let new = match zelf.iterator.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            *zelf.old.write() = Some(new.clone());
            Ok(PyIterReturn::Return(vm.new_tuple((old, new)).into()))
        }
    }
}
