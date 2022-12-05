pub(crate) use decl::make_module;

#[pymodule(name = "itertools")]
mod decl {
    use crate::{
        builtins::{
            int, tuple::IntoPyTuple, PyGenericAlias, PyInt, PyIntRef, PyList, PyTuple, PyTupleRef,
            PyTypeRef,
        },
        common::{
            lock::{PyMutex, PyRwLock, PyRwLockWriteGuard},
            rc::PyRc,
        },
        convert::ToPyObject,
        function::{ArgCallable, ArgIntoBool, FuncArgs, OptionalArg, OptionalOption, PosArgs},
        identifier,
        protocol::{PyIter, PyIterReturn, PyNumber},
        stdlib::sys,
        types::{Constructor, IterNext, IterNextIterable},
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, PyWeakRef, TryFromObject,
        VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::{Signed, ToPrimitive};
    use std::fmt;

    #[pyattr]
    #[pyclass(name = "chain")]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsChain {
        source: PyRwLock<Option<PyIter>>,
        active: PyRwLock<Option<PyIter>>,
    }

    #[pyclass(with(IterNext), flags(BASETYPE, HAS_DICT))]
    impl PyItertoolsChain {
        #[pyslot]
        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let args_list = PyList::from(args.args);
            PyItertoolsChain {
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
            PyItertoolsChain {
                source: PyRwLock::new(Some(source.get_iter(vm)?)),
                active: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let source = zelf.source.read().clone();
            let active = zelf.active.read().clone();
            let cls = zelf.class().to_owned();
            let empty_tuple = vm.ctx.empty_tuple.clone();
            let reduced = match source {
                Some(source) => match active {
                    Some(active) => vm.new_tuple((cls, empty_tuple, (source, active))),
                    None => vm.new_tuple((cls, empty_tuple, (source,))),
                },
                None => vm.new_tuple((cls, empty_tuple)),
            };
            Ok(reduced)
        }

        #[pymethod(magic)]
        fn setstate(zelf: PyRef<Self>, state: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
            let args = state.as_slice();
            if args.is_empty() {
                let msg = String::from("function takes at leat 1 arguments (0 given)");
                return Err(vm.new_type_error(msg));
            }
            if args.len() > 2 {
                let msg = format!("function takes at most 2 arguments ({} given)", args.len());
                return Err(vm.new_type_error(msg));
            }
            let source = &args[0];
            if args.len() == 1 {
                if !PyIter::check(source.as_ref()) {
                    return Err(vm.new_type_error(String::from("Arguments must be iterators.")));
                }
                *zelf.source.write() = source.to_owned().try_into_value(vm)?;
                return Ok(());
            }
            let active = &args[1];

            if !PyIter::check(source.as_ref()) || !PyIter::check(active.as_ref()) {
                return Err(vm.new_type_error(String::from("Arguments must be iterators.")));
            }
            let mut source_lock = zelf.source.write();
            let mut active_lock = zelf.active.write();
            *source_lock = source.to_owned().try_into_value(vm)?;
            *active_lock = active.to_owned().try_into_value(vm)?;
            Ok(())
        }
    }
    impl IterNextIterable for PyItertoolsChain {}
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
            match next {
                Err(_) | Ok(PyIterReturn::StopIteration(_)) => {
                    *zelf.source.write() = None;
                }
                _ => {}
            };
            next
        }
    }

    #[pyattr]
    #[pyclass(name = "compress")]
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
            cls: PyTypeRef,
            Self::Args { data, selectors }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsCompress { data, selectors }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsCompress {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyIter, PyIter)) {
            (
                zelf.class().to_owned(),
                (zelf.data.clone(), zelf.selectors.clone()),
            )
        }
    }

    impl IterNextIterable for PyItertoolsCompress {}
    impl IterNext for PyItertoolsCompress {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            loop {
                let sel_obj = match zelf.selectors.next(vm)? {
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
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCount {
        cur: PyRwLock<PyObjectRef>,
        step: PyObjectRef,
    }

    #[derive(FromArgs)]
    struct CountNewArgs {
        #[pyarg(positional, optional)]
        start: OptionalArg<PyObjectRef>,

        #[pyarg(positional, optional)]
        step: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyItertoolsCount {
        type Args = CountNewArgs;

        fn py_new(
            cls: PyTypeRef,
            Self::Args { start, step }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let start = start.into_option().unwrap_or_else(|| vm.new_pyobj(0));
            let step = step.into_option().unwrap_or_else(|| vm.new_pyobj(1));
            if !PyNumber::check(&start) || !PyNumber::check(&step) {
                return Err(vm.new_type_error("a number is required".to_owned()));
            }

            PyItertoolsCount {
                cur: PyRwLock::new(start),
                step,
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsCount {
        // TODO: Implement this
        // if (lz->cnt == PY_SSIZE_T_MAX)
        //      return Py_BuildValue("0(00)", Py_TYPE(lz), lz->long_cnt, lz->long_step);
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyObjectRef,)) {
            (zelf.class().to_owned(), (zelf.cur.read().clone(),))
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let cur = format!("{}", self.cur.read().clone().repr(vm)?);
            let step = &self.step;
            if vm.bool_eq(step, vm.ctx.new_int(1).as_object())? {
                return Ok(format!("count({cur})"));
            }
            Ok(format!("count({}, {})", cur, step.repr(vm)?))
        }
    }
    impl IterNextIterable for PyItertoolsCount {}
    impl IterNext for PyItertoolsCount {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let mut cur = zelf.cur.write();
            let step = zelf.step.clone();
            let result = cur.clone();
            *cur = vm._iadd(&cur, step.as_object())?;
            Ok(PyIterReturn::Return(result.to_pyobject(vm)))
        }
    }

    #[pyattr]
    #[pyclass(name = "cycle")]
    #[derive(Debug, PyPayload)]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsCycle {}
    impl IterNextIterable for PyItertoolsCycle {}
    impl IterNext for PyItertoolsCycle {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[derive(Debug, PyPayload)]
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
            PyItertoolsRepeat { object, times }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
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
            let cls = zelf.class().to_owned();
            Ok(match zelf.times {
                Some(ref times) => vm.new_tuple((cls, (zelf.object.clone(), *times.read()))),
                None => vm.new_tuple((cls, (zelf.object.clone(),))),
            })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let mut fmt = format!("{}", &self.object.repr(vm)?);
            if let Some(ref times) = self.times {
                fmt.push_str(", ");
                fmt.push_str(&times.read().to_string());
            }
            Ok(format!("repeat({fmt})"))
        }
    }

    impl IterNextIterable for PyItertoolsRepeat {}
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

    #[pyattr]
    #[pyclass(name = "starmap")]
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
            cls: PyTypeRef,
            Self::Args { function, iterable }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            PyItertoolsStarmap { function, iterable }
                .into_ref_with_type(vm, cls)
                .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsStarmap {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyObjectRef, PyIter)) {
            (
                zelf.class().to_owned(),
                (zelf.function.clone(), zelf.iterable.clone()),
            )
        }
    }
    impl IterNextIterable for PyItertoolsStarmap {}
    impl IterNext for PyItertoolsStarmap {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let obj = zelf.iterable.next(vm)?;
            let function = &zelf.function;
            match obj {
                PyIterReturn::Return(obj) => {
                    let args: Vec<_> = obj.try_to_value(vm)?;
                    PyIterReturn::from_pyresult(vm.invoke(function, args), vm)
                }
                PyIterReturn::StopIteration(v) => Ok(PyIterReturn::StopIteration(v)),
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "takewhile")]
    #[derive(Debug, PyPayload)]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsTakewhile {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyObjectRef, PyIter), u32) {
            (
                zelf.class().to_owned(),
                (zelf.predicate.clone(), zelf.iterable.clone()),
                zelf.stop_flag.load() as _,
            )
        }
        #[pymethod(magic)]
        fn setstate(zelf: PyRef<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if let Ok(obj) = ArgIntoBool::try_from_object(vm, state) {
                zelf.stop_flag.store(*obj);
            }
            Ok(())
        }
    }
    impl IterNextIterable for PyItertoolsTakewhile {}
    impl IterNext for PyItertoolsTakewhile {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[derive(Debug, PyPayload)]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsDropwhile {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyObjectRef, PyIter), u32) {
            (
                zelf.class().to_owned(),
                (zelf.predicate.clone().into(), zelf.iterable.clone()),
                (zelf.start_flag.load() as _),
            )
        }
        #[pymethod(magic)]
        fn setstate(zelf: PyRef<Self>, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if let Ok(obj) = ArgIntoBool::try_from_object(vm, state) {
                zelf.start_flag.store(*obj);
            }
            Ok(())
        }
    }
    impl IterNextIterable for PyItertoolsDropwhile {}
    impl IterNext for PyItertoolsDropwhile {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
        fn is_current(&self, grouper: &Py<PyItertoolsGrouper>) -> bool {
            self.grouper
                .as_ref()
                .and_then(|g| g.upgrade())
                .map_or(false, |ref current_grouper| grouper.is(current_grouper))
        }
    }

    #[pyattr]
    #[pyclass(name = "groupby")]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
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
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
                (state.current_key.as_ref().unwrap().clone(), grouper).to_pyobject(vm),
            ))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper")]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
    }

    #[pyclass(with(IterNext))]
    impl PyItertoolsGrouper {}
    impl IterNextIterable for PyItertoolsGrouper {}
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
    #[derive(Debug, PyPayload)]
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

    #[pyclass(with(IterNext), flags(BASETYPE))]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    impl IterNextIterable for PyItertoolsIslice {}
    impl IterNext for PyItertoolsIslice {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor), flags(BASETYPE))]
    impl PyItertoolsFilterFalse {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>) -> (PyTypeRef, (PyObjectRef, PyIter)) {
            (
                zelf.class().to_owned(),
                (zelf.predicate.clone(), zelf.iterable.clone()),
            )
        }
    }
    impl IterNextIterable for PyItertoolsFilterFalse {}
    impl IterNext for PyItertoolsFilterFalse {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[derive(Debug, PyPayload)]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsAccumulate {}

    impl IterNextIterable for PyItertoolsAccumulate {}
    impl IterNext for PyItertoolsAccumulate {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[derive(Debug, PyPayload)]
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
        fn py_new(
            _cls: PyTypeRef,
            Self::Args { iterable, n }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let n = n.unwrap_or(2);

            let copyable = if iterable.class().has_attr(identifier!(vm, __copy__)) {
                vm.call_special_method(iterable.into(), identifier!(vm, __copy__), ())?
            } else {
                PyItertoolsTee::from_iter(iterable, vm)?
            };

            let mut tee_vec: Vec<PyObjectRef> = Vec::with_capacity(n);
            for _ in 0..n {
                tee_vec.push(vm.call_special_method(
                    copyable.clone(),
                    identifier!(vm, __copy__),
                    (),
                )?);
            }

            Ok(PyTuple::new_ref(tee_vec, &vm.ctx).into())
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsTee {
        fn from_iter(iterator: PyIter, vm: &VirtualMachine) -> PyResult {
            let class = PyItertoolsTee::class(vm);
            if iterator.class().is(PyItertoolsTee::class(vm)) {
                return vm.call_special_method(iterator.into(), identifier!(vm, __copy__), ());
            }
            Ok(PyItertoolsTee {
                tee_data: PyItertoolsTeeData::new(iterator, vm)?,
                index: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, class.to_owned())?
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
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
    #[derive(Debug, PyPayload)]
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
                pools.push(arg.try_to_value(vm)?);
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
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
    #[pyclass(name = "combinations")]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsCombinations {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        result: PyRwLock<Option<Vec<PyObjectRef>>>,
        r: AtomicCell<usize>,
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
            cls: PyTypeRef,
            Self::Args { iterable, r }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pool: Vec<_> = iterable.try_to_value(vm)?;

            let r = r.as_bigint();
            if r.is_negative() {
                return Err(vm.new_value_error("r must be non-negative".to_owned()));
            }
            let r = r.to_usize().unwrap();

            let n = pool.len();

            PyItertoolsCombinations {
                pool,
                indices: PyRwLock::new((0..r).collect()),
                result: PyRwLock::new(None),
                r: AtomicCell::new(r),
                exhausted: AtomicCell::new(r > n),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsCombinations {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            let r = zelf.r.load();

            let class = zelf.class().to_owned();

            if zelf.exhausted.load() {
                return vm.new_tuple((
                    class,
                    vm.new_tuple((vm.ctx.empty_tuple.clone(), vm.ctx.new_int(r))),
                ));
            }

            let tup = vm.new_tuple((zelf.pool.clone().into_pytuple(vm), vm.ctx.new_int(r)));

            if zelf.result.read().is_none() {
                vm.new_tuple((class, tup))
            } else {
                let mut indices: Vec<PyObjectRef> = Vec::new();

                for item in &zelf.indices.read()[..r] {
                    indices.push(vm.new_pyobj(*item));
                }

                vm.new_tuple((class, tup, indices.into_pytuple(vm)))
            }
        }
    }

    impl IterNextIterable for PyItertoolsCombinations {}
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
                } else {
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
                        result[i] = elem.to_owned();
                    }

                    result.to_vec()
                }
            } else {
                let res = zelf.pool[0..r].to_vec();
                *result_lock = Some(res.clone());
                res
            };

            Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations_with_replacement")]
    #[derive(Debug, PyPayload)]
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
            let pool: Vec<_> = iterable.try_to_value(vm)?;
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsCombinationsWithReplacement {}

    impl IterNextIterable for PyItertoolsCombinationsWithReplacement {}
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
    #[pyclass(name = "permutations")]
    #[derive(Debug, PyPayload)]
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
            let pool: Vec<_> = iterable.try_to_value(vm)?;

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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsPermutations {}
    impl IterNextIterable for PyItertoolsPermutations {}
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

        fn py_new(cls: PyTypeRef, (iterators, args): Self::Args, vm: &VirtualMachine) -> PyResult {
            let fillvalue = args.fillvalue.unwrap_or_none(vm);
            let iterators = iterators.into_vec();
            PyItertoolsZipLongest {
                iterators,
                fillvalue: PyRwLock::new(fillvalue),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyattr]
    #[pyclass(name = "zip_longest")]
    #[derive(Debug, PyPayload)]
    struct PyItertoolsZipLongest {
        iterators: Vec<PyIter>,
        fillvalue: PyRwLock<PyObjectRef>,
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsZipLongest {
        #[pymethod(magic)]
        fn reduce(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let args: Vec<PyObjectRef> = zelf
                .iterators
                .iter()
                .map(|i| i.clone().to_pyobject(vm))
                .collect();
            Ok(vm.new_tuple((
                zelf.class().to_owned(),
                vm.new_tuple(args),
                zelf.fillvalue.read().to_owned(),
            )))
        }

        #[pymethod(magic)]
        fn setstate(zelf: PyRef<Self>, state: PyObjectRef, _vm: &VirtualMachine) -> PyResult<()> {
            *zelf.fillvalue.write() = state;
            Ok(())
        }
    }
    impl IterNextIterable for PyItertoolsZipLongest {}
    impl IterNext for PyItertoolsZipLongest {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
                        zelf.fillvalue.read().clone()
                    }
                };
                result.push(next_obj);
            }
            Ok(PyIterReturn::Return(vm.ctx.new_tuple(result).into()))
        }
    }

    #[pyattr]
    #[pyclass(name = "pairwise")]
    #[derive(Debug, PyPayload)]
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
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(IterNext, Constructor))]
    impl PyItertoolsPairwise {}
    impl IterNextIterable for PyItertoolsPairwise {}
    impl IterNext for PyItertoolsPairwise {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
