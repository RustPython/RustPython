// spell-checker:ignore compressobj decompressobj zdict chunksize zlibmodule miniz chunker

pub(crate) use zlib::make_module;

#[pymodule]
mod zlib {
    use crate::compression::{
        _decompress, CompressFlushKind, CompressState, CompressStatusKind, Compressor,
        DecompressArgs, DecompressError, DecompressFlushKind, DecompressState, DecompressStatus,
        Decompressor, USE_AFTER_FINISH_ERR, flush_sync,
    };
    use crate::vm::{
        PyObject, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytesRef, PyIntRef, PyTypeRef},
        common::lock::PyMutex,
        convert::{ToPyException, TryFromBorrowedObject},
        function::{ArgBytesLike, ArgPrimitiveIndex, ArgSize, OptionalArg},
        types::Constructor,
    };
    use adler32::RollingAdler32 as Adler32;
    use flate2::{
        Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status,
        write::ZlibEncoder,
    };
    use std::io::Write;

    #[pyattr]
    use libz_sys::{
        Z_BEST_COMPRESSION, Z_BEST_SPEED, Z_BLOCK, Z_DEFAULT_COMPRESSION, Z_DEFAULT_STRATEGY,
        Z_DEFLATED as DEFLATED, Z_FILTERED, Z_FINISH, Z_FIXED, Z_FULL_FLUSH, Z_HUFFMAN_ONLY,
        Z_NO_COMPRESSION, Z_NO_FLUSH, Z_PARTIAL_FLUSH, Z_RLE, Z_SYNC_FLUSH, Z_TREES,
    };

    // we're statically linking libz-rs, so the compile-time and runtime
    // versions will always be the same
    #[pyattr(name = "ZLIB_RUNTIME_VERSION")]
    #[pyattr]
    const ZLIB_VERSION: &str = unsafe {
        match std::ffi::CStr::from_ptr(libz_sys::zlibVersion()).to_str() {
            Ok(s) => s,
            Err(_) => unreachable!(),
        }
    };

    // copied from zlibmodule.c (commit 530f506ac91338)
    #[pyattr]
    const MAX_WBITS: i8 = 15;
    #[pyattr]
    const DEF_BUF_SIZE: usize = 16 * 1024;
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
        wbits: ArgPrimitiveIndex<i8>,
    }

    /// Returns a bytes object containing compressed data.
    #[pyfunction]
    fn compress(args: PyFuncCompressArgs, vm: &VirtualMachine) -> PyResult<PyBytesRef> {
        let PyFuncCompressArgs {
            data,
            level,
            ref wbits,
        } = args;
        let level = level.ok_or_else(|| new_zlib_error("Bad compression level", vm))?;

        let compress = InitOptions::new(wbits.value, vm)?.compress(level);
        let mut encoder = ZlibEncoder::new_with_compress(Vec::new(), compress);
        data.with_ref(|input_bytes| encoder.write_all(input_bytes).unwrap());
        let encoded_bytes = encoder.finish().unwrap();
        Ok(vm.ctx.new_bytes(encoded_bytes))
    }

    enum InitOptions {
        Standard {
            header: bool,
            // [De]Compress::new_with_window_bits is only enabled for zlib; miniz_oxide doesn't
            // support wbits (yet?)
            wbits: u8,
        },
        Gzip {
            wbits: u8,
        },
    }

    impl InitOptions {
        fn new(wbits: i8, vm: &VirtualMachine) -> PyResult<InitOptions> {
            let header = wbits > 0;
            let wbits = wbits.unsigned_abs();
            match wbits {
                // TODO: wbits = 0 should be a valid option:
                // > windowBits can also be zero to request that inflate use the window size in
                // > the zlib header of the compressed stream.
                // but flate2 doesn't expose it
                // 0 => ...
                9..=15 => Ok(InitOptions::Standard { header, wbits }),
                25..=31 => Ok(InitOptions::Gzip { wbits: wbits - 16 }),
                _ => Err(vm.new_value_error("Invalid initialization option".to_owned())),
            }
        }

        fn decompress(self) -> Decompress {
            match self {
                Self::Standard { header, wbits } => Decompress::new_with_window_bits(header, wbits),
                Self::Gzip { wbits } => Decompress::new_gzip(wbits),
            }
        }
        fn compress(self, level: Compression) -> Compress {
            match self {
                Self::Standard { header, wbits } => {
                    Compress::new_with_window_bits(level, header, wbits)
                }
                Self::Gzip { wbits } => Compress::new_gzip(level, wbits),
            }
        }
    }

    #[derive(FromArgs)]
    struct PyFuncDecompressArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i8>,
        #[pyarg(any, default = ArgPrimitiveIndex { value: DEF_BUF_SIZE })]
        bufsize: ArgPrimitiveIndex<usize>,
    }

    /// Returns a bytes object containing the uncompressed data.
    #[pyfunction]
    fn decompress(args: PyFuncDecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let PyFuncDecompressArgs {
            data,
            wbits,
            bufsize,
        } = args;
        data.with_ref(|data| {
            let mut d = InitOptions::new(wbits.value, vm)?.decompress();
            let (buf, stream_end) = _decompress(data, &mut d, bufsize.value, None, flush_sync)
                .map_err(|e| new_zlib_error(e.to_string(), vm))?;
            if !stream_end {
                return Err(new_zlib_error(
                    "Error -5 while decompressing data: incomplete or truncated stream",
                    vm,
                ));
            }
            Ok(buf)
        })
    }

    #[derive(FromArgs)]
    struct DecompressobjArgs {
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i8>,
        #[pyarg(any, optional)]
        zdict: OptionalArg<ArgBytesLike>,
    }

    #[pyfunction]
    fn decompressobj(args: DecompressobjArgs, vm: &VirtualMachine) -> PyResult<PyDecompress> {
        let mut decompress = InitOptions::new(args.wbits.value, vm)?.decompress();
        let zdict = args.zdict.into_option();
        if let Some(dict) = &zdict {
            if args.wbits.value < 0 {
                dict.with_ref(|d| decompress.set_dictionary(d))
                    .map_err(|_| new_zlib_error("failed to set dictionary", vm))?;
            }
        }
        let inner = PyDecompressInner {
            decompress: Some(DecompressWithDict { decompress, zdict }),
            eof: false,
            unused_data: vm.ctx.empty_bytes.clone(),
            unconsumed_tail: vm.ctx.empty_bytes.clone(),
        };
        Ok(PyDecompress {
            inner: PyMutex::new(inner),
        })
    }

    #[derive(Debug)]
    struct PyDecompressInner {
        decompress: Option<DecompressWithDict>,
        eof: bool,
        unused_data: PyBytesRef,
        unconsumed_tail: PyBytesRef,
    }

    #[pyattr]
    #[pyclass(name = "Decompress")]
    #[derive(Debug, PyPayload)]
    struct PyDecompress {
        inner: PyMutex<PyDecompressInner>,
    }

    #[pyclass]
    impl PyDecompress {
        #[pygetset]
        fn eof(&self) -> bool {
            self.inner.lock().eof
        }
        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.inner.lock().unused_data.clone()
        }
        #[pygetset]
        fn unconsumed_tail(&self) -> PyBytesRef {
            self.inner.lock().unconsumed_tail.clone()
        }

        fn decompress_inner(
            inner: &mut PyDecompressInner,
            data: &[u8],
            bufsize: usize,
            max_length: Option<usize>,
            is_flush: bool,
            vm: &VirtualMachine,
        ) -> PyResult<(PyResult<Vec<u8>>, bool)> {
            let Some(d) = &mut inner.decompress else {
                return Err(new_zlib_error(USE_AFTER_FINISH_ERR, vm));
            };

            let prev_in = d.total_in();
            let res = if is_flush {
                // if is_flush: ignore zdict, finish if final chunk
                let calc_flush = |final_chunk| {
                    if final_chunk {
                        FlushDecompress::Finish
                    } else {
                        FlushDecompress::None
                    }
                };
                _decompress(data, &mut d.decompress, bufsize, max_length, calc_flush)
            } else {
                _decompress(data, d, bufsize, max_length, flush_sync)
            }
            .map_err(|e| new_zlib_error(e.to_string(), vm));
            let (ret, stream_end) = match res {
                Ok((buf, stream_end)) => (Ok(buf), stream_end),
                Err(err) => (Err(err), false),
            };
            let consumed = (d.total_in() - prev_in) as usize;

            // save unused input
            let unconsumed = &data[consumed..];
            if !unconsumed.is_empty() {
                if stream_end {
                    let unused = [inner.unused_data.as_bytes(), unconsumed].concat();
                    inner.unused_data = vm.ctx.new_pyref(unused);
                } else {
                    inner.unconsumed_tail = vm.ctx.new_bytes(unconsumed.to_vec());
                }
            } else if !inner.unconsumed_tail.is_empty() {
                inner.unconsumed_tail = vm.ctx.empty_bytes.clone();
            }

            Ok((ret, stream_end))
        }

        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length: usize =
                args.raw_max_length().unwrap_or(0).try_into().map_err(|_| {
                    vm.new_value_error("max_length must be non-negative".to_owned())
                })?;
            let max_length = (max_length != 0).then_some(max_length);
            let data = &*args.data();

            let inner = &mut *self.inner.lock();

            let (ret, stream_end) =
                Self::decompress_inner(inner, data, DEF_BUF_SIZE, max_length, false, vm)?;

            inner.eof |= stream_end;

            ret
        }

        #[pymethod]
        fn flush(&self, length: OptionalArg<ArgSize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let length = match length {
                OptionalArg::Present(ArgSize { value }) if value <= 0 => {
                    return Err(vm.new_value_error("length must be greater than zero".to_owned()));
                }
                OptionalArg::Present(ArgSize { value }) => value as usize,
                OptionalArg::Missing => DEF_BUF_SIZE,
            };

            let inner = &mut *self.inner.lock();
            let data = std::mem::replace(&mut inner.unconsumed_tail, vm.ctx.empty_bytes.clone());

            let (ret, _) = Self::decompress_inner(inner, &data, length, None, true, vm)?;

            if inner.eof {
                inner.decompress = None;
            }

            ret
        }
    }

    #[derive(FromArgs)]
    #[allow(dead_code)] // FIXME: use args
    struct CompressobjArgs {
        #[pyarg(any, default = Level::new(Z_DEFAULT_COMPRESSION))]
        level: Level,
        // only DEFLATED is valid right now, it's w/e
        #[pyarg(any, default = DEFLATED)]
        method: i32,
        #[pyarg(any, default = ArgPrimitiveIndex { value: MAX_WBITS })]
        wbits: ArgPrimitiveIndex<i8>,
        #[pyarg(any, name = "memLevel", default = DEF_MEM_LEVEL)]
        mem_level: u8,
        #[pyarg(any, default = Z_DEFAULT_STRATEGY)]
        strategy: i32,
        #[pyarg(any, optional)]
        zdict: Option<ArgBytesLike>,
    }

    #[pyfunction]
    fn compressobj(args: CompressobjArgs, vm: &VirtualMachine) -> PyResult<PyCompress> {
        let CompressobjArgs {
            level,
            wbits,
            zdict,
            ..
        } = args;
        let level =
            level.ok_or_else(|| vm.new_value_error("invalid initialization option".to_owned()))?;
        #[allow(unused_mut)]
        let mut compress = InitOptions::new(wbits.value, vm)?.compress(level);
        if let Some(zdict) = zdict {
            zdict.with_ref(|zdict| compress.set_dictionary(zdict).unwrap());
        }
        Ok(PyCompress {
            inner: PyMutex::new(CompressState::new(CompressInner::new(compress))),
        })
    }

    #[derive(Debug)]
    struct CompressInner {
        compress: Compress,
    }

    #[pyattr]
    #[pyclass(name = "Compress")]
    #[derive(Debug, PyPayload)]
    struct PyCompress {
        inner: PyMutex<CompressState<CompressInner>>,
    }

    #[pyclass]
    impl PyCompress {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut inner = self.inner.lock();
            data.with_ref(|b| inner.compress(b, vm))
        }

        #[pymethod]
        fn flush(&self, mode: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mode = match mode.unwrap_or(Z_FINISH) {
                Z_NO_FLUSH => return Ok(vec![]),
                Z_PARTIAL_FLUSH => FlushCompress::Partial,
                Z_SYNC_FLUSH => FlushCompress::Sync,
                Z_FULL_FLUSH => FlushCompress::Full,
                Z_FINISH => FlushCompress::Finish,
                _ => return Err(new_zlib_error("invalid mode", vm)),
            };
            self.inner.lock().flush(mode, vm)
        }

        // TODO: This is an optional feature of Compress
        // #[pymethod]
        // #[pymethod(magic)]
        // #[pymethod(name = "__deepcopy__")]
        // fn copy(&self) -> Self {
        //     todo!("<flate2::Compress as Clone>")
        // }
    }

    const CHUNKSIZE: usize = u32::MAX as usize;

    impl CompressInner {
        fn new(compress: Compress) -> Self {
            Self { compress }
        }
    }

    impl CompressStatusKind for Status {
        const OK: Self = Status::Ok;
        const EOF: Self = Status::StreamEnd;

        fn to_usize(self) -> usize {
            self as usize
        }
    }

    impl CompressFlushKind for FlushCompress {
        const NONE: Self = FlushCompress::None;
        const FINISH: Self = FlushCompress::Finish;

        fn to_usize(self) -> usize {
            self as usize
        }
    }

    impl Compressor for CompressInner {
        type Status = Status;
        type Flush = FlushCompress;
        const CHUNKSIZE: usize = CHUNKSIZE;
        const DEF_BUF_SIZE: usize = DEF_BUF_SIZE;

        fn compress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            flush: Self::Flush,
            vm: &VirtualMachine,
        ) -> PyResult<Self::Status> {
            self.compress
                .compress_vec(input, output, flush)
                .map_err(|_| new_zlib_error("error while compressing", vm))
        }

        fn total_in(&mut self) -> usize {
            self.compress.total_in() as usize
        }

        fn new_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
            new_zlib_error(message, vm)
        }
    }

    fn new_zlib_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.class("zlib", "error"), message.into())
    }

    struct Level(Option<flate2::Compression>);

    impl Level {
        fn new(level: i32) -> Self {
            let compression = match level {
                Z_DEFAULT_COMPRESSION => Compression::default(),
                valid_level @ Z_NO_COMPRESSION..=Z_BEST_COMPRESSION => {
                    Compression::new(valid_level as u32)
                }
                _ => return Self(None),
            };
            Self(Some(compression))
        }
        fn ok_or_else(
            self,
            f: impl FnOnce() -> PyBaseExceptionRef,
        ) -> PyResult<flate2::Compression> {
            self.0.ok_or_else(f)
        }
    }

    impl<'a> TryFromBorrowedObject<'a> for Level {
        fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
            let int: i32 = obj.try_index(vm)?.try_to_primitive(vm)?;
            Ok(Self::new(int))
        }
    }

    #[pyattr]
    #[pyclass(name = "_ZlibDecompressor")]
    #[derive(Debug, PyPayload)]
    struct ZlibDecompressor {
        inner: PyMutex<DecompressState<DecompressWithDict>>,
    }

    #[derive(Debug)]
    struct DecompressWithDict {
        decompress: Decompress,
        zdict: Option<ArgBytesLike>,
    }

    impl DecompressStatus for Status {
        fn is_stream_end(&self) -> bool {
            *self == Status::StreamEnd
        }
    }

    impl DecompressFlushKind for FlushDecompress {
        const SYNC: Self = FlushDecompress::Sync;
    }

    impl Decompressor for Decompress {
        type Flush = FlushDecompress;
        type Status = Status;
        type Error = flate2::DecompressError;

        fn total_in(&self) -> u64 {
            self.total_in()
        }
        fn decompress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            flush: Self::Flush,
        ) -> Result<Self::Status, Self::Error> {
            self.decompress_vec(input, output, flush)
        }
    }

    impl Decompressor for DecompressWithDict {
        type Flush = FlushDecompress;
        type Status = Status;
        type Error = flate2::DecompressError;

        fn total_in(&self) -> u64 {
            self.decompress.total_in()
        }
        fn decompress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            flush: Self::Flush,
        ) -> Result<Self::Status, Self::Error> {
            self.decompress.decompress_vec(input, output, flush)
        }
        fn maybe_set_dict(&mut self, err: Self::Error) -> Result<(), Self::Error> {
            let zdict = err.needs_dictionary().and(self.zdict.as_ref()).ok_or(err)?;
            self.decompress.set_dictionary(&zdict.borrow_buf())?;
            Ok(())
        }
    }

    // impl Deconstruct

    impl Constructor for ZlibDecompressor {
        type Args = DecompressobjArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let mut decompress = InitOptions::new(args.wbits.value, vm)?.decompress();
            let zdict = args.zdict.into_option();
            if let Some(dict) = &zdict {
                if args.wbits.value < 0 {
                    dict.with_ref(|d| decompress.set_dictionary(d))
                        .map_err(|_| new_zlib_error("failed to set dictionary", vm))?;
                }
            }
            let inner = DecompressState::new(DecompressWithDict { decompress, zdict }, vm);
            Self {
                inner: PyMutex::new(inner),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(Constructor))]
    impl ZlibDecompressor {
        #[pygetset]
        fn eof(&self) -> bool {
            self.inner.lock().eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.inner.lock().unused_data()
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            self.inner.lock().needs_input()
        }

        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();

            let inner = &mut *self.inner.lock();

            inner
                .decompress(data, max_length, DEF_BUF_SIZE, vm)
                .map_err(|e| match e {
                    DecompressError::Decompress(err) => new_zlib_error(err.to_string(), vm),
                    DecompressError::Eof(err) => err.to_pyexception(vm),
                })
        }

        // TODO: Wait for getstate pyslot to be fixed
        // #[pyslot]
        // fn getstate(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyObject> {
        //     Err(vm.new_type_error("cannot serialize '_ZlibDecompressor' object".to_owned()))
        // }
    }
}
