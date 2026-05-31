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
            use parking_lot::{Condvar, Mutex, ReentrantMutex, ReentrantMutexGuard};

            use core::cell::RefCell;

            type Buf = ReentrantMutex<RefCell<BufInner>>;
        },
        _ => {
            use crate::common::lock::{PyMutex, PyMutexGuard};

            type Buf = PyMutex<VecDeque<PyObjectRef>>;
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

    #[pyattr]
    #[pyclass(module = "_queue", name = "SimpleQueue", unhashable = true)]
    #[derive(Debug, PyPayload)]
    struct PySimpleQueue {
        buf: Buf,
        #[cfg(feature = "threading")]
        not_empty: Condvar,
        #[cfg(feature = "threading")]
        wait_mutex: Mutex<()>,
    }

    impl Default for PySimpleQueue {
        fn default() -> Self {
            Self {
                buf: Buf::new(cfg_select! {
                    feature = "threading" => VecDeque::with_capacity(INITIAL_RING_BUF_CAPACITY).into(),
                    _ => VecDeque::with_capacity(INITIAL_RING_BUF_CAPACITY),
                }),
                #[cfg(feature = "threading")]
                not_empty: Condvar::new(),
                #[cfg(feature = "threading")]
                wait_mutex: Mutex::new(()),
            }
        }
    }

    impl PySimpleQueue {
        fn push(&self, item: PyObjectRef) {
            cfg_select! {
                feature = "threading" => {
                    self.borrow_buf().borrow_mut().push_back(item);
                    self.not_empty.notify_one();
                },
                _ => {
                    self.borrow_buf().push_back(item);
                }
            }
        }

        cfg_select! {
            feature = "threading" => {
                /// Returns a strong reference from the head of the buffer.
                ///
                /// ## See Also
                ///
                /// [`RingBuf_Get`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L133-L154).
                fn get_inner(buf: &ReentrantMutexGuard<'_, RefCell<BufInner>>) -> Option<PyObjectRef> {
                    let mut inner = buf.borrow_mut();

                    let cap = inner.capacity();
                    if inner.len() < (cap / 4) {
                        // Items is less than 25% occupied, shrink it by 50%. This allows for
                        // growth without immediately needing to resize the underlying items array
                        inner.shrink_to(cap / 2)
                    }

                    inner.pop_front()
                }
            }
            _ => {
                /// Returns a strong reference from the head of the buffer.
                ///
                /// ## See Also
                ///
                /// [`RingBuf_Get`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L133-L154).
                fn get_inner(buf: &mut BufInner) -> Option<PyObjectRef> {
                    let cap = buf.capacity();
                    if buf.len() < (cap / 4) {
                        // Items is less than 25% occupied, shrink it by 50%. This allows for
                        // growth without immediately needing to resize the underlying items array
                        buf.shrink_to(cap / 2)
                    }
                    buf.pop_front()
                }
            }
        }

        cfg_select! {
            feature = "threading" => {
                fn borrow_buf(&self) -> ReentrantMutexGuard<'_, RefCell<BufInner>> {
                    self.buf.lock()
                }
            }
            _ => {
                fn borrow_buf(&self) -> PyMutexGuard<'_, BufInner> {
                    self.buf.lock()
                }
            }
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
            cfg_select! {
                feature = "threading" => self.borrow_buf().borrow().is_empty(),
                _ => self.borrow_buf().is_empty(),
            }
        }

        #[pymethod]
        fn qsize(&self) -> usize {
            cfg_select! {
                feature = "threading" => self.borrow_buf().borrow().len(),
                _ => self.borrow_buf().len(),
            }
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
                return Self::get_inner(cfg_select! {
                    feature = "threading" => &self.borrow_buf(),
                    _ => &mut self.borrow_buf(),
                })
                .ok_or_else(|| empty_error(vm));
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

            cfg_select! {
                feature = "threading" => {
                    loop {
                        {
                            let guard = self.borrow_buf();
                            if let Some(item) = Self::get_inner(&guard) {
                                return Ok(item);
                            }
                        } // guard dropped here

                        let mut wait_guard = self.wait_mutex.lock();

                        if let Some(deadline) = deadline {
                            // Sleep until notified or deadline reached
                            let result = self.not_empty.wait_until(&mut wait_guard, deadline);
                            if result.timed_out() {
                                return Err(empty_error(vm));
                            }
                        } else {
                            // Sleep until notified
                            self.not_empty.wait(&mut wait_guard);
                        }
                    }
                }
                _ => {
                    Self::get_inner(&mut self.borrow_buf()).ok_or_else(|| empty_error(vm))
                }
            }
        }

        #[pymethod]
        fn get_nowait(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            Self::get_inner(cfg_select! {
                feature = "threading" => &self.borrow_buf(),
                _ => &mut self.borrow_buf(),
            })
            .ok_or_else(|| empty_error(vm))
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
                    Ok(cfg_select! {
                        feature = "threading" => !zelf.borrow_buf().borrow().is_empty(),
                        _ => !zelf.borrow_buf().is_empty(),
                    })
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
