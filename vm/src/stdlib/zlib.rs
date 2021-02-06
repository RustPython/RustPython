pub(crate) use decl::make_module;

#[pymodule(name = "zlib")]
mod decl {
    use crate::builtins::bytes::{PyBytes, PyBytesRef};
    use crate::builtins::int::{self, PyIntRef};
    use crate::builtins::pytype::PyTypeRef;
    use crate::byteslike::PyBytesLike;
    use crate::common::lock::PyMutex;
    use crate::exceptions::PyBaseExceptionRef;
    use crate::function::OptionalArg;
    use crate::pyobject::{BorrowValue, IntoPyRef, PyResult, PyValue, StaticType};
    use crate::types::create_simple_type;
    use crate::vm::VirtualMachine;

    use adler32::RollingAdler32 as Adler32;
    use crc32fast::Hasher as Crc32;
    use crossbeam_utils::atomic::AtomicCell;
    use flate2::{
        write::ZlibEncoder, Compress, Compression, Decompress, FlushCompress, FlushDecompress,
        Status,
    };
    use std::io::Write;

    #[cfg(not(feature = "zlib"))]
    mod constants {
        pub const Z_NO_COMPRESSION: i32 = 0;
        pub const Z_BEST_COMPRESSION: i32 = 9;
        pub const Z_BEST_SPEED: i32 = 1;
        pub const Z_DEFAULT_COMPRESSION: i32 = -1;
        pub const Z_NO_FLUSH: i32 = 0;
        pub const Z_PARTIAL_FLUSH: i32 = 1;
        pub const Z_SYNC_FLUSH: i32 = 2;
        pub const Z_FULL_FLUSH: i32 = 3;
        // not sure what the value here means, but it's the only compression method zlibmodule
        // supports, so it doesn't really matter
        pub const Z_DEFLATED: i32 = 8;
    }
    #[cfg(feature = "zlib")]
    use libz_sys as constants;

    #[pyattr]
    use constants::{
        Z_BEST_COMPRESSION, Z_BEST_SPEED, Z_DEFAULT_COMPRESSION, Z_DEFLATED as DEFLATED,
        Z_FULL_FLUSH, Z_NO_COMPRESSION, Z_NO_FLUSH, Z_PARTIAL_FLUSH, Z_SYNC_FLUSH,
    };

    #[cfg(feature = "zlib")]
    #[pyattr]
    use libz_sys::{
        Z_BLOCK, Z_DEFAULT_STRATEGY, Z_FILTERED, Z_FINISH, Z_FIXED, Z_HUFFMAN_ONLY, Z_RLE, Z_TREES,
    };

    // copied from zlibmodule.c (commit 530f506ac91338)
    #[pyattr]
    const MAX_WBITS: u8 = 15;
    #[pyattr]
    const DEF_BUF_SIZE: usize = 16 * 1024;
    #[pyattr]
    const DEF_MEM_LEVEL: u8 = 8;

