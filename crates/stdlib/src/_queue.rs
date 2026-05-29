pub(crate) use _queue::module_def;

#[pymodule]
mod _queue {
    use alloc::collections::VecDeque;
    use core::time::Duration;
    use std::time::Instant;

    use crate::{
        common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard},
        vm::{
            Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
            builtins::{PyException, PyType},
            function::TimeoutSeconds,
            types::Constructor,
        },
    };

    const INITIAL_RING_BUF_CAPACITY: usize = 8;

    #[pyattr]
    #[pyexception(name = "Empty", base = PyException, impl)]
    #[derive(Debug)]
    #[repr(transparent)]
    pub(crate) struct PyEmptyError(PyException);

    #[pyattr]
    #[pyclass(module = "_queue", name = "SimpleQueue", unhashable = true)]
    #[derive(Debug, Default, PyPayload)]
    struct PySimpleQueue {
        buf: PyRwLock<VecDeque<PyObjectRef>>,
    }

    impl PySimpleQueue {
        fn borrow_buf(&self) -> PyRwLockReadGuard<'_, VecDeque<PyObjectRef>> {
            self.buf.read()
        }

        fn borrow_buf_mut(&self) -> PyRwLockWriteGuard<'_, VecDeque<PyObjectRef>> {
            self.buf.write()
        }

        /// Returns a strong reference from the head of the buffer.
        ///
        /// ## Safety
        /// Called must ensure inner buf is not empty.
        ///
        /// ## See Also
        ///
        /// [`RingBuf_Get`](https://github.com/python/cpython/blob/v3.14.5/Modules/_queuemodule.c#L133-L154).
        unsafe fn get_inner(&self) -> PyObjectRef {
            let mut buf = self.borrow_buf_mut();

            let cap = buf.capacity();
            if buf.len() < (cap / 4) {
                // Items is less than 25% occupied, shrink it by 50%. This allows for
                // growth without immediately needing to resize the underlying items array
                buf.shrink_to(cap / 2)
            }
            // SAFETY: Called must ensure `buf` is not empty.
            unsafe { buf.pop_front().unwrap_unchecked() }
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

    #[pyclass(with(Constructor))]
    impl PySimpleQueue {
        fn new() -> Self {
            Self {
                buf: PyRwLock::new(VecDeque::with_capacity(INITIAL_RING_BUF_CAPACITY)),
            }
        }

        #[pymethod]
        fn empty(&self) -> bool {
            self.borrow_buf().is_empty()
        }

        #[pymethod]
        fn qsize(&self) -> usize {
            self.borrow_buf().len()
        }

        #[pymethod]
        fn put(&self, args: PutArgs) {
            let PutArgs { item, .. } = args;
            let mut buf = self.borrow_buf_mut();
            buf.push_back(item)
        }

        #[pymethod]
        fn put_nowait(&self, item: PyObjectRef) {
            let mut buf = self.borrow_buf_mut();
            buf.push_back(item)
        }

        #[pymethod]
        fn get(&self, args: GetArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let GetArgs { block, timeout } = args;

            let end_time = if !block {
                None
            } else {
                match timeout.map(|v| v.to_secs_f64()) {
                    Some(v) if v < 0.0 => {
                        return Err(vm.new_value_error("'timeout' must be a non-negative number"));
                    }
                    Some(v) => Some(Duration::from_secs_f64(v)),
                    None => None,
                }
            };

            let start_time = if end_time.is_some() {
                Some(Instant::now())
            } else {
                None
            };

            loop {
                if !self.empty() {
                    return Ok(
                        // SAFETY: We just validated that buf is not empty.
                        unsafe { self.get_inner() },
                    );
                }

                if !block {
                    return Err(vm.new_exception_empty(PyEmptyError::class(&vm.ctx).to_owned()));
                }

                if let (Some(start), Some(end)) = (start_time, end_time) {
                    if start.elapsed() < end {
                        return Err(vm.new_exception_empty(PyEmptyError::class(&vm.ctx).to_owned()));
                    }
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
    }

    impl Constructor for PySimpleQueue {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self::new())
        }
    }
}
