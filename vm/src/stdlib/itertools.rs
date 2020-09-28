pub(crate) use decl::make_module;

#[pymodule(name = "itertools")]
mod decl {
    use crossbeam_utils::atomic::AtomicCell;
    use num_bigint::BigInt;
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use std::fmt;

    use crate::builtins::int::{self, PyInt, PyIntRef};
    use crate::builtins::pybool;
    use crate::builtins::pytype::PyTypeRef;
    use crate::builtins::tuple::PyTupleRef;
    use crate::common::lock::{PyMutex, PyRwLock, PyRwLockWriteGuard};
    use crate::common::rc::PyRc;
    use crate::function::{Args, FuncArgs, OptionalArg, OptionalOption};
    use crate::iterator::{call_next, get_all, get_iter, get_next_object};
    use crate::pyobject::{
        BorrowValue, IdProtocol, IntoPyRef, PyCallable, PyObjectRc, PyObjectRef, PyObjectWeak,
        PyRef, PyResult, PyValue, StaticType, TypeProtocol,
    };
    use crate::vm::VirtualMachine;

    #[pyattr]
    #[pyclass(name = "chain")]
    #[derive(Debug)]
    struct PyItertoolsChain {
        iterables: Vec<PyObjectRef>,
        cur_idx: AtomicCell<usize>,
        cached_iter: PyRwLock<Option<PyObjectRef>>,
    }

    impl PyValue for PyItertoolsChain {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsChain {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            PyItertoolsChain {
                iterables: args.args,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            loop {
                let pos = self.cur_idx.load();
                if pos >= self.iterables.len() {
                    break;
                }
                let cur_iter = if self.cached_iter.read().is_none() {
                    // We need to call "get_iter" outside of the lock.
                    let iter = get_iter(vm, &self.iterables[pos])?;
                    *self.cached_iter.write() = Some(iter.clone());
                    iter
                } else if let Some(cached_iter) = (*(self.cached_iter.read())).clone() {
                    cached_iter
                } else {
                    // Someone changed cached iter to None since we checked.
                    continue;
                };

                // We need to call "call_next" outside of the lock.
                match call_next(vm, &cur_iter) {
                    Ok(ok) => return Ok(ok),
                    Err(err) => {
                        if err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                            self.cur_idx.fetch_add(1);
                            *self.cached_iter.write() = None;
                        } else {
                            return Err(err);
                        }
                    }
                }
            }

            Err(vm.new_stop_iteration())
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pyclassmethod(name = "from_iterable")]
        fn from_iterable(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let it = get_iter(vm, &iterable)?;
            let iterables = get_all(vm, &it)?;

            PyItertoolsChain {
                iterables,
                cur_idx: AtomicCell::new(0),
                cached_iter: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }
    }

    #[pyattr]
    #[pyclass(name = "compress")]
    #[derive(Debug)]
    struct PyItertoolsCompress {
        data: PyObjectRef,
        selector: PyObjectRef,
    }

    impl PyValue for PyItertoolsCompress {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsCompress {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            data: PyObjectRef,
            selector: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let data_iter = get_iter(vm, &data)?;
            let selector_iter = get_iter(vm, &selector)?;

            PyItertoolsCompress {
                data: data_iter,
                selector: selector_iter,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            loop {
                let sel_obj = call_next(vm, &self.selector)?;
                let verdict = pybool::boolval(vm, sel_obj.clone())?;
                let data_obj = call_next(vm, &self.data)?;

                if verdict {
                    return Ok(data_obj);
                }
            }
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "count")]
    #[derive(Debug)]
    struct PyItertoolsCount {
        cur: PyRwLock<BigInt>,
        step: BigInt,
    }

    impl PyValue for PyItertoolsCount {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsCount {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            start: OptionalArg<PyIntRef>,
            step: OptionalArg<PyIntRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let start = match start.into_option() {
                Some(int) => int.borrow_value().clone(),
                None => BigInt::zero(),
            };
            let step = match step.into_option() {
                Some(int) => int.borrow_value().clone(),
                None => BigInt::one(),
            };

            PyItertoolsCount {
                cur: PyRwLock::new(start),
                step,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let mut cur = self.cur.write();
            let result = cur.clone();
            *cur += &self.step;
            Ok(result.into_pyref(vm))
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "cycle")]
    #[derive(Debug)]
    struct PyItertoolsCycle {
        iter: PyObjectRef,
        saved: PyRwLock<Vec<PyObjectRef>>,
        index: AtomicCell<usize>,
    }

