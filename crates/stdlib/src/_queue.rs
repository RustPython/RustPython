pub(crate) use _queue::module_def;

#[pymodule]
mod _queue {
    use alloc::collections::VecDeque;
    use core::time::Duration;
    use std::time::Instant;

    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyException, PyGenericAlias, PyStr, PyType, PyTypeRef},
        function::{PyComparisonValue, TimeoutSeconds},
        protocol::PyNumberMethods,
        types::{AsNumber, Comparable, Constructor, PyComparisonOp, Representable},
    };

    type BufInner = VecDeque<PyObjectRef>;

    cfg_select! {
        feature = "threading" => {
            use parking_lot::{Condvar, Mutex, MutexGuard};

            type Buf = Mutex<BufInner>;
        },
        _ => {
            use crate::common::lock::PyMutex;

            type Buf = PyMutex<BufInner>;
        }
    }

    const INITIAL_RING_BUF_CAPACITY: usize = 8;

    #[pyattr]
    #[pyclass(module = "_queue", name = "Empty", base = PyException)]
    #[repr(transparent)]
    pub(crate) struct PyEmptyError(PyException);

    #[pyclass(flags(HAS_WEAKREF))]
    impl PyEmptyError {}

    /// ## See Also
    ///
    /// [`empty_error`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L347-L355).
    fn empty_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_empty(PyEmptyError::class(&vm.ctx).to_owned())
    }

    #[cfg(feature = "threading")]
    #[derive(Debug)]
    struct Semaphore {
        mutex: Mutex<usize>,
        cond: Condvar,
    }

    #[cfg(feature = "threading")]
    impl Semaphore {
        #[must_use]
        fn new() -> Self {
            Self {
                mutex: Mutex::new(0),
                cond: Condvar::new(),
            }
        }

        fn release(&self) {
            {
                let mut count = self.mutex.lock();
                *count += 1;
            } // lock dropped. now we can notify a waiting thread

            self.cond.notify_one();
        }

        /// Returns `true` if the semaphore was acquired, `false` on timeout.
        #[must_use]
        fn acquire(&self, block: bool, deadline: Option<Instant>, vm: &VirtualMachine) -> bool {
            let mut count = self.mutex.lock();
            loop {
                if *count > 0 {
                    *count -= 1;
                    return true;
                }

                if !block {
                    return false;
                }

                match deadline {
                    Some(dl) => {
                        let result = vm.allow_threads(|| self.cond.wait_until(&mut count, dl));
                        if result.timed_out() && *count == 0 {
                            return false;
                        }
                    }
                    None => {
                        vm.allow_threads(|| self.cond.wait(&mut count));
                    }
                }
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_queue", name = "SimpleQueue", unhashable = true)]
    #[derive(Debug, PyPayload)]
    struct PySimpleQueue {
        buf: Buf,
        #[cfg(feature = "threading")]
        sem: Semaphore,
    }

    impl Default for PySimpleQueue {
        fn default() -> Self {
            Self {
                buf: Buf::new(VecDeque::with_capacity(INITIAL_RING_BUF_CAPACITY)),
                #[cfg(feature = "threading")]
                sem: Semaphore::new(),
            }
        }
    }

    impl PySimpleQueue {
        fn push(&self, item: PyObjectRef) {
            self.buf.lock().push_back(item);

            #[cfg(feature = "threading")]
            self.sem.release();
        }

        /// Returns a strong reference from the head of the buffer.
        ///
        /// ## See Also
        ///
        /// [`RingBuf_Get`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L133-L154).
        fn get_inner(
            #[cfg(feature = "threading")] buf: &mut MutexGuard<'_, BufInner>,
            #[cfg(not(feature = "threading"))] buf: &mut BufInner,
        ) -> Option<PyObjectRef> {
            let cap = buf.capacity();

            if buf.len() < (cap / 4) {
                // Items is less than 25% occupied, shrink it by 50%. This allows for
                // growth without immediately needing to resize the underlying items array
                buf.shrink_to(cap / 2)
            }

            buf.pop_front()
        }
    }

    #[derive(FromArgs)]
    struct PutArgs {
        #[pyarg(positional)]
        item: PyObjectRef,
        #[expect(
            dead_code,
            reason = "Intentional. Provide compatibility with the Queue class"
        )]
        #[pyarg(any, optional, default = true)]
        block: bool,
        #[expect(
            dead_code,
            reason = "Intentional. Provide compatibility with the Queue class"
        )]
        #[pyarg(any, optional)]
        timeout: Option<PyObjectRef>,
    }

    #[derive(FromArgs)]
    struct GetArgs {
        #[pyarg(any, optional, default = true)]
        block: bool,
        #[pyarg(any, optional)]
        timeout: Option<TimeoutSeconds>,
    }

    #[pyclass(
        with(Constructor, Comparable, Representable),
        flags(BASETYPE, HAS_WEAKREF, IMMUTABLETYPE)
    )]
    impl PySimpleQueue {
        #[pymethod]
        fn empty(&self) -> bool {
            self.buf.lock().is_empty()
        }

        #[pymethod]
        fn qsize(&self) -> usize {
            self.buf.lock().len()
        }

        #[pymethod]
        fn put(&self, args: PutArgs) {
            let PutArgs { item, .. } = args;
            self.push(item);
        }

        #[pymethod]
        fn put_nowait(&self, item: PyObjectRef) {
            self.push(item);
        }

        #[pymethod]
        fn get(&self, args: GetArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let GetArgs { block, timeout } = args;

            // Non-blocking: just try once
            if !block {
                return Self::get_inner(&mut self.buf.lock()).ok_or_else(|| empty_error(vm));
            }

            #[cfg_attr(
                not(feature = "threading"),
                expect(
                    unused_variables,
                    reason = "We are still validating the 'timeout' arg even if we don't have threading"
                )
            )]
            let deadline = match timeout.map(|v| v.to_secs_f64()) {
                Some(v) if v < 0.0 => {
                    return Err(vm.new_value_error("'timeout' must be a non-negative number"));
                }
                Some(v) => Some(Instant::now() + Duration::from_secs_f64(v)),
                None => None,
            };

            #[cfg(feature = "threading")]
            {
                if !self.sem.acquire(block, deadline, vm) {
                    return Err(empty_error(vm));
                }
            }

            Self::get_inner(&mut self.buf.lock()).ok_or_else(|| empty_error(vm))
        }

        #[pymethod]
        fn get_nowait(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            #[cfg(feature = "threading")]
            {
                if !self.sem.acquire(false, None, vm) {
                    return Err(empty_error(vm));
                }
            }

            Self::get_inner(&mut self.buf.lock()).ok_or_else(|| empty_error(vm))
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

    impl Constructor for PySimpleQueue {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::default())
        }
    }

    impl AsNumber for PySimpleQueue {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                boolean: Some(|number, _vm| {
                    let zelf = number.obj.downcast_ref::<PySimpleQueue>().unwrap();
                    Ok(!zelf.buf.lock().is_empty())
                }),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Comparable for PySimpleQueue {
        fn cmp(
            zelf: &Py<Self>,
            other: &PyObject,
            op: PyComparisonOp,
            _vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            Ok(if let Some(res) = op.identical_optimization(zelf, other) {
                res.into()
            } else {
                PyComparisonValue::NotImplemented
            })
        }
    }

    impl Representable for PySimpleQueue {
        fn repr(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
            Ok(vm.ctx.new_str(format!(
                "<{} at {:#x}>",
                Self::class(&vm.ctx).slot_name(),
                zelf.get_id()
            )))
        }

        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            unreachable!("repr() is overridden directly")
        }
    }
}
