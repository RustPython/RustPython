pub(crate) use _queue::module_def;

#[pymodule]
mod _queue {
    use alloc::collections::VecDeque;
    use core::time::Duration;
    use std::time::Instant;

    use crate::{
        common::lock::PyMutex,
        vm::{
            AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
            builtins::{PyBaseExceptionRef, PyException, PyGenericAlias, PyStr, PyType, PyTypeRef},
            function::{PyComparisonValue, TimeoutSeconds},
            protocol::PyNumberMethods,
            types::{AsNumber, Comparable, Constructor, PyComparisonOp, Representable},
        },
    };

    use parking_lot::Condvar;

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
        buf: PyMutex<VecDeque<PyObjectRef>>,
        not_empty: Condvar,
    }

    impl PySimpleQueue {
        /// Returns a strong reference from the head of the buffer.
        ///
        /// ## See Also
        ///
        /// [`RingBuf_Get`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L133-L154).
        fn get_inner(buf: &mut VecDeque<PyObjectRef>) -> Option<PyObjectRef> {
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
        fn new() -> Self {
            Self {
                buf: PyMutex::new(VecDeque::with_capacity(INITIAL_RING_BUF_CAPACITY)),
                not_empty: Condvar::new(),
            }
        }

        #[pymethod]
        fn empty(&self) -> bool {
            let buf = self.buf.lock();
            buf.is_empty()
        }

        #[pymethod]
        fn qsize(&self) -> usize {
            let buf = self.buf.lock();
            buf.len()
        }

        #[pymethod]
        fn put(&self, args: PutArgs) {
            let PutArgs { item, .. } = args;
            let mut buf = self.buf.lock();
            buf.push_back(item);
            self.not_empty.notify_one();
        }

        #[pymethod]
        fn put_nowait(&self, item: PyObjectRef) {
            let mut buf = self.buf.lock();
            buf.push_back(item);
            self.not_empty.notify_one();
        }

        #[pymethod]
        fn get(&self, args: GetArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let GetArgs { block, timeout } = args;

            // Non-blocking: just try once
            if !block {
                let mut buf = self.buf.lock();
                return Self::get_inner(&mut buf).ok_or_else(|| empty_error(vm));
            }

            let deadline = match timeout.map(|v| v.to_secs_f64()) {
                Some(v) if v < 0.0 => {
                    return Err(vm.new_value_error("'timeout' must be a non-negative number"));
                }
                Some(v) => Some(Instant::now() + Duration::from_secs_f64(v)),
                None => None,
            };

            let mut buf = self.buf.lock();
            loop {
                if let Some(item) = Self::get_inner(&mut buf) {
                    return Ok(item);
                }

                let timed_out = if let Some(deadline) = deadline {
                    // Sleep until notified or deadline reached
                    let result = self.not_empty.wait_until(&mut buf, deadline);
                    result.timed_out()
                } else {
                    // Sleep until notified
                    self.not_empty.wait(&mut buf);
                    false
                };

                if timed_out {
                    return Err(empty_error(vm));
                }
            }
        }

        #[pymethod]
        fn get_nowait(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            self.get(
                GetArgs {
                    block: false,
                    timeout: None,
                },
                vm,
            )
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
            Ok(Self::new())
        }
    }

    impl AsNumber for PySimpleQueue {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                boolean: Some(|number, _vm| {
                    let zelf = number.obj.downcast_ref::<PySimpleQueue>().unwrap();
                    let buf = zelf.buf.lock();
                    Ok(!buf.is_empty())
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
            Ok(op.identical_optimization(zelf, other).unwrap().into())
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
