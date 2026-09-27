// spell-checker:ignore compresslevel

pub(crate) use _bz2::module_def;

#[pymodule]
mod _bz2 {
    use crate::compression::DecompressorArgs;
    use crate::vm::{
        Py, PyObject, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytes, PyType},
        common::lock::PyMutex,
        function::ArgBytesLike,
        object::{PyAtomicRef, PyResult},
        types::Constructor,
    };
    use alloc::fmt;
    use core::sync::atomic::{AtomicBool, Ordering};
    use rustpython_common::compression::bz2 as backend;

    fn map_bz2_error(error: backend::Bz2Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match error {
            backend::Bz2Error::Param => {
                vm.new_value_error("Internal error - invalid parameters passed to libbzip2")
            }
            backend::Bz2Error::Data => vm.new_os_error("Invalid data stream"),
            backend::Bz2Error::Sequence => vm.new_runtime_error(
                "Internal error - Invalid sequence of commands sent to libbzip2",
            ),
            backend::Bz2Error::Mem => vm.no_memory_error(),
        }
    }

    struct PyBZ2DecompressorInner {
        decompress: backend::Decompressor,
    }

    #[pyattr]
    #[pyclass(name = "BZ2Decompressor", traverse)]
    #[derive(PyPayload)]
    struct BZ2Decompressor {
        #[pytraverse(skip)]
        inner: PyMutex<PyBZ2DecompressorInner>,
        #[pymember]
        #[pytraverse(skip)]
        eof: AtomicBool,
        #[pymember]
        #[pytraverse(skip)]
        needs_input: AtomicBool,
        #[pymember(type = "object_ex")]
        unused_data: PyAtomicRef<Option<PyObject>>,
    }

    impl fmt::Debug for BZ2Decompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_bz2.BZ2Decompressor")
        }
    }

    impl Constructor for BZ2Decompressor {
        type Args = ();

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: PyMutex::new(PyBZ2DecompressorInner {
                    decompress: backend::Decompressor::new(),
                }),
                eof: AtomicBool::new(false),
                needs_input: AtomicBool::new(true),
                unused_data: PyAtomicRef::from(Some(vm.ctx.empty_bytes.clone().into())),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl BZ2Decompressor {
        #[pymethod]
        fn decompress(&self, args: DecompressorArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();

            let mut inner = self.inner.lock();
            if inner.decompress.eof() {
                return Err(vm.new_eof_error("End of stream already reached"));
            }
            if inner.decompress.failed() {
                return Err(vm.new_value_error("Decompressor is unusable after a previous error"));
            }
            let result = inner.decompress.decompress(data, max_length);
            self.eof.store(inner.decompress.eof(), Ordering::Relaxed);
            self.needs_input
                .store(inner.decompress.needs_input(), Ordering::Relaxed);
            let stale = self.unused_data.deref().is_none_or(|obj| {
                obj.downcast_ref::<PyBytes>()
                    .is_none_or(|bytes| bytes.as_bytes() != inner.decompress.unused_data())
            });
            if stale {
                let bytes = vm.ctx.new_bytes(inner.decompress.unused_data().to_vec());
                // The previous bytes object is dropped after the slot is replaced.
                let _previous = unsafe { self.unused_data.swap(Some(bytes.into())) };
            }
            result.map_err(|error| map_bz2_error(error, vm))
        }

        #[pymethod(name = "__reduce__")]
        fn reduce(&self, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot pickle '_bz2.BZ2Decompressor' object"))
        }
    }

    #[pyattr]
    #[pyclass(name = "BZ2Compressor")]
    #[derive(PyPayload)]
    struct BZ2Compressor {
        state: PyMutex<backend::Compressor>,
    }

    impl fmt::Debug for BZ2Compressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_bz2.BZ2Compressor")
        }
    }

    #[derive(FromArgs)]
    struct BZ2CompressorArgs {
        #[pyarg(positional, default = 9)]
        compresslevel: i32,
    }

    impl Constructor for BZ2Compressor {
        type Args = BZ2CompressorArgs;

        fn py_new(
            _cls: &Py<PyType>,
            BZ2CompressorArgs { compresslevel }: Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let compressor = backend::Compressor::new(i64::from(compresslevel))
                .ok_or_else(|| vm.new_value_error("compresslevel must be between 1 and 9"))?;
            Ok(Self {
                state: PyMutex::new(compressor),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl BZ2Compressor {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut compressor = self.state.lock();
            if compressor.is_flushed() {
                return Err(vm.new_value_error("Compressor has been flushed"));
            }
            data.with_ref(|input| compressor.compress(input))
                .map_err(|error| map_bz2_error(error, vm))
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut compressor = self.state.lock();
            if compressor.is_flushed() {
                return Err(vm.new_value_error("Repeated call to flush()"));
            }
            compressor.flush().map_err(|error| map_bz2_error(error, vm))
        }

        #[pymethod(name = "__reduce__")]
        fn reduce(&self, vm: &VirtualMachine) -> PyResult<()> {
            Err(vm.new_type_error("cannot pickle '_bz2.BZ2Compressor' object"))
        }
    }
}
