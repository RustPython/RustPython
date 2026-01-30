//! _asyncio module - provides native asyncio support
//!
//! This module provides native implementations of Future and Task classes,

pub(crate) use _asyncio::module_def;

#[pymodule]
pub(crate) mod _asyncio {
    use crate::{
        common::lock::PyRwLock,
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
            builtins::{
                PyBaseException, PyBaseExceptionRef, PyDict, PyDictRef, PyGenericAlias, PyList,
                PyListRef, PyModule, PySet, PyTuple, PyType, PyTypeRef,
            },
            extend_module,
            function::{FuncArgs, KwArgs, OptionalArg, OptionalOption, PySetterValue},
            protocol::PyIterReturn,
            recursion::ReprGuard,
            types::{
                Callable, Constructor, Destructor, Initializer, IterNext, Iterable, Representable,
                SelfIter,
            },
            warn,
        },
    };
    use crossbeam_utils::atomic::AtomicCell;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);

        // Initialize module-level state
        let weakref_module = vm.import("weakref", 0)?;
        let weak_set_class = vm
            .get_attribute_opt(weakref_module, vm.ctx.intern_str("WeakSet"))?
            .ok_or_else(|| vm.new_attribute_error("WeakSet not found"))?;
        let scheduled_tasks = weak_set_class.call((), vm)?;
        let eager_tasks = PySet::default().into_ref(&vm.ctx);
        let current_tasks = PyDict::default().into_ref(&vm.ctx);

        extend_module!(vm, module, {
            "_scheduled_tasks" => scheduled_tasks,
            "_eager_tasks" => eager_tasks,
            "_current_tasks" => current_tasks,
        });

        // Register fork handler to clear task state in child process
        #[cfg(unix)]
        {
            let on_fork = vm
                .get_attribute_opt(module.to_owned().into(), vm.ctx.intern_str("_on_fork"))?
                .expect("_on_fork not found in _asyncio module");
            vm.state.after_forkers_child.lock().push(on_fork);
        }

        Ok(())
    }

    #[derive(FromArgs)]
    struct AddDoneCallbackArgs {
        #[pyarg(positional)]
        func: PyObjectRef,
        #[pyarg(named, optional)]
        context: OptionalOption<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct CancelArgs {
        #[pyarg(any, optional)]
        msg: OptionalOption<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct LoopArg {
        #[pyarg(any, name = "loop", optional)]
        loop_: OptionalOption<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetStackArgs {
        #[pyarg(named, optional)]
        limit: OptionalOption<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct PrintStackArgs {
        #[pyarg(named, optional)]
        limit: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        file: OptionalOption<PyObjectRef>,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum FutureState {
        Pending,
        Cancelled,
        Finished,
    }

    impl FutureState {
        fn as_str(&self) -> &'static str {
            match self {
                FutureState::Pending => "PENDING",
                FutureState::Cancelled => "CANCELLED",
                FutureState::Finished => "FINISHED",
            }
        }
    }

    /// asyncio.Future implementation
    #[pyattr]
    #[pyclass(name = "Future", module = "_asyncio", traverse)]
    #[derive(Debug, PyPayload)]
    #[repr(C)] // Required for inheritance - ensures base field is at offset 0 in subclasses
    struct PyFuture {
        fut_loop: PyRwLock<Option<PyObjectRef>>,
        fut_callback0: PyRwLock<Option<PyObjectRef>>,
        fut_context0: PyRwLock<Option<PyObjectRef>>,
        fut_callbacks: PyRwLock<Option<PyObjectRef>>,
        fut_exception: PyRwLock<Option<PyObjectRef>>,
        fut_exception_tb: PyRwLock<Option<PyObjectRef>>,
        fut_result: PyRwLock<Option<PyObjectRef>>,
        fut_source_tb: PyRwLock<Option<PyObjectRef>>,
        fut_cancel_msg: PyRwLock<Option<PyObjectRef>>,
        fut_cancelled_exc: PyRwLock<Option<PyObjectRef>>,
        fut_awaited_by: PyRwLock<Option<PyObjectRef>>,
        #[pytraverse(skip)]
        fut_state: AtomicCell<FutureState>,
        #[pytraverse(skip)]
        fut_awaited_by_is_set: AtomicBool,
        #[pytraverse(skip)]
        fut_log_tb: AtomicBool,
        #[pytraverse(skip)]
        fut_blocking: AtomicBool,
    }

    impl Constructor for PyFuture {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(PyFuture::new_empty())
        }
    }

    impl Initializer for PyFuture {
        type Args = FuncArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            // Future does not accept positional arguments
            if !args.args.is_empty() {
                return Err(vm.new_type_error("Future() takes no positional arguments".to_string()));
            }
            // Extract only 'loop' keyword argument
            let loop_ = args.kwargs.get("loop").cloned();
            PyFuture::py_init(&zelf, loop_, vm)
        }
    }

    #[pyclass(
        flags(BASETYPE, HAS_DICT),
        with(Constructor, Initializer, Destructor, Representable, Iterable)
    )]
    impl PyFuture {
        fn new_empty() -> Self {
            Self {
                fut_loop: PyRwLock::new(None),
                fut_callback0: PyRwLock::new(None),
                fut_context0: PyRwLock::new(None),
                fut_callbacks: PyRwLock::new(None),
                fut_exception: PyRwLock::new(None),
                fut_exception_tb: PyRwLock::new(None),
                fut_result: PyRwLock::new(None),
                fut_source_tb: PyRwLock::new(None),
                fut_cancel_msg: PyRwLock::new(None),
                fut_cancelled_exc: PyRwLock::new(None),
                fut_awaited_by: PyRwLock::new(None),
                fut_state: AtomicCell::new(FutureState::Pending),
                fut_awaited_by_is_set: AtomicBool::new(false),
                fut_log_tb: AtomicBool::new(false),
                fut_blocking: AtomicBool::new(false),
            }
        }

        fn py_init(
            zelf: &PyRef<Self>,
            loop_: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // Get the event loop
            let loop_obj = match loop_ {
                Some(l) if !vm.is_none(&l) => l,
                _ => get_event_loop(vm)?,
            };
            *zelf.fut_loop.write() = Some(loop_obj.clone());

            // Check if loop has get_debug method and call it
            if let Ok(Some(get_debug)) =
                vm.get_attribute_opt(loop_obj.clone(), vm.ctx.intern_str("get_debug"))
                && let Ok(debug) = get_debug.call((), vm)
                && debug.try_to_bool(vm).unwrap_or(false)
            {
                // Get source traceback
                if let Ok(tb_module) = vm.import("traceback", 0)
                    && let Ok(Some(extract_stack)) =
                        vm.get_attribute_opt(tb_module, vm.ctx.intern_str("extract_stack"))
                    && let Ok(tb) = extract_stack.call((), vm)
                {
                    *zelf.fut_source_tb.write() = Some(tb);
                }
            }

            Ok(())
        }

        #[pymethod]
        fn result(&self, vm: &VirtualMachine) -> PyResult {
            match self.fut_state.load() {
                FutureState::Pending => Err(new_invalid_state_error(vm, "Result is not ready.")),
                FutureState::Cancelled => {
                    let exc = self.make_cancelled_error_impl(vm);
                    Err(exc)
                }
                FutureState::Finished => {
                    self.fut_log_tb.store(false, Ordering::Relaxed);
                    if let Some(exc) = self.fut_exception.read().clone() {
                        let exc: PyBaseExceptionRef = exc.downcast().unwrap();
                        // Restore the original traceback to prevent traceback accumulation
                        if let Some(tb) = self.fut_exception_tb.read().clone() {
                            let _ = exc.set___traceback__(tb, vm);
                        }
                        Err(exc)
                    } else {
                        Ok(self
                            .fut_result
                            .read()
                            .clone()
                            .unwrap_or_else(|| vm.ctx.none()))
                    }
                }
            }
        }

        #[pymethod]
        fn exception(&self, vm: &VirtualMachine) -> PyResult {
            match self.fut_state.load() {
                FutureState::Pending => Err(new_invalid_state_error(vm, "Exception is not set.")),
                FutureState::Cancelled => {
                    let exc = self.make_cancelled_error_impl(vm);
                    Err(exc)
                }
                FutureState::Finished => {
                    self.fut_log_tb.store(false, Ordering::Relaxed);
                    Ok(self
                        .fut_exception
                        .read()
                        .clone()
                        .unwrap_or_else(|| vm.ctx.none()))
                }
            }
        }

        #[pymethod]
        fn set_result(zelf: PyRef<Self>, result: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if zelf.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            if zelf.fut_state.load() != FutureState::Pending {
                return Err(new_invalid_state_error(vm, "invalid state"));
            }
            *zelf.fut_result.write() = Some(result);
            zelf.fut_state.store(FutureState::Finished);
            Self::schedule_callbacks(&zelf, vm)?;
            Ok(())
        }

        #[pymethod]
        fn set_exception(
            zelf: PyRef<Self>,
            exception: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if zelf.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            if zelf.fut_state.load() != FutureState::Pending {
                return Err(new_invalid_state_error(vm, "invalid state"));
            }

            // Normalize the exception
            let exc = if exception.fast_isinstance(vm.ctx.types.type_type) {
                exception.call((), vm)?
            } else {
                exception
            };

            if !exc.fast_isinstance(vm.ctx.exceptions.base_exception_type) {
                return Err(vm.new_type_error(format!(
                    "exception must be a BaseException, not {}",
                    exc.class().name()
                )));
            }

            // Wrap StopIteration in RuntimeError
            let exc = if exc.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
                let msg = "StopIteration interacts badly with generators and cannot be raised into a Future";
                let runtime_err = vm.new_runtime_error(msg.to_string());
                // Set cause and context to the original StopIteration
                let stop_iter: PyRef<PyBaseException> = exc.downcast().unwrap();
                runtime_err.set___cause__(Some(stop_iter.clone()));
                runtime_err.set___context__(Some(stop_iter));
                runtime_err.into()
            } else {
                exc
            };

            // Save the original traceback for later restoration
            if let Ok(exc_ref) = exc.clone().downcast::<PyBaseException>() {
                let tb = exc_ref.__traceback__().map(|tb| tb.into());
                *zelf.fut_exception_tb.write() = tb;
            }

            *zelf.fut_exception.write() = Some(exc);
            zelf.fut_state.store(FutureState::Finished);
            zelf.fut_log_tb.store(true, Ordering::Relaxed);
            Self::schedule_callbacks(&zelf, vm)?;
            Ok(())
        }

        #[pymethod]
        fn add_done_callback(
            zelf: PyRef<Self>,
            args: AddDoneCallbackArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if zelf.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            let ctx = match args.context.flatten() {
                Some(c) => c,
                None => get_copy_context(vm)?,
            };

            if zelf.fut_state.load() != FutureState::Pending {
                Self::call_soon_with_context(&zelf, args.func, Some(ctx), vm)?;
            } else if zelf.fut_callback0.read().is_none() {
                *zelf.fut_callback0.write() = Some(args.func);
                *zelf.fut_context0.write() = Some(ctx);
            } else {
                let tuple = vm.ctx.new_tuple(vec![args.func, ctx]);
                let mut callbacks = zelf.fut_callbacks.write();
                if callbacks.is_none() {
                    *callbacks = Some(vm.ctx.new_list(vec![tuple.into()]).into());
                } else {
                    let list = callbacks.as_ref().unwrap();
                    vm.call_method(list, "append", (tuple,))?;
                }
            }
            Ok(())
        }

        #[pymethod]
        fn remove_done_callback(&self, func: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            if self.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            let mut cleared_callback0 = 0usize;

            // Check fut_callback0 first
            // Clone to release lock before comparison (which may run Python code)
            let cb0 = self.fut_callback0.read().clone();
            if let Some(cb0) = cb0 {
                let cmp = vm.identical_or_equal(&cb0, &func)?;
                if cmp {
                    *self.fut_callback0.write() = None;
                    *self.fut_context0.write() = None;
                    cleared_callback0 = 1;
                }
            }

            // Check if fut_callbacks exists
            let callbacks = self.fut_callbacks.read().clone();
            let callbacks = match callbacks {
                Some(c) => c,
                None => return Ok(cleared_callback0),
            };

            let list: PyListRef = callbacks.downcast().unwrap();
            let len = list.borrow_vec().len();

            if len == 0 {
                *self.fut_callbacks.write() = None;
                return Ok(cleared_callback0);
            }

            // Special case for single callback
            if len == 1 {
                let item = list.borrow_vec().first().cloned();
                if let Some(item) = item {
                    let tuple: &PyTuple = item.downcast_ref().unwrap();
                    let cb = tuple.first().unwrap().clone();
                    let cmp = vm.identical_or_equal(&cb, &func)?;
                    if cmp {
                        *self.fut_callbacks.write() = None;
                        return Ok(1 + cleared_callback0);
                    }
                }
                return Ok(cleared_callback0);
            }

            // Multiple callbacks - iterate with index, checking validity each time
            // to handle evil comparisons
            let mut new_callbacks = Vec::with_capacity(len);
            let mut i = 0usize;
            let mut removed = 0usize;

            loop {
                // Re-check fut_callbacks on each iteration (evil code may have cleared it)
                let callbacks = self.fut_callbacks.read().clone();
                let callbacks = match callbacks {
                    Some(c) => c,
                    None => break,
                };
                let list: PyListRef = callbacks.downcast().unwrap();
                let current_len = list.borrow_vec().len();
                if i >= current_len {
                    break;
                }

                // Get item and release lock before comparison
                let item = list.borrow_vec().get(i).cloned();
                let item = match item {
                    Some(item) => item,
                    None => break,
                };

                let tuple: &PyTuple = item.downcast_ref().unwrap();
                let cb = tuple.first().unwrap().clone();
                let cmp = vm.identical_or_equal(&cb, &func)?;

                if !cmp {
                    new_callbacks.push(item);
                } else {
                    removed += 1;
                }
                i += 1;
            }

            // Update fut_callbacks with filtered list
            if new_callbacks.is_empty() {
                *self.fut_callbacks.write() = None;
            } else {
                *self.fut_callbacks.write() = Some(vm.ctx.new_list(new_callbacks).into());
            }

            Ok(removed + cleared_callback0)
        }

        #[pymethod]
        fn cancel(zelf: PyRef<Self>, args: CancelArgs, vm: &VirtualMachine) -> PyResult<bool> {
            if zelf.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            if zelf.fut_state.load() != FutureState::Pending {
                // Clear log_tb even when cancel fails
                zelf.fut_log_tb.store(false, Ordering::Relaxed);
                return Ok(false);
            }

            *zelf.fut_cancel_msg.write() = args.msg.flatten();
            zelf.fut_state.store(FutureState::Cancelled);
            Self::schedule_callbacks(&zelf, vm)?;
            Ok(true)
        }

        #[pymethod]
        fn cancelled(&self) -> bool {
            self.fut_state.load() == FutureState::Cancelled
        }

        #[pymethod]
        fn done(&self) -> bool {
            self.fut_state.load() != FutureState::Pending
        }

        #[pymethod]
        fn get_loop(&self, vm: &VirtualMachine) -> PyResult {
            self.fut_loop
                .read()
                .clone()
                .ok_or_else(|| vm.new_runtime_error("Future object is not initialized."))
        }

        #[pymethod]
        fn _make_cancelled_error(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            self.make_cancelled_error_impl(vm)
        }

        fn make_cancelled_error_impl(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            if let Some(exc) = self.fut_cancelled_exc.read().clone()
                && let Ok(exc) = exc.downcast::<PyBaseException>()
            {
                return exc;
            }

            let msg = self.fut_cancel_msg.read().clone();
            let args = if let Some(m) = msg { vec![m] } else { vec![] };

            let exc = match get_cancelled_error_type(vm) {
                Ok(cancelled_error) => vm.new_exception(cancelled_error, args),
                Err(_) => vm.new_runtime_error("cancelled"),
            };
            *self.fut_cancelled_exc.write() = Some(exc.clone().into());
            exc
        }

        fn schedule_callbacks(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // Collect all callbacks first to avoid holding locks during callback execution
            // This prevents deadlock when callbacks access the future's properties
            let mut callbacks_to_call: Vec<(PyObjectRef, Option<PyObjectRef>)> = Vec::new();

            // Take callback0 - release lock before collecting from list
            let cb0 = zelf.fut_callback0.write().take();
            let ctx0 = zelf.fut_context0.write().take();
            if let Some(cb) = cb0 {
                callbacks_to_call.push((cb, ctx0));
            }

            // Take callbacks list and collect items
            let callbacks_list = zelf.fut_callbacks.write().take();
            if let Some(callbacks) = callbacks_list
                && let Ok(list) = callbacks.downcast::<PyList>()
            {
                // Clone the items while holding the list lock, then release
                let items: Vec<_> = list.borrow_vec().iter().cloned().collect();
                for item in items {
                    if let Some(tuple) = item.downcast_ref::<PyTuple>()
                        && let (Some(cb), Some(ctx)) = (tuple.first(), tuple.get(1))
                    {
                        callbacks_to_call.push((cb.clone(), Some(ctx.clone())));
                    }
                }
            }

            // Now call all callbacks without holding any locks
            for (cb, ctx) in callbacks_to_call {
                Self::call_soon_with_context(zelf, cb, ctx, vm)?;
            }

            Ok(())
        }

        fn call_soon_with_context(
            zelf: &PyRef<Self>,
            callback: PyObjectRef,
            context: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let loop_obj = zelf.fut_loop.read().clone();
            if let Some(loop_obj) = loop_obj {
                // call_soon(callback, *args, context=context)
                // callback receives the future as its argument
                let future_arg: PyObjectRef = zelf.clone().into();
                let args = if let Some(ctx) = context {
                    FuncArgs::new(
                        vec![callback, future_arg],
                        KwArgs::new([("context".to_owned(), ctx)].into_iter().collect()),
                    )
                } else {
                    FuncArgs::new(vec![callback, future_arg], KwArgs::default())
                };
                vm.call_method(&loop_obj, "call_soon", args)?;
            }
            Ok(())
        }

        // Properties
        #[pygetset]
        fn _state(&self) -> &'static str {
            self.fut_state.load().as_str()
        }

        #[pygetset]
        fn _asyncio_future_blocking(&self) -> bool {
            self.fut_blocking.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set__asyncio_future_blocking(
            &self,
            value: PySetterValue<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    self.fut_blocking.store(v, Ordering::Relaxed);
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete attribute".to_string()))
                }
            }
        }

        #[pygetset]
        fn _loop(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.fut_loop
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _callbacks(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let mut result = Vec::new();

            if let Some(cb0) = self.fut_callback0.read().clone() {
                let ctx0 = self
                    .fut_context0
                    .read()
                    .clone()
                    .unwrap_or_else(|| vm.ctx.none());
                result.push(vm.ctx.new_tuple(vec![cb0, ctx0]).into());
            }

            if let Some(callbacks) = self.fut_callbacks.read().clone() {
                let list: PyListRef = callbacks.downcast().unwrap();
                for item in list.borrow_vec().iter() {
                    result.push(item.clone());
                }
            }

            // Return None if no callbacks
            if result.is_empty() {
                Ok(vm.ctx.none())
            } else {
                Ok(vm.ctx.new_list(result).into())
            }
        }

        #[pygetset]
        fn _result(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.fut_result
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _exception(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.fut_exception
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _log_traceback(&self) -> bool {
            self.fut_log_tb.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set__log_traceback(
            &self,
            value: PySetterValue<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    if v {
                        return Err(vm.new_value_error(
                            "_log_traceback can only be set to False".to_string(),
                        ));
                    }
                    self.fut_log_tb.store(false, Ordering::Relaxed);
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete attribute".to_string()))
                }
            }
        }

        #[pygetset]
        fn _source_traceback(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.fut_source_tb
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _cancel_message(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.fut_cancel_msg
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset(setter)]
        fn set__cancel_message(&self, value: PySetterValue) {
            match value {
                PySetterValue::Assign(v) => *self.fut_cancel_msg.write() = Some(v),
                PySetterValue::Delete => *self.fut_cancel_msg.write() = None,
            }
        }

        #[pygetset]
        fn _asyncio_awaited_by(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let awaited_by = self.fut_awaited_by.read().clone();
            match awaited_by {
                None => Ok(vm.ctx.none()),
                Some(obj) => {
                    if self.fut_awaited_by_is_set.load(Ordering::Relaxed) {
                        // Already a Set
                        Ok(obj)
                    } else {
                        // Single object - create a Set for the return value
                        let new_set = PySet::default().into_ref(&vm.ctx);
                        new_set.add(obj, vm)?;
                        Ok(new_set.into())
                    }
                }
            }
        }

        /// Add waiter to fut_awaited_by with single-object optimization
        fn awaited_by_add(&self, waiter: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut awaited_by = self.fut_awaited_by.write();
            if awaited_by.is_none() {
                // First waiter - store directly
                *awaited_by = Some(waiter);
                return Ok(());
            }

            if self.fut_awaited_by_is_set.load(Ordering::Relaxed) {
                // Already a Set - add to it
                let set = awaited_by.as_ref().unwrap();
                vm.call_method(set, "add", (waiter,))?;
            } else {
                // Single object - convert to Set
                let existing = awaited_by.take().unwrap();
                let new_set = PySet::default().into_ref(&vm.ctx);
                new_set.add(existing, vm)?;
                new_set.add(waiter, vm)?;
                *awaited_by = Some(new_set.into());
                self.fut_awaited_by_is_set.store(true, Ordering::Relaxed);
            }
            Ok(())
        }

        /// Discard waiter from fut_awaited_by with single-object optimization
        fn awaited_by_discard(&self, waiter: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let mut awaited_by = self.fut_awaited_by.write();
            if awaited_by.is_none() {
                return Ok(());
            }

            let obj = awaited_by.as_ref().unwrap();
            if !self.fut_awaited_by_is_set.load(Ordering::Relaxed) {
                // Single object - check if it matches
                if obj.is(waiter) {
                    *awaited_by = None;
                }
            } else {
                // It's a Set - use discard
                vm.call_method(obj, "discard", (waiter.to_owned(),))?;
            }
            Ok(())
        }

        #[pymethod]
        fn __iter__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyFutureIter> {
            Self::__await__(zelf, vm)
        }

        #[pymethod]
        fn __await__(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyFutureIter> {
            Ok(PyFutureIter {
                future: PyRwLock::new(Some(zelf.into())),
            })
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl Destructor for PyFuture {
        fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // Check if we should log the traceback
            // Don't log if log_tb is false or if the future was cancelled
            if !zelf.fut_log_tb.load(Ordering::Relaxed) {
                return Ok(());
            }

            if zelf.fut_state.load() == FutureState::Cancelled {
                return Ok(());
            }

            let exc = zelf.fut_exception.read().clone();
            let exc = match exc {
                Some(e) => e,
                None => return Ok(()),
            };

            let loop_obj = zelf.fut_loop.read().clone();
            let loop_obj = match loop_obj {
                Some(l) => l,
                None => return Ok(()),
            };

            // Create context dict for call_exception_handler
            let context = PyDict::default().into_ref(&vm.ctx);
            let class_name = zelf.class().name().to_string();
            let message = format!("{} exception was never retrieved", class_name);
            context.set_item(
                vm.ctx.intern_str("message"),
                vm.ctx.new_str(message).into(),
                vm,
            )?;
            context.set_item(vm.ctx.intern_str("exception"), exc, vm)?;
            context.set_item(vm.ctx.intern_str("future"), zelf.to_owned().into(), vm)?;

            if let Some(tb) = zelf.fut_source_tb.read().clone() {
                context.set_item(vm.ctx.intern_str("source_traceback"), tb, vm)?;
            }

            // Call loop.call_exception_handler(context)
            let _ = vm.call_method(&loop_obj, "call_exception_handler", (context,));
            Ok(())
        }
    }

    impl Representable for PyFuture {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let class_name = zelf.class().name().to_string();
            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                let info = get_future_repr_info(zelf.as_object(), vm)?;
                Ok(format!("<{} {}>", class_name, info))
            } else {
                Ok(format!("<{} ...>", class_name))
            }
        }
    }

    impl Iterable for PyFuture {
        fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
            Ok(PyFutureIter {
                future: PyRwLock::new(Some(zelf.into())),
            }
            .into_pyobject(_vm))
        }
    }

    fn get_future_repr_info(future: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
        // Try to use asyncio.base_futures._future_repr_info
        // Import from sys.modules if available, otherwise try regular import
        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        let module =
            if let Ok(m) = sys_modules.get_item(&*vm.ctx.new_str("asyncio.base_futures"), vm) {
                m
            } else {
                // vm.import returns the top-level module, get base_futures submodule
                match vm
                    .import("asyncio.base_futures", 0)
                    .and_then(|asyncio| asyncio.get_attr(vm.ctx.intern_str("base_futures"), vm))
                {
                    Ok(m) => m,
                    Err(_) => return get_future_repr_info_fallback(future, vm),
                }
            };

        let func = match vm.get_attribute_opt(module, vm.ctx.intern_str("_future_repr_info")) {
            Ok(Some(f)) => f,
            _ => return get_future_repr_info_fallback(future, vm),
        };

        let info = match func.call((future.to_owned(),), vm) {
            Ok(i) => i,
            Err(_) => return get_future_repr_info_fallback(future, vm),
        };

        let list: PyListRef = match info.downcast() {
            Ok(l) => l,
            Err(_) => return get_future_repr_info_fallback(future, vm),
        };

        let parts: Vec<String> = list
            .borrow_vec()
            .iter()
            .filter_map(|x: &PyObjectRef| x.str(vm).ok().map(|s| s.as_str().to_string()))
            .collect();
        Ok(parts.join(" "))
    }

    fn get_future_repr_info_fallback(future: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
        // Fallback: build repr from properties directly
        if let Ok(Some(state)) =
            vm.get_attribute_opt(future.to_owned(), vm.ctx.intern_str("_state"))
        {
            let state_str = state
                .str(vm)
                .map(|s| s.as_str().to_lowercase())
                .unwrap_or_else(|_| "unknown".to_string());
            return Ok(state_str);
        }
        Ok("state=unknown".to_string())
    }

    fn get_task_repr_info(task: &PyObject, vm: &VirtualMachine) -> PyResult<String> {
        // vm.import returns the top-level module, get base_tasks submodule
        match vm
            .import("asyncio.base_tasks", 0)
            .and_then(|asyncio| asyncio.get_attr(vm.ctx.intern_str("base_tasks"), vm))
        {
            Ok(base_tasks) => {
                match vm.get_attribute_opt(base_tasks, vm.ctx.intern_str("_task_repr_info")) {
                    Ok(Some(func)) => {
                        let info: PyObjectRef = func.call((task.to_owned(),), vm)?;
                        let list: PyListRef = info.downcast().map_err(|_| {
                            vm.new_type_error("_task_repr_info should return a list")
                        })?;
                        let parts: Vec<String> = list
                            .borrow_vec()
                            .iter()
                            .map(|x: &PyObjectRef| x.str(vm).map(|s| s.as_str().to_string()))
                            .collect::<PyResult<Vec<_>>>()?;
                        Ok(parts.join(" "))
                    }
                    _ => get_future_repr_info(task, vm),
                }
            }
            Err(_) => get_future_repr_info(task, vm),
        }
    }

    #[pyattr]
    #[pyclass(name = "FutureIter", module = "_asyncio", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyFutureIter {
        future: PyRwLock<Option<PyObjectRef>>,
    }

    #[pyclass(with(IterNext, Iterable))]
    impl PyFutureIter {
        #[pymethod]
        fn send(&self, _value: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let future = self.future.read().clone();
            let future = match future {
                Some(f) => f,
                None => return Err(vm.new_stop_iteration(None)),
            };

            // Try to get blocking flag (check Task first since it inherits from Future)
            let blocking = if let Some(task) = future.downcast_ref::<PyTask>() {
                task.base.fut_blocking.load(Ordering::Relaxed)
            } else if let Some(fut) = future.downcast_ref::<PyFuture>() {
                fut.fut_blocking.load(Ordering::Relaxed)
            } else {
                // For non-native futures, check the attribute
                vm.get_attribute_opt(
                    future.clone(),
                    vm.ctx.intern_str("_asyncio_future_blocking"),
                )?
                .map(|v| v.try_to_bool(vm))
                .transpose()?
                .unwrap_or(false)
            };

            // Check if future is done
            let done = vm.call_method(&future, "done", ())?;
            if done.try_to_bool(vm)? {
                *self.future.write() = None;
                let result = vm.call_method(&future, "result", ())?;
                return Err(vm.new_stop_iteration(Some(result)));
            }

            // If still pending and blocking is already set, raise RuntimeError
            // This means await wasn't used with future
            if blocking {
                return Err(vm.new_runtime_error("await wasn't used with future"));
            }

            // First call: set blocking flag and yield the future (check Task first)
            if let Some(task) = future.downcast_ref::<PyTask>() {
                task.base.fut_blocking.store(true, Ordering::Relaxed);
            } else if let Some(fut) = future.downcast_ref::<PyFuture>() {
                fut.fut_blocking.store(true, Ordering::Relaxed);
            } else {
                future.set_attr(
                    vm.ctx.intern_str("_asyncio_future_blocking"),
                    vm.ctx.true_value.clone(),
                    vm,
                )?;
            }
            Ok(future)
        }

        #[pymethod]
        fn throw(
            &self,
            exc_type: PyObjectRef,
            exc_val: OptionalArg,
            exc_tb: OptionalArg,
            vm: &VirtualMachine,
        ) -> PyResult {
            // Warn about deprecated (type, val, tb) signature
            if exc_val.is_present() || exc_tb.is_present() {
                warn::warn(
                    vm.ctx.new_str(
                        "the (type, val, tb) signature of throw() is deprecated, \
                         use throw(val) instead",
                    ),
                    Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                    1,
                    None,
                    vm,
                )?;
            }

            *self.future.write() = None;

            // Validate tb if present
            if let OptionalArg::Present(ref tb) = exc_tb
                && !vm.is_none(tb)
                && !tb.fast_isinstance(vm.ctx.types.traceback_type)
            {
                return Err(vm.new_type_error(format!(
                    "throw() third argument must be a traceback object, not '{}'",
                    tb.class().name()
                )));
            }

            let exc = if exc_type.fast_isinstance(vm.ctx.types.type_type) {
                // exc_type is a class
                let exc_class: PyTypeRef = exc_type.clone().downcast().unwrap();
                // Must be a subclass of BaseException
                if !exc_class.fast_issubclass(vm.ctx.exceptions.base_exception_type) {
                    return Err(vm.new_type_error(
                        "exceptions must be classes or instances deriving from BaseException, not type".to_string()
                    ));
                }

                let val = exc_val.unwrap_or_none(vm);
                if vm.is_none(&val) {
                    exc_type.call((), vm)?
                } else if val.fast_isinstance(&exc_class) {
                    val
                } else {
                    exc_type.call((val,), vm)?
                }
            } else if exc_type.fast_isinstance(vm.ctx.exceptions.base_exception_type) {
                // exc_type is an exception instance
                if let OptionalArg::Present(ref val) = exc_val
                    && !vm.is_none(val)
                {
                    return Err(vm.new_type_error(
                        "instance exception may not have a separate value".to_string(),
                    ));
                }
                exc_type
            } else {
                // exc_type is neither a class nor an exception instance
                return Err(vm.new_type_error(format!(
                    "exceptions must be classes or instances deriving from BaseException, not {}",
                    exc_type.class().name()
                )));
            };

            if let OptionalArg::Present(tb) = exc_tb
                && !vm.is_none(&tb)
            {
                exc.set_attr(vm.ctx.intern_str("__traceback__"), tb, vm)?;
            }

            Err(exc.downcast().unwrap())
        }

        #[pymethod]
        fn close(&self) {
            *self.future.write() = None;
        }
    }

    impl SelfIter for PyFutureIter {}
    impl IterNext for PyFutureIter {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            PyIterReturn::from_pyresult(zelf.send(vm.ctx.none(), vm), vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "Task", module = "_asyncio", base = PyFuture, traverse)]
    #[derive(Debug)]
    #[repr(C)]
    struct PyTask {
        // Base class (must be first field for inheritance)
        base: PyFuture,
        // Task-specific fields
        task_coro: PyRwLock<Option<PyObjectRef>>,
        task_fut_waiter: PyRwLock<Option<PyObjectRef>>,
        task_name: PyRwLock<Option<PyObjectRef>>,
        task_context: PyRwLock<Option<PyObjectRef>>,
        #[pytraverse(skip)]
        task_must_cancel: AtomicBool,
        #[pytraverse(skip)]
        task_num_cancels_requested: AtomicI32,
        #[pytraverse(skip)]
        task_log_destroy_pending: AtomicBool,
    }

    #[derive(FromArgs)]
    struct TaskInitArgs {
        #[pyarg(positional)]
        coro: PyObjectRef,
        #[pyarg(named, name = "loop", optional)]
        loop_: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        name: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        context: OptionalOption<PyObjectRef>,
        #[pyarg(named, optional)]
        eager_start: OptionalOption<bool>,
    }

    static TASK_NAME_COUNTER: AtomicU64 = AtomicU64::new(0);

    impl Constructor for PyTask {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                base: PyFuture::new_empty(),
                task_coro: PyRwLock::new(None),
                task_fut_waiter: PyRwLock::new(None),
                task_name: PyRwLock::new(None),
                task_context: PyRwLock::new(None),
                task_must_cancel: AtomicBool::new(false),
                task_num_cancels_requested: AtomicI32::new(0),
                task_log_destroy_pending: AtomicBool::new(true),
            })
        }
    }

    impl Initializer for PyTask {
        type Args = TaskInitArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            PyTask::py_init(&zelf, args, vm)
        }
    }

    #[pyclass(
        flags(BASETYPE, HAS_DICT),
        with(Constructor, Initializer, Destructor, Representable, Iterable)
    )]
    impl PyTask {
        fn py_init(zelf: &PyRef<Self>, args: TaskInitArgs, vm: &VirtualMachine) -> PyResult<()> {
            // Validate coroutine
            if !is_coroutine(args.coro.clone(), vm)? {
                return Err(vm.new_type_error(format!(
                    "a coroutine was expected, got {}",
                    args.coro.repr(vm)?
                )));
            }

            // Get the event loop
            let loop_obj = match args.loop_.flatten() {
                Some(l) => l,
                None => get_running_loop(vm)
                    .map_err(|_| vm.new_runtime_error("no current event loop"))?,
            };
            *zelf.base.fut_loop.write() = Some(loop_obj.clone());

            // Check if loop has get_debug method and capture source traceback if enabled
            if let Ok(Some(get_debug)) =
                vm.get_attribute_opt(loop_obj.clone(), vm.ctx.intern_str("get_debug"))
                && let Ok(debug) = get_debug.call((), vm)
                && debug.try_to_bool(vm).unwrap_or(false)
            {
                // Get source traceback
                if let Ok(tb_module) = vm.import("traceback", 0)
                    && let Ok(Some(extract_stack)) =
                        vm.get_attribute_opt(tb_module, vm.ctx.intern_str("extract_stack"))
                    && let Ok(tb) = extract_stack.call((), vm)
                {
                    *zelf.base.fut_source_tb.write() = Some(tb);
                }
            }

            // Get or create context
            let context = match args.context.flatten() {
                Some(c) => c,
                None => get_copy_context(vm)?,
            };
            *zelf.task_context.write() = Some(context);

            // Set coroutine
            *zelf.task_coro.write() = Some(args.coro);

            // Set task name
            let name = match args.name.flatten() {
                Some(n) => {
                    if !n.fast_isinstance(vm.ctx.types.str_type) {
                        n.str(vm)?.into()
                    } else {
                        n
                    }
                }
                None => {
                    let counter = TASK_NAME_COUNTER.fetch_add(1, Ordering::SeqCst);
                    vm.ctx.new_str(format!("Task-{}", counter + 1)).into()
                }
            };
            *zelf.task_name.write() = Some(name);

            let eager_start = args.eager_start.flatten().unwrap_or(false);

            // Check if we should do eager start: only if the loop is running
            let do_eager_start = if eager_start {
                let is_running = vm.call_method(&loop_obj, "is_running", ())?;
                is_running.is_true(vm)?
            } else {
                false
            };

            if do_eager_start {
                // Eager start: run first step synchronously (loop is already running)
                task_eager_start(zelf, vm)?;
            } else {
                // Non-eager or loop not running: schedule the first step
                _register_task(zelf.clone().into(), vm)?;
                let task_obj: PyObjectRef = zelf.clone().into();
                let step_wrapper = TaskStepMethWrapper::new(task_obj).into_ref(&vm.ctx);
                vm.call_method(&loop_obj, "call_soon", (step_wrapper,))?;
            }

            Ok(())
        }

        // Future methods delegation
        #[pymethod]
        fn result(&self, vm: &VirtualMachine) -> PyResult {
            match self.base.fut_state.load() {
                FutureState::Pending => Err(new_invalid_state_error(vm, "Result is not ready.")),
                FutureState::Cancelled => Err(self.make_cancelled_error_impl(vm)),
                FutureState::Finished => {
                    self.base.fut_log_tb.store(false, Ordering::Relaxed);
                    if let Some(exc) = self.base.fut_exception.read().clone() {
                        let exc: PyBaseExceptionRef = exc.downcast().unwrap();
                        // Restore the original traceback to prevent traceback accumulation
                        if let Some(tb) = self.base.fut_exception_tb.read().clone() {
                            let _ = exc.set___traceback__(tb, vm);
                        }
                        Err(exc)
                    } else {
                        Ok(self
                            .base
                            .fut_result
                            .read()
                            .clone()
                            .unwrap_or_else(|| vm.ctx.none()))
                    }
                }
            }
        }

        #[pymethod]
        fn exception(&self, vm: &VirtualMachine) -> PyResult {
            match self.base.fut_state.load() {
                FutureState::Pending => Err(new_invalid_state_error(vm, "Exception is not set.")),
                FutureState::Cancelled => Err(self.make_cancelled_error_impl(vm)),
                FutureState::Finished => {
                    self.base.fut_log_tb.store(false, Ordering::Relaxed);
                    Ok(self
                        .base
                        .fut_exception
                        .read()
                        .clone()
                        .unwrap_or_else(|| vm.ctx.none()))
                }
            }
        }

        #[pymethod]
        fn set_result(
            _zelf: PyObjectRef,
            _result: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            Err(vm.new_runtime_error("Task does not support set_result operation"))
        }

        #[pymethod]
        fn set_exception(&self, _exception: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_runtime_error("Task does not support set_exception operation"))
        }

        fn make_cancelled_error_impl(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            if let Some(exc) = self.base.fut_cancelled_exc.read().clone()
                && let Ok(exc) = exc.downcast::<PyBaseException>()
            {
                return exc;
            }

            let msg = self.base.fut_cancel_msg.read().clone();
            let args = if let Some(m) = msg { vec![m] } else { vec![] };

            let exc = match get_cancelled_error_type(vm) {
                Ok(cancelled_error) => vm.new_exception(cancelled_error, args),
                Err(_) => vm.new_runtime_error("cancelled"),
            };
            *self.base.fut_cancelled_exc.write() = Some(exc.clone().into());
            exc
        }

        #[pymethod]
        fn add_done_callback(
            zelf: PyRef<Self>,
            args: AddDoneCallbackArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if zelf.base.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            let ctx = match args.context.flatten() {
                Some(c) => c,
                None => get_copy_context(vm)?,
            };

            if zelf.base.fut_state.load() != FutureState::Pending {
                Self::call_soon_with_context(&zelf, args.func, Some(ctx), vm)?;
            } else if zelf.base.fut_callback0.read().is_none() {
                *zelf.base.fut_callback0.write() = Some(args.func);
                *zelf.base.fut_context0.write() = Some(ctx);
            } else {
                let tuple = vm.ctx.new_tuple(vec![args.func, ctx]);
                let mut callbacks = zelf.base.fut_callbacks.write();
                if callbacks.is_none() {
                    *callbacks = Some(vm.ctx.new_list(vec![tuple.into()]).into());
                } else {
                    let list = callbacks.as_ref().unwrap();
                    vm.call_method(list, "append", (tuple,))?;
                }
            }
            Ok(())
        }

        #[pymethod]
        fn remove_done_callback(&self, func: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            if self.base.fut_loop.read().is_none() {
                return Err(vm.new_runtime_error("Future object is not initialized.".to_string()));
            }
            let mut cleared_callback0 = 0usize;

            // Check fut_callback0 first
            // Clone to release lock before comparison (which may run Python code)
            let cb0 = self.base.fut_callback0.read().clone();
            if let Some(cb0) = cb0 {
                let cmp = vm.identical_or_equal(&cb0, &func)?;
                if cmp {
                    *self.base.fut_callback0.write() = None;
                    *self.base.fut_context0.write() = None;
                    cleared_callback0 = 1;
                }
            }

            // Check if fut_callbacks exists
            let callbacks = self.base.fut_callbacks.read().clone();
            let callbacks = match callbacks {
                Some(c) => c,
                None => return Ok(cleared_callback0),
            };

            let list: PyListRef = callbacks.downcast().unwrap();
            let len = list.borrow_vec().len();

            if len == 0 {
                *self.base.fut_callbacks.write() = None;
                return Ok(cleared_callback0);
            }

            // Special case for single callback
            if len == 1 {
                let item = list.borrow_vec().first().cloned();
                if let Some(item) = item {
                    let tuple: &PyTuple = item.downcast_ref().unwrap();
                    let cb = tuple.first().unwrap().clone();
                    let cmp = vm.identical_or_equal(&cb, &func)?;
                    if cmp {
                        *self.base.fut_callbacks.write() = None;
                        return Ok(1 + cleared_callback0);
                    }
                }
                return Ok(cleared_callback0);
            }

            // Multiple callbacks - iterate with index, checking validity each time
            // to handle evil comparisons
            let mut new_callbacks = Vec::with_capacity(len);
            let mut i = 0usize;
            let mut removed = 0usize;

            loop {
                // Re-check fut_callbacks on each iteration (evil code may have cleared it)
                let callbacks = self.base.fut_callbacks.read().clone();
                let callbacks = match callbacks {
                    Some(c) => c,
                    None => break,
                };
                let list: PyListRef = callbacks.downcast().unwrap();
                let current_len = list.borrow_vec().len();
                if i >= current_len {
                    break;
                }

                // Get item and release lock before comparison
                let item = list.borrow_vec().get(i).cloned();
                let item = match item {
                    Some(item) => item,
                    None => break,
                };

                let tuple: &PyTuple = item.downcast_ref().unwrap();
                let cb = tuple.first().unwrap().clone();
                let cmp = vm.identical_or_equal(&cb, &func)?;

                if !cmp {
                    new_callbacks.push(item);
                } else {
                    removed += 1;
                }
                i += 1;
            }

            // Update fut_callbacks with filtered list
            if new_callbacks.is_empty() {
                *self.base.fut_callbacks.write() = None;
            } else {
                *self.base.fut_callbacks.write() = Some(vm.ctx.new_list(new_callbacks).into());
            }

            Ok(removed + cleared_callback0)
        }

        fn schedule_callbacks(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
            // Collect all callbacks first to avoid holding locks during callback execution
            // This prevents deadlock when callbacks access the future's properties
            let mut callbacks_to_call: Vec<(PyObjectRef, Option<PyObjectRef>)> = Vec::new();

            // Take callback0 - release lock before collecting from list
            let cb0 = zelf.base.fut_callback0.write().take();
            let ctx0 = zelf.base.fut_context0.write().take();
            if let Some(cb) = cb0 {
                callbacks_to_call.push((cb, ctx0));
            }

            // Take callbacks list and collect items
            let callbacks_list = zelf.base.fut_callbacks.write().take();
            if let Some(callbacks) = callbacks_list
                && let Ok(list) = callbacks.downcast::<PyList>()
            {
                // Clone the items while holding the list lock, then release
                let items: Vec<_> = list.borrow_vec().iter().cloned().collect();
                for item in items {
                    if let Some(tuple) = item.downcast_ref::<PyTuple>()
                        && let (Some(cb), Some(ctx)) = (tuple.first(), tuple.get(1))
                    {
                        callbacks_to_call.push((cb.clone(), Some(ctx.clone())));
                    }
                }
            }

            // Now call all callbacks without holding any locks
            for (cb, ctx) in callbacks_to_call {
                Self::call_soon_with_context(zelf, cb, ctx, vm)?;
            }

            Ok(())
        }

        fn call_soon_with_context(
            zelf: &PyRef<Self>,
            callback: PyObjectRef,
            context: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let loop_obj = zelf.base.fut_loop.read().clone();
            if let Some(loop_obj) = loop_obj {
                // call_soon(callback, *args, context=context)
                // callback receives the task as its argument
                let task_arg: PyObjectRef = zelf.clone().into();
                let args = if let Some(ctx) = context {
                    FuncArgs::new(
                        vec![callback, task_arg],
                        KwArgs::new([("context".to_owned(), ctx)].into_iter().collect()),
                    )
                } else {
                    FuncArgs::new(vec![callback, task_arg], KwArgs::default())
                };
                vm.call_method(&loop_obj, "call_soon", args)?;
            }
            Ok(())
        }

        #[pymethod]
        fn cancel(&self, args: CancelArgs, vm: &VirtualMachine) -> PyResult<bool> {
            if self.base.fut_state.load() != FutureState::Pending {
                // Clear log_tb even when cancel fails (task is already done)
                self.base.fut_log_tb.store(false, Ordering::Relaxed);
                return Ok(false);
            }

            self.task_num_cancels_requested
                .fetch_add(1, Ordering::SeqCst);

            let msg_value = args.msg.flatten();

            if let Some(fut_waiter) = self.task_fut_waiter.read().clone() {
                // Call cancel with msg=msg keyword argument
                let cancel_args = if let Some(ref m) = msg_value {
                    FuncArgs::new(
                        vec![],
                        KwArgs::new([("msg".to_owned(), m.clone())].into_iter().collect()),
                    )
                } else {
                    FuncArgs::new(vec![], KwArgs::default())
                };
                let cancel_result = vm.call_method(&fut_waiter, "cancel", cancel_args)?;
                if cancel_result.try_to_bool(vm)? {
                    return Ok(true);
                }
            }

            self.task_must_cancel.store(true, Ordering::Relaxed);
            *self.base.fut_cancel_msg.write() = msg_value;
            Ok(true)
        }

        #[pymethod]
        fn cancelled(&self) -> bool {
            self.base.fut_state.load() == FutureState::Cancelled
        }

        #[pymethod]
        fn done(&self) -> bool {
            self.base.fut_state.load() != FutureState::Pending
        }

        #[pymethod]
        fn cancelling(&self) -> i32 {
            self.task_num_cancels_requested.load(Ordering::SeqCst)
        }

        #[pymethod]
        fn uncancel(&self) -> i32 {
            let prev = self
                .task_num_cancels_requested
                .fetch_sub(1, Ordering::SeqCst);
            if prev <= 0 {
                self.task_num_cancels_requested.store(0, Ordering::SeqCst);
                0
            } else {
                let new_val = prev - 1;
                // When cancelling count reaches 0, reset _must_cancel
                if new_val == 0 {
                    self.task_must_cancel.store(false, Ordering::SeqCst);
                }
                new_val
            }
        }

        #[pymethod]
        fn get_coro(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task_coro
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pymethod]
        fn get_context(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task_context
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pymethod]
        fn get_name(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task_name
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pymethod]
        fn set_name(&self, name: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let name = if !name.fast_isinstance(vm.ctx.types.str_type) {
                name.str(vm)?.into()
            } else {
                name
            };
            *self.task_name.write() = Some(name);
            Ok(())
        }

        #[pymethod]
        fn get_loop(&self, vm: &VirtualMachine) -> PyResult {
            self.base
                .fut_loop
                .read()
                .clone()
                .ok_or_else(|| vm.new_runtime_error("Task object is not initialized."))
        }

        #[pymethod]
        fn get_stack(zelf: PyRef<Self>, args: GetStackArgs, vm: &VirtualMachine) -> PyResult {
            let limit = args.limit.flatten().unwrap_or_else(|| vm.ctx.none());
            // vm.import returns the top-level module, get base_tasks submodule
            let asyncio = vm.import("asyncio.base_tasks", 0)?;
            let base_tasks = asyncio.get_attr(vm.ctx.intern_str("base_tasks"), vm)?;
            let get_stack_func = base_tasks.get_attr(vm.ctx.intern_str("_task_get_stack"), vm)?;
            get_stack_func.call((zelf, limit), vm)
        }

        #[pymethod]
        fn print_stack(
            zelf: PyRef<Self>,
            args: PrintStackArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let limit = args.limit.flatten().unwrap_or_else(|| vm.ctx.none());
            let file = args.file.flatten().unwrap_or_else(|| vm.ctx.none());
            // vm.import returns the top-level module, get base_tasks submodule
            let asyncio = vm.import("asyncio.base_tasks", 0)?;
            let base_tasks = asyncio.get_attr(vm.ctx.intern_str("base_tasks"), vm)?;
            let print_stack_func =
                base_tasks.get_attr(vm.ctx.intern_str("_task_print_stack"), vm)?;
            print_stack_func.call((zelf, limit, file), vm)?;
            Ok(())
        }

        #[pymethod]
        fn _make_cancelled_error(&self, vm: &VirtualMachine) -> PyBaseExceptionRef {
            self.make_cancelled_error_impl(vm)
        }

        // Properties
        #[pygetset]
        fn _state(&self) -> &'static str {
            self.base.fut_state.load().as_str()
        }

        #[pygetset]
        fn _asyncio_future_blocking(&self) -> bool {
            self.base.fut_blocking.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set__asyncio_future_blocking(
            &self,
            value: PySetterValue<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    self.base.fut_blocking.store(v, Ordering::Relaxed);
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete attribute".to_string()))
                }
            }
        }

        #[pygetset]
        fn _loop(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base
                .fut_loop
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _log_destroy_pending(&self) -> bool {
            self.task_log_destroy_pending.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set__log_destroy_pending(
            &self,
            value: PySetterValue<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    self.task_log_destroy_pending.store(v, Ordering::Relaxed);
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("can't delete _log_destroy_pending".to_owned()))
                }
            }
        }

        #[pygetset]
        fn _log_traceback(&self) -> bool {
            self.base.fut_log_tb.load(Ordering::Relaxed)
        }

        #[pygetset(setter)]
        fn set__log_traceback(
            &self,
            value: PySetterValue<bool>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    if v {
                        return Err(vm.new_value_error(
                            "_log_traceback can only be set to False".to_string(),
                        ));
                    }
                    self.base.fut_log_tb.store(false, Ordering::Relaxed);
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_attribute_error("cannot delete attribute".to_string()))
                }
            }
        }

        #[pygetset]
        fn _must_cancel(&self) -> bool {
            self.task_must_cancel.load(Ordering::Relaxed)
        }

        #[pygetset]
        fn _coro(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task_coro
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _fut_waiter(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task_fut_waiter
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _source_traceback(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base
                .fut_source_tb
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _result(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base
                .fut_result
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _exception(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base
                .fut_exception
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn _cancel_message(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.base
                .fut_cancel_msg
                .read()
                .clone()
                .unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset(setter)]
        fn set__cancel_message(&self, value: PySetterValue) {
            match value {
                PySetterValue::Assign(v) => *self.base.fut_cancel_msg.write() = Some(v),
                PySetterValue::Delete => *self.base.fut_cancel_msg.write() = None,
            }
        }

        #[pygetset]
        fn _callbacks(&self, vm: &VirtualMachine) -> PyObjectRef {
            let mut result: Vec<PyObjectRef> = Vec::new();
            if let Some(cb) = self.base.fut_callback0.read().clone() {
                let ctx = self
                    .base
                    .fut_context0
                    .read()
                    .clone()
                    .unwrap_or_else(|| vm.ctx.none());
                result.push(vm.ctx.new_tuple(vec![cb, ctx]).into());
            }
            if let Some(callbacks) = self.base.fut_callbacks.read().clone()
                && let Ok(list) = callbacks.downcast::<PyList>()
            {
                for item in list.borrow_vec().iter() {
                    result.push(item.clone());
                }
            }
            // Return None if no callbacks
            if result.is_empty() {
                vm.ctx.none()
            } else {
                vm.ctx.new_list(result).into()
            }
        }

        #[pymethod]
        fn __iter__(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyFutureIter> {
            Ok(PyFutureIter {
                future: PyRwLock::new(Some(zelf.into())),
            })
        }

        #[pymethod]
        fn __await__(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult<PyFutureIter> {
            Ok(PyFutureIter {
                future: PyRwLock::new(Some(zelf.into())),
            })
        }

        #[pyclassmethod]
        fn __class_getitem__(
            cls: PyTypeRef,
            args: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyGenericAlias {
            PyGenericAlias::from_args(cls, args, vm)
        }
    }

    impl Destructor for PyTask {
        fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let loop_obj = zelf.base.fut_loop.read().clone();

            // Check if task is pending and log_destroy_pending is True
            if zelf.base.fut_state.load() == FutureState::Pending
                && zelf.task_log_destroy_pending.load(Ordering::Relaxed)
            {
                if let Some(loop_obj) = loop_obj.clone() {
                    let context = PyDict::default().into_ref(&vm.ctx);
                    let task_repr = zelf
                        .as_object()
                        .repr(vm)
                        .unwrap_or_else(|_| vm.ctx.new_str("<Task>"));
                    let message =
                        format!("Task was destroyed but it is pending!\ntask: {}", task_repr);
                    context.set_item(
                        vm.ctx.intern_str("message"),
                        vm.ctx.new_str(message).into(),
                        vm,
                    )?;
                    context.set_item(vm.ctx.intern_str("task"), zelf.to_owned().into(), vm)?;

                    if let Some(tb) = zelf.base.fut_source_tb.read().clone() {
                        context.set_item(vm.ctx.intern_str("source_traceback"), tb, vm)?;
                    }

                    let _ = vm.call_method(&loop_obj, "call_exception_handler", (context,));
                }
                return Ok(());
            }

            // Check if we should log the traceback for exception
            if !zelf.base.fut_log_tb.load(Ordering::Relaxed) {
                return Ok(());
            }

            let exc = zelf.base.fut_exception.read().clone();
            let exc = match exc {
                Some(e) => e,
                None => return Ok(()),
            };

            let loop_obj = match loop_obj {
                Some(l) => l,
                None => return Ok(()),
            };

            // Create context dict for call_exception_handler
            let context = PyDict::default().into_ref(&vm.ctx);
            let class_name = zelf.class().name().to_string();
            let message = format!("{} exception was never retrieved", class_name);
            context.set_item(
                vm.ctx.intern_str("message"),
                vm.ctx.new_str(message).into(),
                vm,
            )?;
            context.set_item(vm.ctx.intern_str("exception"), exc, vm)?;
            context.set_item(vm.ctx.intern_str("future"), zelf.to_owned().into(), vm)?;

            if let Some(tb) = zelf.base.fut_source_tb.read().clone() {
                context.set_item(vm.ctx.intern_str("source_traceback"), tb, vm)?;
            }

            // Call loop.call_exception_handler(context)
            let _ = vm.call_method(&loop_obj, "call_exception_handler", (context,));
            Ok(())
        }
    }

    impl Representable for PyTask {
        fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let class_name = zelf.class().name().to_string();

            if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
                // Try to use _task_repr_info if available
                if let Ok(info) = get_task_repr_info(zelf.as_object(), vm)
                    && info != "state=unknown"
                {
                    return Ok(format!("<{} {}>", class_name, info));
                }

                // Fallback: build repr from task properties directly
                let state = zelf.base.fut_state.load().as_str().to_lowercase();
                let name = zelf
                    .task_name
                    .read()
                    .as_ref()
                    .and_then(|n| n.str(vm).ok())
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "?".to_string());
                let coro_repr = zelf
                    .task_coro
                    .read()
                    .as_ref()
                    .and_then(|c| c.repr(vm).ok())
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_else(|| "?".to_string());

                Ok(format!(
                    "<{} {} name='{}' coro={}>",
                    class_name, state, name, coro_repr
                ))
            } else {
                Ok(format!("<{} ...>", class_name))
            }
        }
    }

    impl Iterable for PyTask {
        fn iter(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyResult {
            Ok(PyFutureIter {
                future: PyRwLock::new(Some(zelf.into())),
            }
            .into_pyobject(_vm))
        }
    }

    /// Eager start: run first step synchronously
    fn task_eager_start(zelf: &PyRef<PyTask>, vm: &VirtualMachine) -> PyResult<()> {
        let loop_obj = zelf.base.fut_loop.read().clone();
        let loop_obj = match loop_obj {
            Some(l) => l,
            None => return Err(vm.new_runtime_error("Task has no loop")),
        };

        // Register task before running step
        let task_obj: PyObjectRef = zelf.clone().into();
        _register_task(task_obj.clone(), vm)?;

        // Register as eager task
        _register_eager_task(task_obj.clone(), vm)?;

        // Swap current task - save previous task
        let prev_task = _swap_current_task(loop_obj.clone(), task_obj.clone(), vm)?;

        // Get coro and context
        let coro = zelf.task_coro.read().clone();
        let context = zelf.task_context.read().clone();

        // Run the first step with context (using context.run(callable, *args))
        let step_result = if let Some(ctx) = context {
            // Call context.run(coro.send, None)
            let coro_ref = match coro {
                Some(c) => c,
                None => {
                    let _ = _swap_current_task(loop_obj.clone(), prev_task, vm);
                    _unregister_eager_task(task_obj.clone(), vm)?;
                    return Ok(());
                }
            };
            let send_method = coro_ref.get_attr(vm.ctx.intern_str("send"), vm)?;
            vm.call_method(&ctx, "run", (send_method, vm.ctx.none()))
        } else {
            // Run without context
            match coro {
                Some(c) => vm.call_method(&c, "send", (vm.ctx.none(),)),
                None => {
                    let _ = _swap_current_task(loop_obj.clone(), prev_task, vm);
                    _unregister_eager_task(task_obj.clone(), vm)?;
                    return Ok(());
                }
            }
        };

        // Restore previous task
        let _ = _swap_current_task(loop_obj.clone(), prev_task, vm);

        // Unregister from eager tasks
        _unregister_eager_task(task_obj.clone(), vm)?;

        // Handle the result
        match step_result {
            Ok(result) => {
                task_step_handle_result(zelf, result, vm)?;
            }
            Err(e) => {
                task_step_handle_exception(zelf, e, vm)?;
            }
        }

        // If task is no longer pending, clear the coroutine
        if zelf.base.fut_state.load() != FutureState::Pending {
            *zelf.task_coro.write() = None;
        }

        Ok(())
    }

    /// Task step implementation
    fn task_step_impl(
        task: &PyObjectRef,
        exc: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let task_ref: PyRef<PyTask> = task
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("task_step called with non-Task object"))?;

        if task_ref.base.fut_state.load() != FutureState::Pending {
            // Task is already done - report InvalidStateError via exception handler
            let loop_obj = task_ref.base.fut_loop.read().clone();
            if let Some(loop_obj) = loop_obj {
                let exc = new_invalid_state_error(vm, "step(): already done");
                let context = vm.ctx.new_dict();
                context.set_item("message", vm.new_pyobj("step(): already done"), vm)?;
                context.set_item("exception", exc.clone().into(), vm)?;
                context.set_item("task", task.clone(), vm)?;
                let _ = vm.call_method(&loop_obj, "call_exception_handler", (context,));
            }
            return Ok(vm.ctx.none());
        }

        *task_ref.task_fut_waiter.write() = None;

        let coro = task_ref.task_coro.read().clone();
        let coro = match coro {
            Some(c) => c,
            None => return Ok(vm.ctx.none()),
        };

        // Get event loop for enter/leave task
        let loop_obj = task_ref.base.fut_loop.read().clone();
        let loop_obj = match loop_obj {
            Some(l) => l,
            None => return Ok(vm.ctx.none()),
        };

        // Get task context
        let context = task_ref.task_context.read().clone();

        // Enter task - register as current task
        _enter_task(loop_obj.clone(), task.clone(), vm)?;

        // Determine the exception to throw (if any)
        // If task_must_cancel is set and exc is None or not CancelledError, create CancelledError
        let exc_to_throw = if task_ref.task_must_cancel.load(Ordering::Relaxed) {
            task_ref.task_must_cancel.store(false, Ordering::Relaxed);
            if let Some(ref e) = exc {
                if is_cancelled_error_obj(e, vm) {
                    exc.clone()
                } else {
                    Some(task_ref.make_cancelled_error_impl(vm).into())
                }
            } else {
                Some(task_ref.make_cancelled_error_impl(vm).into())
            }
        } else {
            exc
        };

        // Run coroutine step within task's context
        let result = if let Some(ctx) = context {
            // Use context.run(callable, *args) to run within the task's context
            if let Some(ref exc_obj) = exc_to_throw {
                let throw_method = coro.get_attr(vm.ctx.intern_str("throw"), vm)?;
                vm.call_method(&ctx, "run", (throw_method, exc_obj.clone()))
            } else {
                let send_method = coro.get_attr(vm.ctx.intern_str("send"), vm)?;
                vm.call_method(&ctx, "run", (send_method, vm.ctx.none()))
            }
        } else {
            // Fallback: run without context
            if let Some(ref exc_obj) = exc_to_throw {
                vm.call_method(&coro, "throw", (exc_obj.clone(),))
            } else {
                vm.call_method(&coro, "send", (vm.ctx.none(),))
            }
        };

        // Leave task - unregister as current task (must happen even on error)
        let _ = _leave_task(loop_obj, task.clone(), vm);

        match result {
            Ok(result) => {
                task_step_handle_result(&task_ref, result, vm)?;
            }
            Err(e) => {
                task_step_handle_exception(&task_ref, e, vm)?;
            }
        }

        Ok(vm.ctx.none())
    }

    fn task_step_handle_result(
        task: &PyRef<PyTask>,
        result: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check if task awaits on itself
        let task_obj: PyObjectRef = task.clone().into();
        if result.is(&task_obj) {
            let msg = format!(
                "Task cannot await on itself: {}",
                task_obj.repr(vm)?.as_str()
            );
            task.base.fut_state.store(FutureState::Finished);
            *task.base.fut_exception.write() = Some(vm.new_runtime_error(msg).into());
            PyTask::schedule_callbacks(task, vm)?;
            _unregister_task(task_obj, vm)?;
            return Ok(());
        }

        let blocking = vm
            .get_attribute_opt(
                result.clone(),
                vm.ctx.intern_str("_asyncio_future_blocking"),
            )?
            .and_then(|v| v.try_to_bool(vm).ok())
            .unwrap_or(false);

        if blocking {
            result.set_attr(
                vm.ctx.intern_str("_asyncio_future_blocking"),
                vm.ctx.new_bool(false),
                vm,
            )?;

            // Get the future's loop, similar to get_future_loop:
            // 1. If it's our native Future/Task, access fut_loop directly (check Task first)
            // 2. Otherwise try get_loop(), falling back to _loop on AttributeError
            let fut_loop = if let Ok(task) = result.clone().downcast::<PyTask>() {
                task.base
                    .fut_loop
                    .read()
                    .clone()
                    .unwrap_or_else(|| vm.ctx.none())
            } else if let Ok(fut) = result.clone().downcast::<PyFuture>() {
                fut.fut_loop.read().clone().unwrap_or_else(|| vm.ctx.none())
            } else {
                // Try get_loop(), fall back to _loop on AttributeError
                match vm.call_method(&result, "get_loop", ()) {
                    Ok(loop_obj) => loop_obj,
                    Err(e) if e.fast_isinstance(vm.ctx.exceptions.attribute_error) => {
                        result.get_attr(vm.ctx.intern_str("_loop"), vm)?
                    }
                    Err(e) => return Err(e),
                }
            };
            let task_loop = task.base.fut_loop.read().clone();
            if let Some(task_loop) = task_loop
                && !fut_loop.is(&task_loop)
            {
                let task_repr = task
                    .as_object()
                    .repr(vm)
                    .unwrap_or_else(|_| vm.ctx.new_str("<Task>"));
                let result_repr = result
                    .repr(vm)
                    .unwrap_or_else(|_| vm.ctx.new_str("<Future>"));
                let msg = format!(
                    "Task {} got Future {} attached to a different loop",
                    task_repr, result_repr
                );
                task.base.fut_state.store(FutureState::Finished);
                *task.base.fut_exception.write() = Some(vm.new_runtime_error(msg).into());
                PyTask::schedule_callbacks(task, vm)?;
                _unregister_task(task.clone().into(), vm)?;
                return Ok(());
            }

            *task.task_fut_waiter.write() = Some(result.clone());

            let task_obj: PyObjectRef = task.clone().into();
            let wakeup_wrapper = TaskWakeupMethWrapper::new(task_obj.clone()).into_ref(&vm.ctx);
            vm.call_method(&result, "add_done_callback", (wakeup_wrapper,))?;

            // Track awaited_by relationship for introspection
            future_add_to_awaited_by(result.clone(), task_obj, vm)?;

            // If task_must_cancel is set, cancel the awaited future immediately
            // This propagates the cancellation through the future chain
            if task.task_must_cancel.load(Ordering::Relaxed) {
                let cancel_msg = task.base.fut_cancel_msg.read().clone();
                let cancel_args = if let Some(ref m) = cancel_msg {
                    FuncArgs::new(
                        vec![],
                        KwArgs::new([("msg".to_owned(), m.clone())].into_iter().collect()),
                    )
                } else {
                    FuncArgs::new(vec![], KwArgs::default())
                };
                let cancel_result = vm.call_method(&result, "cancel", cancel_args)?;
                if cancel_result.try_to_bool(vm).unwrap_or(false) {
                    task.task_must_cancel.store(false, Ordering::Relaxed);
                }
            }
        } else if vm.is_none(&result) {
            let loop_obj = task.base.fut_loop.read().clone();
            if let Some(loop_obj) = loop_obj {
                let task_obj: PyObjectRef = task.clone().into();
                let step_wrapper = TaskStepMethWrapper::new(task_obj).into_ref(&vm.ctx);
                vm.call_method(&loop_obj, "call_soon", (step_wrapper,))?;
            }
        } else {
            let msg = format!("Task got bad yield: {}", result.repr(vm)?.as_str());
            task.base.fut_state.store(FutureState::Finished);
            *task.base.fut_exception.write() = Some(vm.new_runtime_error(msg).into());
            PyTask::schedule_callbacks(task, vm)?;
            _unregister_task(task.clone().into(), vm)?;
        }

        Ok(())
    }

    fn task_step_handle_exception(
        task: &PyRef<PyTask>,
        exc: PyBaseExceptionRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Check for KeyboardInterrupt or SystemExit - these should be re-raised
        let should_reraise = exc.fast_isinstance(vm.ctx.exceptions.keyboard_interrupt)
            || exc.fast_isinstance(vm.ctx.exceptions.system_exit);

        if exc.fast_isinstance(vm.ctx.exceptions.stop_iteration) {
            // Check if task was cancelled while running
            if task.task_must_cancel.load(Ordering::Relaxed) {
                // Task was cancelled - treat as cancelled instead of result
                task.task_must_cancel.store(false, Ordering::Relaxed);
                let cancelled_exc = task.base.make_cancelled_error_impl(vm);
                task.base.fut_state.store(FutureState::Cancelled);
                *task.base.fut_cancelled_exc.write() = Some(cancelled_exc.into());
            } else {
                let result = exc.get_arg(0).unwrap_or_else(|| vm.ctx.none());
                task.base.fut_state.store(FutureState::Finished);
                *task.base.fut_result.write() = Some(result);
            }
            PyTask::schedule_callbacks(task, vm)?;
            _unregister_task(task.clone().into(), vm)?;
        } else if is_cancelled_error(&exc, vm) {
            task.base.fut_state.store(FutureState::Cancelled);
            *task.base.fut_cancelled_exc.write() = Some(exc.clone().into());
            PyTask::schedule_callbacks(task, vm)?;
            _unregister_task(task.clone().into(), vm)?;
        } else {
            task.base.fut_state.store(FutureState::Finished);
            // Save the original traceback for later restoration
            let tb = exc.__traceback__().map(|tb| tb.into());
            *task.base.fut_exception_tb.write() = tb;
            *task.base.fut_exception.write() = Some(exc.clone().into());
            task.base.fut_log_tb.store(true, Ordering::Relaxed);
            PyTask::schedule_callbacks(task, vm)?;
            _unregister_task(task.clone().into(), vm)?;
        }

        // Re-raise KeyboardInterrupt and SystemExit after storing in task
        if should_reraise {
            return Err(exc);
        }

        Ok(())
    }

    fn task_wakeup_impl(task: &PyObjectRef, fut: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let task_ref: PyRef<PyTask> = task
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("task_wakeup called with non-Task object"))?;

        // Remove awaited_by relationship before resuming
        future_discard_from_awaited_by(fut.clone(), task.clone(), vm)?;

        *task_ref.task_fut_waiter.write() = None;

        // Call result() on the awaited future to get either result or exception
        // If result() raises an exception (like CancelledError), pass it to task_step
        let exc = match vm.call_method(fut, "result", ()) {
            Ok(_) => None,
            Err(e) => Some(e.into()),
        };

        // Call task_step directly instead of using call_soon
        // This allows the awaiting task to continue in the same event loop iteration
        task_step_impl(task, exc, vm)
    }

    // Module Functions

    fn get_all_tasks_set(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Use the module-level _scheduled_tasks WeakSet
        let asyncio_module = vm.import("_asyncio", 0)?;
        vm.get_attribute_opt(asyncio_module, vm.ctx.intern_str("_scheduled_tasks"))?
            .ok_or_else(|| vm.new_attribute_error("_scheduled_tasks not found"))
    }

    fn get_eager_tasks_set(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Use the module-level _eager_tasks Set
        let asyncio_module = vm.import("_asyncio", 0)?;
        vm.get_attribute_opt(asyncio_module, vm.ctx.intern_str("_eager_tasks"))?
            .ok_or_else(|| vm.new_attribute_error("_eager_tasks not found"))
    }

    fn get_current_tasks_dict(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Use the module-level _current_tasks Dict
        let asyncio_module = vm.import("_asyncio", 0)?;
        vm.get_attribute_opt(asyncio_module, vm.ctx.intern_str("_current_tasks"))?
            .ok_or_else(|| vm.new_attribute_error("_current_tasks not found"))
    }

    #[pyfunction]
    fn _get_running_loop(vm: &VirtualMachine) -> PyObjectRef {
        vm.asyncio_running_loop
            .borrow()
            .clone()
            .unwrap_or_else(|| vm.ctx.none())
    }

    #[pyfunction]
    fn _set_running_loop(loop_: OptionalOption<PyObjectRef>, vm: &VirtualMachine) {
        *vm.asyncio_running_loop.borrow_mut() = loop_.flatten();
    }

    #[pyfunction]
    fn get_running_loop(vm: &VirtualMachine) -> PyResult {
        vm.asyncio_running_loop
            .borrow()
            .clone()
            .ok_or_else(|| vm.new_runtime_error("no running event loop"))
    }

    #[pyfunction]
    fn get_event_loop(vm: &VirtualMachine) -> PyResult {
        if let Some(loop_) = vm.asyncio_running_loop.borrow().clone() {
            return Ok(loop_);
        }

        let asyncio_events = vm.import("asyncio.events", 0)?;
        let get_event_loop_policy = vm
            .get_attribute_opt(asyncio_events, vm.ctx.intern_str("get_event_loop_policy"))?
            .ok_or_else(|| vm.new_attribute_error("get_event_loop_policy"))?;
        let policy = get_event_loop_policy.call((), vm)?;
        let get_event_loop = vm
            .get_attribute_opt(policy, vm.ctx.intern_str("get_event_loop"))?
            .ok_or_else(|| vm.new_attribute_error("get_event_loop"))?;
        get_event_loop.call((), vm)
    }

    #[pyfunction]
    fn current_task(args: LoopArg, vm: &VirtualMachine) -> PyResult {
        let loop_obj = match args.loop_.flatten() {
            Some(l) if !vm.is_none(&l) => l,
            _ => {
                // When loop is None or not provided, use the running loop
                match vm.asyncio_running_loop.borrow().clone() {
                    Some(l) => l,
                    None => return Err(vm.new_runtime_error("no running event loop")),
                }
            }
        };

        // Fast path: if the loop is the current thread's running loop,
        // return the per-thread running task directly
        let is_current_loop = vm
            .asyncio_running_loop
            .borrow()
            .as_ref()
            .is_some_and(|rl| rl.is(&loop_obj));

        if is_current_loop {
            return Ok(vm
                .asyncio_running_task
                .borrow()
                .clone()
                .unwrap_or_else(|| vm.ctx.none()));
        }

        // Slow path: look up in the module-level dict for cross-thread queries
        let current_tasks = get_current_tasks_dict(vm)?;
        let dict: PyDictRef = current_tasks.downcast().unwrap();

        match dict.get_item(&*loop_obj, vm) {
            Ok(task) => Ok(task),
            Err(_) => Ok(vm.ctx.none()),
        }
    }

    #[pyfunction]
    fn all_tasks(args: LoopArg, vm: &VirtualMachine) -> PyResult {
        let loop_obj = match args.loop_.flatten() {
            Some(l) if !vm.is_none(&l) => l,
            _ => get_running_loop(vm)?,
        };

        let all_tasks_set = get_all_tasks_set(vm)?;
        let result_set = PySet::default().into_ref(&vm.ctx);

        let iter = vm.call_method(&all_tasks_set, "__iter__", ())?;
        loop {
            match vm.call_method(&iter, "__next__", ()) {
                Ok(task) => {
                    // Try get_loop() method first, fallback to _loop property
                    let task_loop = if let Ok(l) = vm.call_method(&task, "get_loop", ()) {
                        Some(l)
                    } else if let Ok(Some(l)) =
                        vm.get_attribute_opt(task.clone(), vm.ctx.intern_str("_loop"))
                    {
                        Some(l)
                    } else {
                        None
                    };

                    if let Some(task_loop) = task_loop
                        && task_loop.is(&loop_obj)
                        && let Ok(done) = vm.call_method(&task, "done", ())
                        && !done.try_to_bool(vm).unwrap_or(true)
                    {
                        result_set.add(task, vm)?;
                    }
                }
                Err(e) if e.fast_isinstance(vm.ctx.exceptions.stop_iteration) => break,
                Err(e) => return Err(e),
            }
        }

        Ok(result_set.into())
    }

    #[pyfunction]
    fn _register_task(task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let all_tasks_set = get_all_tasks_set(vm)?;
        vm.call_method(&all_tasks_set, "add", (task,))?;
        Ok(())
    }

    #[pyfunction]
    fn _unregister_task(task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let all_tasks_set = get_all_tasks_set(vm)?;
        vm.call_method(&all_tasks_set, "discard", (task,))?;
        Ok(())
    }

    #[pyfunction]
    fn _register_eager_task(task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let eager_tasks_set = get_eager_tasks_set(vm)?;
        vm.call_method(&eager_tasks_set, "add", (task,))?;
        Ok(())
    }

    #[pyfunction]
    fn _unregister_eager_task(task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let eager_tasks_set = get_eager_tasks_set(vm)?;
        vm.call_method(&eager_tasks_set, "discard", (task,))?;
        Ok(())
    }

    #[pyfunction]
    fn _enter_task(loop_: PyObjectRef, task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Per-thread check, matching CPython's ts->asyncio_running_task
        {
            let running_task = vm.asyncio_running_task.borrow();
            if running_task.is_some() {
                return Err(vm.new_runtime_error(format!(
                    "Cannot enter into task {:?} while another task {:?} is being executed.",
                    task,
                    running_task.as_ref().unwrap()
                )));
            }
        }

        *vm.asyncio_running_task.borrow_mut() = Some(task.clone());

        // Also update the module-level dict for cross-thread queries
        if let Ok(current_tasks) = get_current_tasks_dict(vm)
            && let Ok(dict) = current_tasks.downcast::<rustpython_vm::builtins::PyDict>()
        {
            let _ = dict.set_item(&*loop_, task, vm);
        }
        Ok(())
    }

    #[pyfunction]
    fn _leave_task(loop_: PyObjectRef, task: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        // Per-thread check, matching CPython's ts->asyncio_running_task
        {
            let running_task = vm.asyncio_running_task.borrow();
            match running_task.as_ref() {
                None => {
                    return Err(vm.new_runtime_error(
                        "_leave_task: task is not the current task".to_owned(),
                    ));
                }
                Some(current) if !current.is(&task) => {
                    return Err(vm.new_runtime_error(
                        "_leave_task: task is not the current task".to_owned(),
                    ));
                }
                _ => {}
            }
        }

        *vm.asyncio_running_task.borrow_mut() = None;

        // Also update the module-level dict
        if let Ok(current_tasks) = get_current_tasks_dict(vm)
            && let Ok(dict) = current_tasks.downcast::<rustpython_vm::builtins::PyDict>()
        {
            let _ = dict.del_item(&*loop_, vm);
        }
        Ok(())
    }

    #[pyfunction]
    fn _swap_current_task(loop_: PyObjectRef, task: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Per-thread swap, matching CPython's swap_current_task
        let prev = vm
            .asyncio_running_task
            .borrow()
            .clone()
            .unwrap_or_else(|| vm.ctx.none());

        if vm.is_none(&task) {
            *vm.asyncio_running_task.borrow_mut() = None;
        } else {
            *vm.asyncio_running_task.borrow_mut() = Some(task.clone());
        }

        // Also update the module-level dict for cross-thread queries
        if let Ok(current_tasks) = get_current_tasks_dict(vm)
            && let Ok(dict) = current_tasks.downcast::<rustpython_vm::builtins::PyDict>()
        {
            if vm.is_none(&task) {
                let _ = dict.del_item(&*loop_, vm);
            } else {
                let _ = dict.set_item(&*loop_, task, vm);
            }
        }

        Ok(prev)
    }

    /// Reset task state after fork in child process.
    #[pyfunction]
    fn _on_fork(vm: &VirtualMachine) -> PyResult<()> {
        // Clear current_tasks dict so child process doesn't inherit parent's tasks
        if let Ok(current_tasks) = get_current_tasks_dict(vm) {
            vm.call_method(&current_tasks, "clear", ())?;
        }
        // Clear the running loop and task
        *vm.asyncio_running_loop.borrow_mut() = None;
        *vm.asyncio_running_task.borrow_mut() = None;
        Ok(())
    }

    #[pyfunction]
    fn future_add_to_awaited_by(
        fut: PyObjectRef,
        waiter: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Only operate on native Future/Task objects (including subclasses).
        // Non-native objects are silently ignored.
        if let Some(task) = fut.downcast_ref::<PyTask>() {
            return task.base.awaited_by_add(waiter, vm);
        }
        if let Some(future) = fut.downcast_ref::<PyFuture>() {
            return future.awaited_by_add(waiter, vm);
        }
        Ok(())
    }

    #[pyfunction]
    fn future_discard_from_awaited_by(
        fut: PyObjectRef,
        waiter: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        // Only operate on native Future/Task objects (including subclasses).
        // Non-native objects are silently ignored.
        if let Some(task) = fut.downcast_ref::<PyTask>() {
            return task.base.awaited_by_discard(&waiter, vm);
        }
        if let Some(future) = fut.downcast_ref::<PyFuture>() {
            return future.awaited_by_discard(&waiter, vm);
        }
        Ok(())
    }

    // TaskStepMethWrapper - wrapper for task step callback with proper repr

    #[pyattr]
    #[pyclass(name, traverse)]
    #[derive(Debug, PyPayload)]
    struct TaskStepMethWrapper {
        task: PyRwLock<Option<PyObjectRef>>,
    }

    #[pyclass(with(Callable, Representable))]
    impl TaskStepMethWrapper {
        fn new(task: PyObjectRef) -> Self {
            Self {
                task: PyRwLock::new(Some(task)),
            }
        }

        // __self__ property returns the task, used by _format_handle in base_events.py
        #[pygetset]
        fn __self__(&self, vm: &VirtualMachine) -> PyObjectRef {
            self.task.read().clone().unwrap_or_else(|| vm.ctx.none())
        }

        #[pygetset]
        fn __qualname__(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            match self.task.read().as_ref() {
                Some(t) => vm.get_attribute_opt(t.clone(), vm.ctx.intern_str("__qualname__")),
                None => Ok(None),
            }
        }
    }

    impl Callable for TaskStepMethWrapper {
        type Args = ();
        fn call(zelf: &Py<Self>, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let task = zelf.task.read().clone();
            match task {
                Some(t) => task_step_impl(&t, None, vm),
                None => Ok(vm.ctx.none()),
            }
        }
    }

    impl Representable for TaskStepMethWrapper {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} object at {:#x}>",
                zelf.class().name(),
                zelf.get_id()
            ))
        }
    }

    /// TaskWakeupMethWrapper - wrapper for task wakeup callback with proper repr
    #[pyattr]
    #[pyclass(name, traverse)]
    #[derive(Debug, PyPayload)]
    struct TaskWakeupMethWrapper {
        task: PyRwLock<Option<PyObjectRef>>,
    }

    #[pyclass(with(Callable, Representable))]
    impl TaskWakeupMethWrapper {
        fn new(task: PyObjectRef) -> Self {
            Self {
                task: PyRwLock::new(Some(task)),
            }
        }

        #[pygetset]
        fn __qualname__(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            match self.task.read().as_ref() {
                Some(t) => vm.get_attribute_opt(t.clone(), vm.ctx.intern_str("__qualname__")),
                None => Ok(None),
            }
        }
    }

    impl Callable for TaskWakeupMethWrapper {
        type Args = (PyObjectRef,);
        fn call(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let task = zelf.task.read().clone();
            match task {
                Some(t) => task_wakeup_impl(&t, &args.0, vm),
                None => Ok(vm.ctx.none()),
            }
        }
    }

    impl Representable for TaskWakeupMethWrapper {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<{} object at {:#x}>",
                zelf.class().name(),
                zelf.get_id()
            ))
        }
    }

    fn is_coroutine(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        if obj.class().is(vm.ctx.types.coroutine_type) {
            return Ok(true);
        }

        let asyncio_coroutines = vm.import("asyncio.coroutines", 0)?;
        if let Some(iscoroutine) =
            vm.get_attribute_opt(asyncio_coroutines, vm.ctx.intern_str("iscoroutine"))?
        {
            let result = iscoroutine.call((obj,), vm)?;
            result.try_to_bool(vm)
        } else {
            Ok(false)
        }
    }

    fn new_invalid_state_error(vm: &VirtualMachine, msg: &str) -> PyBaseExceptionRef {
        match vm.import("asyncio.exceptions", 0) {
            Ok(module) => {
                match vm.get_attribute_opt(module, vm.ctx.intern_str("InvalidStateError")) {
                    Ok(Some(exc_type)) => match exc_type.call((msg,), vm) {
                        Ok(exc) => exc.downcast().unwrap(),
                        Err(_) => vm.new_runtime_error(msg.to_string()),
                    },
                    _ => vm.new_runtime_error(msg.to_string()),
                }
            }
            Err(_) => vm.new_runtime_error(msg.to_string()),
        }
    }

    fn get_copy_context(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let contextvars = vm.import("contextvars", 0)?;
        let copy_context = vm
            .get_attribute_opt(contextvars, vm.ctx.intern_str("copy_context"))?
            .ok_or_else(|| vm.new_attribute_error("copy_context not found"))?;
        copy_context.call((), vm)
    }

    fn get_cancelled_error_type(vm: &VirtualMachine) -> PyResult<PyTypeRef> {
        let module = vm.import("asyncio.exceptions", 0)?;
        let exc_type = vm
            .get_attribute_opt(module, vm.ctx.intern_str("CancelledError"))?
            .ok_or_else(|| vm.new_attribute_error("CancelledError not found"))?;
        exc_type
            .downcast()
            .map_err(|_| vm.new_type_error("CancelledError is not a type".to_string()))
    }

    fn is_cancelled_error(exc: &PyBaseExceptionRef, vm: &VirtualMachine) -> bool {
        match get_cancelled_error_type(vm) {
            Ok(cancelled_error) => exc.fast_isinstance(&cancelled_error),
            Err(_) => false,
        }
    }

    fn is_cancelled_error_obj(obj: &PyObjectRef, vm: &VirtualMachine) -> bool {
        match get_cancelled_error_type(vm) {
            Ok(cancelled_error) => obj.fast_isinstance(&cancelled_error),
            Err(_) => false,
        }
    }
}