    #[pyattr]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        create_simple_type("error", &vm.ctx.exceptions.exception_type)
    }

    /// Compute an Adler-32 checksum of data.
    #[pyfunction]
    fn adler32(data: PyBytesLike, begin_state: OptionalArg<PyIntRef>) -> u32 {
        data.with_ref(|data| {
            let begin_state =
                begin_state.map_or(1, |i| int::bigint_unsigned_mask(i.borrow_value()));

            let mut hasher = Adler32::from_value(begin_state);
            hasher.update_buffer(data);
            hasher.hash()
        })
    }

    /// Compute a CRC-32 checksum of data.
    #[pyfunction]
    fn crc32(data: PyBytesLike, begin_state: OptionalArg<PyIntRef>) -> u32 {
        data.with_ref(|data| {
            let begin_state =
                begin_state.map_or(0, |i| int::bigint_unsigned_mask(i.borrow_value()));

            let mut hasher = Crc32::new_with_initial(begin_state);
            hasher.update(data);
            hasher.finalize()
        })
    }

    fn compression_from_int(level: Option<i32>) -> Option<Compression> {
        match level.unwrap_or(Z_DEFAULT_COMPRESSION) {
            Z_DEFAULT_COMPRESSION => Some(Compression::default()),
            valid_level @ Z_NO_COMPRESSION..=Z_BEST_COMPRESSION => {
                Some(Compression::new(valid_level as u32))
            }
            _ => None,
        }
    }

    /// Returns a bytes object containing compressed data.
    #[pyfunction]
    fn compress(data: PyBytesLike, level: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult {
        let compression = compression_from_int(level.into_option())
            .ok_or_else(|| new_zlib_error("Bad compression level", vm))?;

        let mut encoder = ZlibEncoder::new(Vec::new(), compression);
        data.with_ref(|input_bytes| encoder.write_all(input_bytes).unwrap());
        let encoded_bytes = encoder.finish().unwrap();

        Ok(vm.ctx.new_bytes(encoded_bytes))
    }

    enum InitOptions {
        Standard {
            header: bool,
            // [De]Compress::new_with_window_bits is only enabled for zlib; miniz_oxide doesn't
            // support wbits (yet?)
            #[cfg(feature = "zlib")]
            wbits: u8,
        },
        #[cfg(feature = "zlib")]
        Gzip { wbits: u8 },
    }

    impl InitOptions {
        fn decompress(self) -> Decompress {
            match self {
                #[cfg(not(feature = "zlib"))]
                Self::Standard { header } => Decompress::new(header),
                #[cfg(feature = "zlib")]
                Self::Standard { header, wbits } => Decompress::new_with_window_bits(header, wbits),
                #[cfg(feature = "zlib")]
                Self::Gzip { wbits } => Decompress::new_gzip(wbits),
            }
        }
        fn compress(self, level: Compression) -> Compress {
            match self {
                #[cfg(not(feature = "zlib"))]
                Self::Standard { header } => Compress::new(level, header),
                #[cfg(feature = "zlib")]
                Self::Standard { header, wbits } => {
                    Compress::new_with_window_bits(level, header, wbits)
                }
                #[cfg(feature = "zlib")]
                Self::Gzip { wbits } => Compress::new_gzip(level, wbits),
            }
        }
    }

    fn header_from_wbits(wbits: OptionalArg<i8>, vm: &VirtualMachine) -> PyResult<InitOptions> {
        let wbits = wbits.unwrap_or(MAX_WBITS as i8);
        let header = wbits > 0;
        let wbits = wbits.abs() as u8;
        match wbits {
            9..=15 => Ok(InitOptions::Standard {
                header,
                #[cfg(feature = "zlib")]
                wbits,
            }),
            #[cfg(feature = "zlib")]
            25..=31 => Ok(InitOptions::Gzip { wbits: wbits - 16 }),
            _ => Err(vm.new_value_error("Invalid initialization option".to_owned())),
        }
    }

    fn _decompress(
        mut data: &[u8],
        d: &mut Decompress,
        bufsize: usize,
        max_length: Option<usize>,
        is_flush: bool,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<u8>, bool)> {
        if data.is_empty() {
            return Ok((Vec::new(), true));
        }
        let mut buf = Vec::new();

        loop {
            let final_chunk = data.len() <= CHUNKSIZE;
            let chunk = if final_chunk {
                data
            } else {
                &data[..CHUNKSIZE]
            };
            // if this is the final chunk, finish it
            let flush = if is_flush {
                if final_chunk {
                    FlushDecompress::Finish
                } else {
                    FlushDecompress::None
                }
            } else {
                FlushDecompress::Sync
            };
            loop {
                let additional = if let Some(max_length) = max_length {
                    std::cmp::min(bufsize, max_length - buf.capacity())
                } else {
                    bufsize
                };
                if additional == 0 {
                    return Ok((buf, false));
                }

                buf.reserve_exact(additional);
                let prev_in = d.total_in();
                let status = d
                    .decompress_vec(chunk, &mut buf, flush)
                    .map_err(|_| new_zlib_error("invalid input data", vm))?;
                let consumed = d.total_in() - prev_in;
                data = &data[consumed as usize..];
                let stream_end = status == Status::StreamEnd;
                if stream_end || data.is_empty() {
                    // we've reached the end of the stream, we're done
                    buf.shrink_to_fit();
                    return Ok((buf, stream_end));
                } else if !chunk.is_empty() && consumed == 0 {
                    // we're gonna need a bigger buffer
                    continue;
                } else {
                    // next chunk
                    break;
                }
            }
        }
    }

    /// Returns a bytes object containing the uncompressed data.
    #[pyfunction]
    fn decompress(
        data: PyBytesLike,
        wbits: OptionalArg<i8>,
        bufsize: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        data.with_ref(|data| {
            let bufsize = bufsize.unwrap_or(DEF_BUF_SIZE);

            let mut d = header_from_wbits(wbits, vm)?.decompress();

            _decompress(data, &mut d, bufsize, None, false, vm).and_then(|(buf, stream_end)| {
                if stream_end {
                    Ok(buf)
                } else {
                    Err(new_zlib_error("incomplete or truncated stream", vm))
                }
            })
        })
    }

    #[pyfunction]
    fn decompressobj(args: DecompressobjArgs, vm: &VirtualMachine) -> PyResult<PyDecompress> {
        #[allow(unused_mut)]
        let mut decompress = header_from_wbits(args.wbits, vm)?.decompress();
        #[cfg(feature = "zlib")]
        if let OptionalArg::Present(dict) = args.zdict {
            dict.with_ref(|d| decompress.set_dictionary(d).unwrap());
        }
        Ok(PyDecompress {
            decompress: PyMutex::new(decompress),
            eof: AtomicCell::new(false),
            unused_data: PyMutex::new(PyBytes::from(vec![]).into_ref(vm)),
            unconsumed_tail: PyMutex::new(PyBytes::from(vec![]).into_ref(vm)),
        })
    }
    #[pyattr]
    #[pyclass(name = "Decompress")]
    #[derive(Debug)]
    struct PyDecompress {
        decompress: PyMutex<Decompress>,
        eof: AtomicCell<bool>,
        unused_data: PyMutex<PyBytesRef>,
        unconsumed_tail: PyMutex<PyBytesRef>,
    }
    impl PyValue for PyDecompress {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }
    #[pyimpl]
    impl PyDecompress {
        #[pyproperty]
        fn eof(&self) -> bool {
            self.eof.load()
        }
        #[pyproperty]
        fn unused_data(&self) -> PyBytesRef {
            self.unused_data.lock().clone()
        }
        #[pyproperty]
        fn unconsumed_tail(&self) -> PyBytesRef {
            self.unconsumed_tail.lock().clone()
        }

        fn save_unused_input(
            &self,
            d: &mut Decompress,
            data: &[u8],
            stream_end: bool,
            orig_in: u64,
            vm: &VirtualMachine,
        ) {
            let leftover = &data[(d.total_in() - orig_in) as usize..];

            if stream_end && !leftover.is_empty() {
                let mut unused_data = self.unused_data.lock();
                let unused: Vec<_> = unused_data
                    .borrow_value()
                    .iter()
                    .chain(leftover)
                    .copied()
                    .collect();
                *unused_data = unused.into_pyref(vm);
            }
        }

        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = if args.max_length == 0 {
                None
            } else {
                Some(args.max_length)
            };
            let data = args.data.borrow_value();
            let data = &*data;

            let mut d = self.decompress.lock();
            let orig_in = d.total_in();

            let (ret, stream_end) =
                match _decompress(data, &mut d, DEF_BUF_SIZE, max_length, false, vm) {
                    Ok((buf, true)) => {
                        self.eof.store(true);
                        (Ok(buf), true)
                    }
                    Ok((buf, false)) => (Ok(buf), false),
                    Err(err) => (Err(err), false),
                };
            self.save_unused_input(&mut d, data, stream_end, orig_in, vm);

            let leftover = if stream_end {
                b""
            } else {
                &data[(d.total_in() - orig_in) as usize..]
            };

            let mut unconsumed_tail = self.unconsumed_tail.lock();
            if !leftover.is_empty() || !unconsumed_tail.is_empty() {
                *unconsumed_tail = PyBytes::from(leftover.to_owned()).into_ref(vm);
            }

            ret
        }

        #[pymethod]
        fn flush(&self, length: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let length = match length {
                OptionalArg::Present(l) => {
                    if l <= 0 {
                        return Err(
                            vm.new_value_error("length must be greater than zero".to_owned())
                        );
                    } else {
                        l as usize
                    }
                }
                OptionalArg::Missing => DEF_BUF_SIZE,
            };

            let mut data = self.unconsumed_tail.lock();
            let mut d = self.decompress.lock();

            let orig_in = d.total_in();

            let (ret, stream_end) = match _decompress(&data, &mut d, length, None, true, vm) {
                Ok((buf, stream_end)) => (Ok(buf), stream_end),
                Err(err) => (Err(err), false),
            };
            self.save_unused_input(&mut d, &data, stream_end, orig_in, vm);

            *data = PyBytes::from(Vec::new()).into_ref(vm);

            // TODO: drop the inner decompressor, somehow
            // if stream_end {
            //
            // }
            ret
        }
    }

    #[derive(FromArgs)]
    struct DecompressArgs {
        #[pyarg(positional)]
        data: PyBytesLike,
        #[pyarg(any, default = "0")]
        max_length: usize,
    }

    #[derive(FromArgs)]
    struct DecompressobjArgs {
        #[pyarg(any, optional)]
        wbits: OptionalArg<i8>,
        #[cfg(feature = "zlib")]
        #[pyarg(any, optional)]
        zdict: OptionalArg<PyBytesLike>,
    }

    #[pyfunction]
    fn compressobj(
        level: OptionalArg<i32>,
        // only DEFLATED is valid right now, it's w/e
        _method: OptionalArg<i32>,
        wbits: OptionalArg<i8>,
        // these aren't used.
        _mem_level: OptionalArg<i32>, // this is memLevel in CPython
        _strategy: OptionalArg<i32>,
        _zdict: OptionalArg<PyBytesLike>,
        vm: &VirtualMachine,
    ) -> PyResult<PyCompress> {
        let level = compression_from_int(level.into_option())
            .ok_or_else(|| vm.new_value_error("invalid initialization option".to_owned()))?;
        let compress = header_from_wbits(wbits, vm)?.compress(level);
        Ok(PyCompress {
            inner: PyMutex::new(CompressInner {
                compress,
                unconsumed: Vec::new(),
            }),
        })
    }

    #[derive(Debug)]
    struct CompressInner {
        compress: Compress,
        unconsumed: Vec<u8>,
    }

    #[pyattr]
    #[pyclass(name = "Compress")]
    #[derive(Debug)]
    struct PyCompress {
        inner: PyMutex<CompressInner>,
    }

    impl PyValue for PyCompress {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl PyCompress {
        #[pymethod]
        fn compress(&self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut inner = self.inner.lock();
            data.with_ref(|b| inner.compress(b, vm))
        }

        // TODO: mode argument isn't used
        #[pymethod]
        fn flush(&self, _mode: OptionalArg<i32>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.inner.lock().flush(vm)
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
        fn save_unconsumed_input(&mut self, data: &[u8], orig_in: u64) {
            let leftover = &data[(self.compress.total_in() - orig_in) as usize..];
            self.unconsumed.extend_from_slice(leftover);
        }

        fn compress(&mut self, data: &[u8], vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let orig_in = self.compress.total_in();
            let unconsumed = std::mem::take(&mut self.unconsumed);
            let mut buf = Vec::new();

            'outer: for chunk in unconsumed.chunks(CHUNKSIZE).chain(data.chunks(CHUNKSIZE)) {
                loop {
                    buf.reserve(DEF_BUF_SIZE);
                    let status = self
                        .compress
                        .compress_vec(chunk, &mut buf, FlushCompress::None)
                        .map_err(|_| {
                            self.save_unconsumed_input(data, orig_in);
                            new_zlib_error("error while compressing", vm)
                        })?;
                    match status {
                        _ if buf.len() == buf.capacity() => continue,
                        Status::StreamEnd => break 'outer,
                        _ => break,
                    }
                }
            }
            self.save_unconsumed_input(data, orig_in);

            buf.shrink_to_fit();
            Ok(buf)
        }

        // TODO: flush mode (FlushDecompress) parameter
        fn flush(&mut self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let data = std::mem::take(&mut self.unconsumed);
            let mut data_it = data.chunks(CHUNKSIZE);
            let mut buf = Vec::new();

            loop {
                let chunk = data_it.next().unwrap_or(&[]);
                if buf.len() == buf.capacity() {
                    buf.reserve(DEF_BUF_SIZE);
                }
                let status = self
                    .compress
                    .compress_vec(chunk, &mut buf, FlushCompress::Finish)
                    .map_err(|_| new_zlib_error("error while compressing", vm))?;
                match status {
                    Status::StreamEnd => break,
                    _ => continue,
                }
            }

            buf.shrink_to_fit();
            Ok(buf)
        }
    }

    fn new_zlib_error(message: &str, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.class("zlib", "error"), message.to_owned())
    }
}