    impl PyValue for PyItertoolsCycle {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsCycle {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsCycle {
                iter,
                saved: PyRwLock::new(Vec::new()),
                index: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let item = if let Some(item) = get_next_object(vm, &self.iter)? {
                self.saved.write().push(item.clone());
                item
            } else {
                let saved = self.saved.read();
                if saved.len() == 0 {
                    return Err(vm.new_stop_iteration());
                }

                let last_index = self.index.fetch_add(1);

                if last_index >= saved.len() - 1 {
                    self.index.store(0);
                }

                saved[last_index].clone()
            };

            Ok(item)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "repeat")]
    #[derive(Debug)]
    struct PyItertoolsRepeat {
        object: PyObjectRef,
        times: Option<PyRwLock<BigInt>>,
    }

    impl PyValue for PyItertoolsRepeat {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsRepeat {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            object: PyObjectRef,
            times: OptionalArg<PyIntRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let times = match times.into_option() {
                Some(int) => Some(PyRwLock::new(int.borrow_value().clone())),
                None => None,
            };

            PyItertoolsRepeat { object, times }.into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            if let Some(ref times) = self.times {
                let mut times = times.write();
                if !times.is_positive() {
                    return Err(vm.new_stop_iteration());
                }
                *times -= 1;
            }

            Ok(self.object.clone())
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(name = "__length_hint__")]
        fn length_hint(&self, vm: &VirtualMachine) -> PyObjectRef {
            match self.times {
                Some(ref times) => vm.ctx.new_int(times.read().clone()),
                None => vm.ctx.new_int(0),
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "starmap")]
    #[derive(Debug)]
    struct PyItertoolsStarmap {
        function: PyObjectRef,
        iter: PyObjectRef,
    }

    impl PyValue for PyItertoolsStarmap {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsStarmap {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            function: PyObjectRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsStarmap { function, iter }.into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let obj = call_next(vm, &self.iter)?;
            let function = &self.function;

            vm.invoke(function, vm.extract_elements(&obj)?)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "takewhile")]
    #[derive(Debug)]
    struct PyItertoolsTakewhile {
        predicate: PyObjectRef,
        iterable: PyObjectRef,
        stop_flag: AtomicCell<bool>,
    }

    impl PyValue for PyItertoolsTakewhile {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsTakewhile {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            predicate: PyObjectRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsTakewhile {
                predicate,
                iterable: iter,
                stop_flag: AtomicCell::new(false),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            if self.stop_flag.load() {
                return Err(vm.new_stop_iteration());
            }

            // might be StopIteration or anything else, which is propagated upwards
            let obj = call_next(vm, &self.iterable)?;
            let predicate = &self.predicate;

            let verdict = vm.invoke(predicate, (obj.clone(),))?;
            let verdict = pybool::boolval(vm, verdict)?;
            if verdict {
                Ok(obj)
            } else {
                self.stop_flag.store(true);
                Err(vm.new_stop_iteration())
            }
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "dropwhile")]
    #[derive(Debug)]
    struct PyItertoolsDropwhile {
        predicate: PyCallable,
        iterable: PyObjectRef,
        start_flag: AtomicCell<bool>,
    }

    impl PyValue for PyItertoolsDropwhile {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsDropwhile {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            predicate: PyCallable,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsDropwhile {
                predicate,
                iterable: iter,
                start_flag: AtomicCell::new(false),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let predicate = &self.predicate;
            let iterable = &self.iterable;

            if !self.start_flag.load() {
                loop {
                    let obj = call_next(vm, iterable)?;
                    let pred = predicate.clone();
                    let pred_value = vm.invoke(&pred.into_object(), (obj.clone(),))?;
                    if !pybool::boolval(vm, pred_value)? {
                        self.start_flag.store(true);
                        return Ok(obj);
                    }
                }
            }
            call_next(vm, iterable)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    struct GroupByState {
        current_value: Option<PyObjectRef>,
        current_key: Option<PyObjectRef>,
        next_group: bool,
        grouper: Option<PyObjectWeak<PyItertoolsGrouper>>,
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
        fn is_current(&self, grouper: &PyItertoolsGrouperRef) -> bool {
            self.grouper
                .as_ref()
                .and_then(|g| g.upgrade())
                .map_or(false, |ref current_grouper| grouper.is(current_grouper))
        }
    }

