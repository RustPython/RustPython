// spell-checker:ignore compressobj decompressobj zdict chunksize zlibmodule

pub(crate) use zlib::module_def;

#[pymodule]
mod zlib {
    use crate::compression::DecompressArgs;
    use crate::vm::{
        Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytesRef, PyIntRef, PyType, PyTypeRef},
        common::lock::PyMutex,
        convert::TryFromBorrowedObject,
        function::{ArgBytesLike, ArgPrimitiveIndex, ArgSize, OptionalArg},
        types::Constructor,
    };
    use adler32::RollingAdler32 as Adler32;
    use alloc::fmt;
    use rustpython_common::compression::zlib as backend;

    #[pyattr]
    use rustpython_common::compression::zlib::{
        Z_BEST_COMPRESSION, Z_BEST_SPEED, Z_BLOCK, Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY,
        Z_DEFLATED as DEFLATED, Z_FILTERED, Z_FINISH, Z_FIXED, Z_FULL_FLUSH, Z_HUFFMAN_ONLY,
        Z_NO_COMPRESSION, Z_NO_FLUSH, Z_PARTIAL_FLUSH, Z_RLE, Z_SYNC_FLUSH, Z_TREES,
    };

    #[pyattr(name = "__version__")]
    const __VERSION__: &str = "1.0";

    // We statically link libz-rs, so the compile-time and runtime versions
    // always match.
    #[pyattr(name = "ZLIB_RUNTIME_VERSION")]
    #[pyattr]
    const ZLIB_VERSION: &str = backend::version();

    #[pyattr]
    const MAX_WBITS: i32 = backend::MAX_WBITS;
    #[pyattr]
    const DEF_BUF_SIZE: usize = backend::DEF_BUF_SIZE;
    #[pyattr]
    const DEF_MEM_LEVEL: u8 = 8;

    #[pyattr(once)]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "zlib",
            "error",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyfunction]
    fn adler32(data: ArgBytesLike, begin_state: OptionalArg<PyIntRef>) -> u32 {
        data.with_ref(|data| {
            let begin_state = begin_state.map_or(1, |i| i.as_u32_mask());
            let mut hasher = Adler32::from_value(begin_state);
            hasher.update_buffer(data);
            hasher.hash()
        })
    }

    #[pyfunction]
    fn crc32(data: ArgBytesLike, begin_state: OptionalArg<PyIntRef>) -> u32 {
        crate::binascii::crc32(data, begin_state)
    }

    #[derive(FromArgs)]
    struct PyFuncCompressArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, default = Level::new(Z_DEFAULT_COMPRESSION))]
        level: Level,
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i32>,
    }

    #[pyfunction]
    fn compress(args: PyFuncCompressArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let PyFuncCompressArgs { data, level, wbits } = args;
        let level = level
            .value()
            .ok_or_else(|| new_zlib_error("Bad compression level", vm))?;
        let encoded = data.with_ref(|data| backend::compress(data, level, wbits.value));
        encoded
            .map(|data| vm.ctx.new_bytes(data))
            .map_err(|err| new_init_or_zlib_error(err, vm))
    }

    #[derive(FromArgs)]
    struct PyFuncDecompressArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i32>,
        #[pyarg(any, default = ArgPrimitiveIndex { value: DEF_BUF_SIZE })]
        bufsize: ArgPrimitiveIndex<usize>,
    }

    #[pyfunction]
    fn decompress(args: PyFuncDecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let PyFuncDecompressArgs {
            data,
            wbits,
            bufsize,
        } = args;
        data.with_ref(|data| backend::decompress(data, wbits.value, bufsize.value))
            .map_err(|err| new_init_or_zlib_error(err, vm))
    }

    #[derive(FromArgs)]
    struct DecompressobjArgs {
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i32>,
        #[pyarg(any, optional)]
        zdict: OptionalArg<ArgBytesLike>,
    }

    fn owned_dict(zdict: OptionalArg<ArgBytesLike>) -> Option<Vec<u8>> {
        zdict
            .into_option()
            .map(|dict| dict.with_ref(|data| data.to_vec()))
    }

    #[pyfunction]
    fn decompressobj(args: DecompressobjArgs, vm: &VirtualMachine) -> PyResult<PyDecompress> {
        let decompress = backend::Decompressor::new(args.wbits.value, owned_dict(args.zdict))
            .map_err(|err| new_init_or_zlib_error(err, vm))?;
        Ok(PyDecompress {
            inner: PyMutex::new(PyDecompressInner {
                decompress,
                unused_data: vm.ctx.empty_bytes.clone(),
                unconsumed_tail: vm.ctx.empty_bytes.clone(),
            }),
        })
    }

    struct PyDecompressInner {
        decompress: backend::Decompressor,
        unused_data: PyBytesRef,
        unconsumed_tail: PyBytesRef,
    }

    impl PyDecompressInner {
        fn sync_visible_state(&mut self, vm: &VirtualMachine) {
            if self.unused_data.as_bytes() != self.decompress.unused_data() {
                self.unused_data = vm.ctx.new_bytes(self.decompress.unused_data().to_vec());
            }
            if self.unconsumed_tail.as_bytes() != self.decompress.unconsumed_tail() {
                self.unconsumed_tail = vm.ctx.new_bytes(self.decompress.unconsumed_tail().to_vec());
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "Decompress", traverse)]
    #[derive(PyPayload)]
    struct PyDecompress {
        #[pytraverse(skip)]
        inner: PyMutex<PyDecompressInner>,
    }

    impl fmt::Debug for PyDecompress {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "zlib.Decompress")
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PyDecompress {
        fn copy_inner(&self, vm: &VirtualMachine) -> PyResult<Self> {
            let inner = self.inner.lock();
            let decompress = inner
                .decompress
                .copy()
                .map_err(|err| vm.new_value_error(err))?;
            Ok(Self {
                inner: PyMutex::new(PyDecompressInner {
                    decompress,
                    unused_data: inner.unused_data.clone(),
                    unconsumed_tail: inner.unconsumed_tail.clone(),
                }),
            })
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.inner.lock().decompress.eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.inner.lock().unused_data.clone()
        }

        #[pygetset]
        fn unconsumed_tail(&self) -> PyBytesRef {
            self.inner.lock().unconsumed_tail.clone()
        }

        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length: usize = args
                .raw_max_length()
                .unwrap_or(0)
                .try_into()
                .map_err(|_| vm.new_value_error("max_length must be non-negative"))?;
            let max_length = (max_length != 0).then_some(max_length);
            let data = &*args.data();

            let mut inner = self.inner.lock();
            let result = inner.decompress.decompress(data, max_length);
            inner.sync_visible_state(vm);
            result.map_err(|err| new_zlib_error(err, vm))
        }

        #[pymethod]
        fn flush(&self, length: OptionalArg<ArgSize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let length = match length {
                OptionalArg::Present(ArgSize { value }) if value <= 0 => {
                    return Err(vm.new_value_error("length must be greater than zero"));
                }
                OptionalArg::Present(ArgSize { value }) => value as usize,
                OptionalArg::Missing => DEF_BUF_SIZE,
            };

            let mut inner = self.inner.lock();
            let result = inner.decompress.flush(length);
            inner.sync_visible_state(vm);
            result.map_err(|err| new_zlib_error(err, vm))
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }

        #[pymethod(name = "__copy__")]
        fn copy_dunder(&self, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }

        #[pymethod(name = "__deepcopy__")]
        fn deepcopy(&self, _memo: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }
    }

    #[derive(FromArgs)]
    struct CompressobjArgs {
        #[pyarg(any, default = Level::new(Z_DEFAULT_COMPRESSION))]
        level: Level,
        #[pyarg(any, default = DEFLATED)]
        method: i32,
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i32>,
        #[pyarg(any, name = "memLevel", default = DEF_MEM_LEVEL)]
        mem_level: u8,
        #[pyarg(any, default = Z_DEFAULT_STRATEGY)]
        strategy: i32,
        #[pyarg(any, optional)]
        zdict: OptionalArg<ArgBytesLike>,
    }

    #[pyfunction]
    fn compressobj(args: CompressobjArgs, vm: &VirtualMachine) -> PyResult<PyCompress> {
        let CompressobjArgs {
            level,
            method,
            wbits,
            mem_level,
            strategy,
            zdict,
        } = args;
        let level = level
            .value()
            .ok_or_else(|| vm.new_value_error("Invalid initialization option"))?;
        let zdict = owned_dict(zdict);
        let compress = backend::Compressor::new(
            level,
            method,
            wbits.value,
            mem_level.into(),
            strategy,
            zdict.as_deref(),
        )
        .map_err(|err| new_init_or_zlib_error(err, vm))?;
        Ok(PyCompress {
            inner: PyMutex::new(compress),
        })
    }

    #[pyattr]
    #[pyclass(name = "Compress", traverse)]
    #[derive(PyPayload)]
    struct PyCompress {
        #[pytraverse(skip)]
        inner: PyMutex<backend::Compressor>,
    }

    impl fmt::Debug for PyCompress {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "zlib.Compress")
        }
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PyCompress {
        fn copy_inner(&self, vm: &VirtualMachine) -> PyResult<Self> {
            let compress = self
                .inner
                .lock()
                .copy()
                .map_err(|err| vm.new_value_error(err))?;
            Ok(Self {
                inner: PyMutex::new(compress),
            })
        }

        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            data.with_ref(|data| self.inner.lock().compress(data))
                .map_err(|err| new_zlib_error(err, vm))
        }

        #[pymethod]
        fn flush(&self, mode: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.inner
                .lock()
                .flush(mode.unwrap_or(Z_FINISH))
                .map_err(|err| new_zlib_error(err, vm))
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }

        #[pymethod(name = "__copy__")]
        fn copy_dunder(&self, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }

        #[pymethod(name = "__deepcopy__")]
        fn deepcopy(&self, _memo: PyObjectRef, vm: &VirtualMachine) -> PyResult<Self> {
            self.copy_inner(vm)
        }
    }

    fn new_zlib_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.class("zlib", "error"), message.into().into())
    }

    fn new_init_or_zlib_error(
        error: backend::InitError,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        match error {
            backend::InitError::InvalidOption => {
                vm.new_value_error("Invalid initialization option")
            }
            backend::InitError::Zlib(message) => new_zlib_error(message, vm),
        }
    }

    struct Level(Option<i32>);

    impl Level {
        const fn new(level: i32) -> Self {
            if matches!(
                level,
                Z_DEFAULT_COMPRESSION | Z_NO_COMPRESSION..=Z_BEST_COMPRESSION
            ) {
                Self(Some(level))
            } else {
                Self(None)
            }
        }

        const fn value(self) -> Option<i32> {
            self.0
        }
    }

    impl<'a> TryFromBorrowedObject<'a> for Level {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let level: i32 = obj.try_index(vm)?.try_to_primitive(vm)?;
            Ok(Self::new(level))
        }
    }

    struct PyZlibDecompressorInner {
        decompress: backend::ZlibDecompressor,
        unused_data: PyBytesRef,
    }

    #[pyattr]
    #[pyclass(name = "_ZlibDecompressor", traverse)]
    #[derive(PyPayload)]
    struct ZlibDecompressor {
        #[pytraverse(skip)]
        inner: PyMutex<PyZlibDecompressorInner>,
    }

    impl fmt::Debug for ZlibDecompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "zlib._ZlibDecompressor")
        }
    }

    impl Constructor for ZlibDecompressor {
        type Args = DecompressobjArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let decompress =
                backend::ZlibDecompressor::new(args.wbits.value, owned_dict(args.zdict))
                    .map_err(|err| new_init_or_zlib_error(err, vm))?;
            Ok(Self {
                inner: PyMutex::new(PyZlibDecompressorInner {
                    decompress,
                    unused_data: vm.ctx.empty_bytes.clone(),
                }),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl ZlibDecompressor {
        #[pygetset]
        fn eof(&self) -> bool {
            self.inner.lock().decompress.eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.inner.lock().unused_data.clone()
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            self.inner.lock().decompress.needs_input()
        }

        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();

            let mut inner = self.inner.lock();
            let result = inner.decompress.decompress(data, max_length);
            if inner.unused_data.as_bytes() != inner.decompress.unused_data() {
                inner.unused_data = vm.ctx.new_bytes(inner.decompress.unused_data().to_vec());
            }
            result.map_err(|err| match err {
                backend::DecompressError::Zlib(err) => new_zlib_error(err, vm),
                backend::DecompressError::Eof => vm.new_eof_error("End of stream already reached"),
            })
        }
    }
}