    #[pyattr]
    #[pyclass(name = "groupby")]
    struct PyItertoolsGroupBy {
        iterable: PyObjectRef,
        key_func: Option<PyObjectRef>,
        state: PyMutex<GroupByState>,
    }

    impl PyValue for PyItertoolsGroupBy {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
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
        iterable: PyObjectRef,
        #[pyarg(any, optional)]
        key: OptionalOption<PyObjectRef>,
    }

    #[pyimpl]
    impl PyItertoolsGroupBy {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, args: GroupByArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &args.iterable)?;

            PyItertoolsGroupBy {
                iterable: iter,
                key_func: args.key.flatten(),
                state: PyMutex::new(GroupByState {
                    current_key: None,
                    current_value: None,
                    next_group: false,
                    grouper: None,
                }),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(
            zelf: PyRef<Self>,
            vm: &VirtualMachine,
        ) -> PyResult<(PyObjectRef, PyItertoolsGrouperRef)> {
            let mut state = zelf.state.lock();
            state.grouper = None;

            if !state.next_group {
                // FIXME: unnecessary clone. current_key always exist until assinging new
                let current_key = state.current_key.clone();
                drop(state);

                let (value, key) = if let Some(old_key) = current_key {
                    loop {
                        let (value, new_key) = zelf.advance(vm)?;
                        if !vm.bool_eq(&new_key, &old_key)? {
                            break (value, new_key);
                        }
                    }
                } else {
                    zelf.advance(vm)?
                };

                state = zelf.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
            }

            state.next_group = false;

            let grouper = PyItertoolsGrouper {
                groupby: zelf.clone(),
            }
            .into_ref(vm);

            state.grouper = Some(PyObjectRc::downgrade(&grouper.clone().into_typed_pyobj()));
            Ok((state.current_key.as_ref().unwrap().clone(), grouper))
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        pub(super) fn advance(&self, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
            let new_value = call_next(vm, &self.iterable)?;
            let new_key = if let Some(ref kf) = self.key_func {
                vm.invoke(kf, vec![new_value.clone()])?
            } else {
                new_value.clone()
            };
            Ok((new_value, new_key))
        }
    }

    #[pyattr]
    #[pyclass(name = "_grouper")]
    #[derive(Debug)]
    struct PyItertoolsGrouper {
        groupby: PyRef<PyItertoolsGroupBy>,
    }

    type PyItertoolsGrouperRef = PyRef<PyItertoolsGrouper>;

    impl PyValue for PyItertoolsGrouper {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsGrouper {
        #[pymethod(name = "__next__")]
        fn next(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let old_key = {
                let mut state = zelf.groupby.state.lock();

                if !state.is_current(&zelf) {
                    return Err(vm.new_stop_iteration());
                }

                // check to see if the value has already been retrieved from the iterator
                if let Some(val) = state.current_value.take() {
                    return Ok(val);
                }

                state.current_key.as_ref().unwrap().clone()
            };
            let (value, key) = zelf.groupby.advance(vm)?;
            if vm.bool_eq(&key, &old_key)? {
                Ok(value)
            } else {
                let mut state = zelf.groupby.state.lock();
                state.current_value = Some(value);
                state.current_key = Some(key);
                state.next_group = true;
                state.grouper = None;
                Err(vm.new_stop_iteration())
            }
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "islice")]
    #[derive(Debug)]
    struct PyItertoolsIslice {
        iterable: PyObjectRef,
        cur: AtomicCell<usize>,
        next: AtomicCell<usize>,
        stop: Option<usize>,
        step: usize,
    }

    impl PyValue for PyItertoolsIslice {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    fn pyobject_to_opt_usize(obj: PyObjectRef, vm: &VirtualMachine) -> Option<usize> {
        let is_int = obj.isinstance(&vm.ctx.types.int_type);
        if is_int {
            int::get_value(&obj).to_usize()
        } else {
            None
        }
    }

    #[pyimpl]
    impl PyItertoolsIslice {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
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
                    let (iter, start, stop, step): (
                        PyObjectRef,
                        PyObjectRef,
                        PyObjectRef,
                        PyObjectRef,
                    ) = args.bind(vm)?;

                    let start = if !vm.is_none(&start) {
                        pyobject_to_opt_usize(start, &vm).ok_or_else(|| {
                        vm.new_value_error(
                            "Indices for islice() must be None or an integer: 0 <= x <= sys.maxsize.".to_owned(),
                        )
                    })?
                    } else {
                        0usize
                    };

                    let step = if !vm.is_none(&step) {
                        pyobject_to_opt_usize(step, &vm).ok_or_else(|| {
                            vm.new_value_error(
                                "Step for islice() must be a positive integer or None.".to_owned(),
                            )
                        })?
                    } else {
                        1usize
                    };

                    (iter, start, stop, step)
                }
            };

            let stop = if !vm.is_none(&stop) {
                Some(pyobject_to_opt_usize(stop, &vm).ok_or_else(|| {
                    vm.new_value_error(
                    "Stop argument for islice() must be None or an integer: 0 <= x <= sys.maxsize."
                        .to_owned(),
                )
                })?)
            } else {
                None
            };

            let iter = get_iter(vm, &iter)?;

            PyItertoolsIslice {
                iterable: iter,
                cur: AtomicCell::new(0),
                next: AtomicCell::new(start),
                stop,
                step,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            while self.cur.load() < self.next.load() {
                call_next(vm, &self.iterable)?;
                self.cur.fetch_add(1);
            }

            if let Some(stop) = self.stop {
                if self.cur.load() >= stop {
                    return Err(vm.new_stop_iteration());
                }
            }

            let obj = call_next(vm, &self.iterable)?;
            self.cur.fetch_add(1);

            // TODO is this overflow check required? attempts to copy CPython.
            let (next, ovf) = self.next.load().overflowing_add(self.step);
            self.next.store(if ovf { self.stop.unwrap() } else { next });

            Ok(obj)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "filterfalse")]
    #[derive(Debug)]
    struct PyItertoolsFilterFalse {
        predicate: PyObjectRef,
        iterable: PyObjectRef,
    }

    impl PyValue for PyItertoolsFilterFalse {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsFilterFalse {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            predicate: PyObjectRef,
            iterable: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsFilterFalse {
                predicate,
                iterable: iter,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let predicate = &self.predicate;
            let iterable = &self.iterable;

            loop {
                let obj = call_next(vm, iterable)?;
                let pred_value = if vm.is_none(predicate) {
                    obj.clone()
                } else {
                    vm.invoke(predicate, vec![obj.clone()])?
                };

                if !pybool::boolval(vm, pred_value)? {
                    return Ok(obj);
                }
            }
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "accumulate")]
    #[derive(Debug)]
    struct PyItertoolsAccumulate {
        iterable: PyObjectRef,
        binop: PyObjectRef,
        acc_value: PyRwLock<Option<PyObjectRef>>,
    }

    impl PyValue for PyItertoolsAccumulate {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsAccumulate {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            binop: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;

            PyItertoolsAccumulate {
                iterable: iter,
                binop: binop.unwrap_or_none(vm),
                acc_value: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let iterable = &self.iterable;
            let obj = call_next(vm, iterable)?;

            let acc_value = self.acc_value.read().clone();

            let next_acc_value = match acc_value {
                None => obj,
                Some(value) => {
                    if vm.is_none(&self.binop) {
                        vm._add(&value, &obj)?
                    } else {
                        vm.invoke(&self.binop, vec![value, obj])?
                    }
                }
            };
            *self.acc_value.write() = Some(next_acc_value.clone());

            Ok(next_acc_value)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[derive(Debug)]
    struct PyItertoolsTeeData {
        iterable: PyObjectRef,
        values: PyRwLock<Vec<PyObjectRef>>,
    }

    impl PyItertoolsTeeData {
        fn new(iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRc<PyItertoolsTeeData>> {
            Ok(PyRc::new(PyItertoolsTeeData {
                iterable: get_iter(vm, &iterable)?,
                values: PyRwLock::new(vec![]),
            }))
        }

        fn get_item(&self, vm: &VirtualMachine, index: usize) -> PyResult {
            if self.values.read().len() == index {
                let result = call_next(vm, &self.iterable)?;
                self.values.write().push(result);
            }
            Ok(self.values.read()[index].clone())
        }
    }

    #[pyattr]
    #[pyclass(name = "tee")]
    #[derive(Debug)]
    struct PyItertoolsTee {
        tee_data: PyRc<PyItertoolsTeeData>,
        index: AtomicCell<usize>,
    }

    impl PyValue for PyItertoolsTee {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsTee {
        fn from_iter(iterable: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let class = PyItertoolsTee::class(vm);
            let it = get_iter(vm, &iterable)?;
            if it.class().is(PyItertoolsTee::class(vm)) {
                return vm.call_method(&it, "__copy__", ());
            }
            Ok(PyItertoolsTee {
                tee_data: PyItertoolsTeeData::new(it, vm)?,
                index: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, class.clone())?
            .into_object())
        }

        // TODO: make tee() a function, rename this class to itertools._tee and make
        // teedata a python class
        #[pyslot]
        #[allow(clippy::new_ret_no_self)]
        fn tp_new(
            _cls: PyTypeRef,
            iterable: PyObjectRef,
            n: OptionalArg<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
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

            Ok(PyTupleRef::with_elements(tee_vec, &vm.ctx))
        }

        #[pymethod(name = "__copy__")]
        fn copy(&self, vm: &VirtualMachine) -> PyResult {
            Ok(PyItertoolsTee {
                tee_data: PyRc::clone(&self.tee_data),
                index: AtomicCell::new(self.index.load()),
            }
            .into_ref_with_type(vm, Self::class(vm).clone())?
            .into_object())
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            let value = self.tee_data.get_item(vm, self.index.load())?;
            self.index.fetch_add(1);
            Ok(value)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "product")]
    #[derive(Debug)]
    struct PyItertoolsProduct {
        pools: Vec<Vec<PyObjectRef>>,
        idxs: PyRwLock<Vec<usize>>,
        cur: AtomicCell<usize>,
        stop: AtomicCell<bool>,
    }

    impl PyValue for PyItertoolsProduct {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[derive(FromArgs)]
    struct ProductArgs {
        #[pyarg(named, optional)]
        repeat: OptionalArg<usize>,
    }

    #[pyimpl]
    impl PyItertoolsProduct {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterables: Args<PyObjectRef>,
            args: ProductArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let repeat = match args.repeat.into_option() {
                Some(i) => i,
                None => 1,
            };

            let mut pools = Vec::new();
            for arg in iterables.into_iter() {
                let it = get_iter(vm, &arg)?;
                let pool = get_all(vm, &it)?;

                pools.push(pool);
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
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if self.stop.load() {
                return Err(vm.new_stop_iteration());
            }

            let pools = &self.pools;

            for p in pools {
                if p.is_empty() {
                    return Err(vm.new_stop_iteration());
                }
            }

            let idxs = self.idxs.write();
            let res = vm.ctx.new_tuple(
                pools
                    .iter()
                    .zip(idxs.iter())
                    .map(|(pool, idx)| pool[*idx].clone())
                    .collect(),
            );

            self.update_idxs(idxs);

            Ok(res)
        }

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

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations")]
    #[derive(Debug)]
    struct PyItertoolsCombinations {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    impl PyValue for PyItertoolsCombinations {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsCombinations {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            r: PyIntRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;
            let pool = get_all(vm, &iter)?;

            let r = r.borrow_value();
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
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if self.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = self.pool.len();
            let r = self.r.load();

            if r == 0 {
                self.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let res = vm.ctx.new_tuple(
                self.indices
                    .read()
                    .iter()
                    .map(|&i| self.pool[i].clone())
                    .collect(),
            );

            let mut indices = self.indices.write();

            // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
            let mut idx = r as isize - 1;
            while idx >= 0 && indices[idx as usize] == idx as usize + n - r {
                idx -= 1;
            }

            // If no suitable index is found, then the indices are all at
            // their maximum value and we're done.
            if idx < 0 {
                self.exhausted.store(true);
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

            Ok(res)
        }
    }

    #[pyattr]
    #[pyclass(name = "combinations_with_replacement")]
    #[derive(Debug)]
    struct PyItertoolsCombinationsWithReplacement {
        pool: Vec<PyObjectRef>,
        indices: PyRwLock<Vec<usize>>,
        r: AtomicCell<usize>,
        exhausted: AtomicCell<bool>,
    }

    impl PyValue for PyItertoolsCombinationsWithReplacement {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsCombinationsWithReplacement {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            r: PyIntRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;
            let pool = get_all(vm, &iter)?;

            let r = r.borrow_value();
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
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if self.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = self.pool.len();
            let r = self.r.load();

            if r == 0 {
                self.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let mut indices = self.indices.write();

            let res = vm
                .ctx
                .new_tuple(indices.iter().map(|&i| self.pool[i].clone()).collect());

            // Scan indices right-to-left until finding one that is not at its maximum (i + n - r).
            let mut idx = r as isize - 1;
            while idx >= 0 && indices[idx as usize] == n - 1 {
                idx -= 1;
            }

            // If no suitable index is found, then the indices are all at
            // their maximum value and we're done.
            if idx < 0 {
                self.exhausted.store(true);
            } else {
                let index = indices[idx as usize] + 1;

                // Increment the current index which we know is not at its
                // maximum. Then set all to the right to the same value.
                for j in idx as usize..r {
                    indices[j as usize] = index as usize;
                }
            }

            Ok(res)
        }
    }

    #[pyattr]
    #[pyclass(name = "permutations")]
    #[derive(Debug)]
    struct PyItertoolsPermutations {
        pool: Vec<PyObjectRef>,               // Collected input iterable
        indices: PyRwLock<Vec<usize>>,        // One index per element in pool
        cycles: PyRwLock<Vec<usize>>,         // One rollover counter per element in the result
        result: PyRwLock<Option<Vec<usize>>>, // Indexes of the most recently returned result
        r: AtomicCell<usize>,                 // Size of result tuple
        exhausted: AtomicCell<bool>,          // Set when the iterator is exhausted
    }

    impl PyValue for PyItertoolsPermutations {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyItertoolsPermutations {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterable: PyObjectRef,
            r: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let iter = get_iter(vm, &iterable)?;
            let pool = get_all(vm, &iter)?;

            let n = pool.len();
            // If r is not provided, r == n. If provided, r must be a positive integer, or None.
            // If None, it behaves the same as if it was not provided.
            let r = match r.flatten() {
                Some(r) => {
                    let val = r
                        .payload::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("Expected int as r".to_owned()))?
                        .borrow_value();

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
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            // stop signal
            if self.exhausted.load() {
                return Err(vm.new_stop_iteration());
            }

            let n = self.pool.len();
            let r = self.r.load();

            if n == 0 {
                self.exhausted.store(true);
                return Ok(vm.ctx.new_tuple(vec![]));
            }

            let mut result = self.result.write();

            if let Some(ref mut result) = *result {
                let mut indices = self.indices.write();
                let mut cycles = self.cycles.write();
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
                    self.exhausted.store(true);
                    return Err(vm.new_stop_iteration());
                }
            } else {
                // On the first pass, initialize result tuple using the indices
                *result = Some((0..r).collect());
            }

            Ok(vm.ctx.new_tuple(
                result
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&i| self.pool[i].clone())
                    .collect(),
            ))
        }
    }

    #[pyattr]
    #[pyclass(name = "zip_longest")]
    #[derive(Debug)]
    struct PyItertoolsZipLongest {
        iterators: Vec<PyObjectRef>,
        fillvalue: PyObjectRef,
    }

    impl PyValue for PyItertoolsZipLongest {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[derive(FromArgs)]
    struct ZiplongestArgs {
        #[pyarg(named, optional)]
        fillvalue: OptionalArg<PyObjectRef>,
    }

    #[pyimpl]
    impl PyItertoolsZipLongest {
        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            iterables: Args,
            args: ZiplongestArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyRef<Self>> {
            let fillvalue = args.fillvalue.unwrap_or_none(vm);
            let iterators = iterables
                .into_iter()
                .map(|iterable| get_iter(vm, &iterable))
                .collect::<Result<Vec<_>, _>>()?;

            PyItertoolsZipLongest {
                iterators,
                fillvalue,
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(name = "__next__")]
        fn next(&self, vm: &VirtualMachine) -> PyResult {
            if self.iterators.is_empty() {
                Err(vm.new_stop_iteration())
            } else {
                let mut result: Vec<PyObjectRef> = Vec::new();
                let mut numactive = self.iterators.len();

                for idx in 0..self.iterators.len() {
                    let next_obj = match call_next(vm, &self.iterators[idx]) {
                        Ok(obj) => obj,
                        Err(err) => {
                            if !err.isinstance(&vm.ctx.exceptions.stop_iteration) {
                                return Err(err);
                            }
                            numactive -= 1;
                            if numactive == 0 {
                                return Err(vm.new_stop_iteration());
                            }
                            self.fillvalue.clone()
                        }
                    };
                    result.push(next_obj);
                }
                Ok(vm.ctx.new_tuple(result))
            }
        }

        #[pymethod(name = "__iter__")]
        fn iter(zelf: PyRef<Self>) -> PyRef<Self> {
            zelf
        }
    }
}
