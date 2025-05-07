// spell-checker:ignore ARMTHUMB

pub(crate) use _lzma::make_module;

#[pymodule]
mod _lzma {
    use crate::compression::{
        CompressFlushKind, CompressState, CompressStatusKind, Compressor, DecompressArgs,
        DecompressError, DecompressState, DecompressStatus, Decompressor,
    };
    #[pyattr]
    use lzma_sys::{
        LZMA_CHECK_CRC32 as CHECK_CRC32, LZMA_CHECK_CRC64 as CHECK_CRC64,
        LZMA_CHECK_NONE as CHECK_NONE, LZMA_CHECK_SHA256 as CHECK_SHA256,
    };
    #[pyattr]
    use lzma_sys::{
        LZMA_FILTER_ARM as FILTER_ARM, LZMA_FILTER_ARMTHUMB as FILTER_ARMTHUMB,
        LZMA_FILTER_IA64 as FILTER_IA64, LZMA_FILTER_LZMA1 as FILTER_LZMA1,
        LZMA_FILTER_LZMA2 as FILTER_LZMA2, LZMA_FILTER_POWERPC as FILTER_POWERPC,
        LZMA_FILTER_SPARC as FILTER_SPARC, LZMA_FILTER_X86 as FILTER_X86,
    };
    #[pyattr]
    use lzma_sys::{
        LZMA_MF_BT2 as MF_BT2, LZMA_MF_BT3 as MF_BT3, LZMA_MF_BT4 as MF_BT4, LZMA_MF_HC3 as MF_HC3,
        LZMA_MF_HC4 as MF_HC4,
    };
    #[pyattr]
    use lzma_sys::{LZMA_MODE_FAST as MODE_FAST, LZMA_MODE_NORMAL as MODE_NORMAL};
    #[pyattr]
    use lzma_sys::{
        LZMA_PRESET_DEFAULT as PRESET_DEFAULT, LZMA_PRESET_EXTREME as PRESET_EXTREME,
        LZMA_PRESET_LEVEL_MASK as PRESET_LEVEL_MASK,
    };
    use rustpython_common::lock::PyMutex;
    use rustpython_vm::builtins::{PyBaseExceptionRef, PyBytesRef, PyTypeRef};
    use rustpython_vm::convert::ToPyException;
    use rustpython_vm::function::ArgBytesLike;
    use rustpython_vm::types::Constructor;
    use rustpython_vm::{PyObjectRef, PyPayload, PyResult, VirtualMachine};
    use std::fmt;
    use xz2::stream::{Action, Check, Error, Filters, Status, Stream};

    #[cfg(windows)]
    type EnumVal = i32;
    #[cfg(not(windows))]
    type EnumVal = u32;

    const BUFSIZ: usize = 8192;
    // TODO: can't find this in lzma-sys, but find way not to hardcode this
    #[pyattr]
    const FILTER_DELTA: i32 = 3;
    #[pyattr]
    const CHECK_UNKNOWN: i32 = 16;

    // the variant ids are hardcoded to be equivalent to the C enum values
    enum Format {
        Auto = 0,
        Xz = 1,
        Alone = 2,
        Raw = 3,
    }

    #[pyattr]
    const FORMAT_AUTO: i32 = Format::Auto as i32;
    #[pyattr]
    const FORMAT_XZ: i32 = Format::Xz as i32;
    #[pyattr]
    const FORMAT_ALONE: i32 = Format::Alone as i32;
    #[pyattr]
    const FORMAT_RAW: i32 = Format::Raw as i32;

    #[pyattr(once, name = "LZMAError")]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "lzma",
            "LZMAError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    fn new_lzma_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.class("lzma", "LZMAError"), message.into())
    }

    #[pyfunction]
    fn is_check_supported(check: i32) -> bool {
        unsafe { lzma_sys::lzma_check_is_supported(check as _) != 0 }
    }

    // TODO: To implement these we need a function to convert a pyobject to a lzma filter and related structs
    #[pyfunction]
    fn _encode_filter_properties() -> PyResult<()> {
        Ok(())
    }

    #[pyfunction]
    fn _decode_filter_properties(_filter_id: u64, _buffer: ArgBytesLike) -> PyResult<()> {
        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "LZMADecompressor")]
    #[derive(PyPayload)]
    struct LZMADecompressor {
        state: PyMutex<DecompressState<Stream>>,
    }

    impl Decompressor for Stream {
        type Flush = ();
        type Status = Status;
        type Error = Error;

        fn total_in(&self) -> u64 {
            self.total_in()
        }
        fn decompress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            (): Self::Flush,
        ) -> Result<Self::Status, Self::Error> {
            self.process_vec(input, output, Action::Run)
        }
    }

    impl DecompressStatus for Status {
        fn is_stream_end(&self) -> bool {
            *self == Status::StreamEnd
        }
    }

    impl fmt::Debug for LZMADecompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_lzma.LZMADecompressor")
        }
    }
    #[derive(FromArgs)]
    pub struct LZMADecompressorConstructorArgs {
        #[pyarg(any, default = FORMAT_AUTO)]
        format: i32,
        #[pyarg(any, optional)]
        memlimit: Option<u64>,
        #[pyarg(any, optional)]
        filters: Option<u32>,
    }

    impl Constructor for LZMADecompressor {
        type Args = LZMADecompressorConstructorArgs;

        fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let memlimit = args.memlimit.unwrap_or(u64::MAX);
            let filters = args.filters.unwrap_or(0);
            let stream_result = match args.format {
                FORMAT_AUTO => Stream::new_auto_decoder(memlimit, filters),
                FORMAT_XZ => Stream::new_stream_decoder(memlimit, filters),
                FORMAT_ALONE => Stream::new_lzma_decoder(memlimit),
                // TODO: FORMAT_RAW
                _ => return Err(new_lzma_error("Invalid format", vm)),
            };
            Self {
                state: PyMutex::new(DecompressState::new(
                    stream_result
                        .map_err(|_| new_lzma_error("Failed to initialize decoder", vm))?,
                    vm,
                )),
            }
            .into_ref_with_type(vm, cls)
            .map(Into::into)
        }
    }

    #[pyclass(with(Constructor))]
    impl LZMADecompressor {
        #[pymethod]
        fn decompress(&self, args: DecompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();

            let mut state = self.state.lock();
            state
                .decompress(data, max_length, BUFSIZ, vm)
                .map_err(|e| match e {
                    DecompressError::Decompress(err) => vm.new_os_error(err.to_string()),
                    DecompressError::Eof(err) => err.to_pyexception(vm),
                })
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.state.lock().eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.state.lock().unused_data()
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            // False if the decompress() method can provide more
            // decompressed data before requiring new uncompressed input.
            self.state.lock().needs_input()
        }

        // TODO: mro()?
    }

    struct CompressorInner {
        stream: Stream,
    }

    impl CompressStatusKind for Status {
        const OK: Self = Status::Ok;
        const EOF: Self = Status::StreamEnd;

        fn to_usize(self) -> usize {
            self as usize
        }
    }

    impl CompressFlushKind for Action {
        const NONE: Self = Action::Run;
        const FINISH: Self = Action::Finish;

        fn to_usize(self) -> usize {
            self as usize
        }
    }

    impl Compressor for CompressorInner {
        type Status = Status;
        type Flush = Action;
        const CHUNKSIZE: usize = u32::MAX as usize;
        const DEF_BUF_SIZE: usize = 16 * 1024;

        fn compress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            flush: Self::Flush,
            vm: &VirtualMachine,
        ) -> PyResult<Self::Status> {
            self.stream
                .process_vec(input, output, flush)
                .map_err(|_| new_lzma_error("Failed to compress data", vm))
        }

        fn total_in(&mut self) -> usize {
            self.stream.total_in() as usize
        }

        fn new_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
            new_lzma_error(message, vm)
        }
    }

    impl CompressorInner {
        fn new(stream: Stream) -> Self {
            Self { stream }
        }
    }

    #[pyattr]
    #[pyclass(name = "LZMACompressor")]
    #[derive(PyPayload)]
    struct LZMACompressor {
        state: PyMutex<CompressState<CompressorInner>>,
    }

    impl fmt::Debug for LZMACompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_lzma.LZMACompressor")
        }
    }

    fn int_to_check(check: i32) -> Option<Check> {
        if check == -1 {
            return Some(Check::None);
        }
        match check as EnumVal {
            CHECK_NONE => Some(Check::None),
            CHECK_CRC32 => Some(Check::Crc32),
            CHECK_CRC64 => Some(Check::Crc64),
            CHECK_SHA256 => Some(Check::Sha256),
            _ => None,
        }
    }

    fn parse_filter_chain_spec(
        filter_specs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Filters> {
        // TODO: don't hardcode
        const LZMA_FILTERS_MAX: usize = 4;
        if filter_specs.len() > LZMA_FILTERS_MAX {
            return Err(new_lzma_error(
                format!("Too many filters - liblzma supports a maximum of {LZMA_FILTERS_MAX}"),
                vm,
            ));
        }
        let filters = Filters::new();
        for _item in filter_specs {}
        Ok(filters)
    }

    impl LZMACompressor {
        fn init_xz(
            check: i32,
            preset: u32,
            filters: Option<Vec<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<Stream> {
            let real_check = int_to_check(check)
                .ok_or_else(|| vm.new_type_error("Invalid check value".to_string()))?;
            if let Some(filters) = filters {
                let filters = parse_filter_chain_spec(filters, vm)?;
                Ok(Stream::new_stream_encoder(&filters, real_check)
                    .map_err(|_| new_lzma_error("Failed to initialize encoder", vm))?)
            } else {
                Ok(Stream::new_easy_encoder(preset, real_check)
                    .map_err(|_| new_lzma_error("Failed to initialize encoder", vm))?)
            }
        }
    }

    #[derive(FromArgs)]
    pub struct LZMACompressorConstructorArgs {
        // format=FORMAT_XZ, check=-1, preset=None, filters=None
        //  {'format': 3, 'filters': [{'id': 3, 'dist': 2}, {'id': 33, 'preset': 2147483654}]}
        #[pyarg(any, default = FORMAT_XZ)]
        format: i32,
        #[pyarg(any, default = -1)]
        check: i32,
        #[pyarg(any, optional)]
        preset: Option<u32>,
        #[pyarg(any, optional)]
        filters: Option<Vec<PyObjectRef>>,
        #[pyarg(any, optional)]
        _filter_specs: Option<Vec<PyObjectRef>>,
        #[pyarg(positional, optional)]
        preset_obj: Option<PyObjectRef>,
    }

    impl Constructor for LZMACompressor {
        type Args = LZMACompressorConstructorArgs;

        fn py_new(_cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            let preset = args.preset.unwrap_or(PRESET_DEFAULT);
            #[allow(clippy::unnecessary_cast)]
            if args.format != FORMAT_XZ as i32
                && args.check != -1
                && args.check != CHECK_NONE as i32
            {
                return Err(new_lzma_error(
                    "Integrity checks are only supported by FORMAT_XZ",
                    vm,
                ));
            }
            if args.preset_obj.is_some() && args.filters.is_some() {
                return Err(new_lzma_error(
                    "Cannot specify both preset and filter chain",
                    vm,
                ));
            }
            let stream = match args.format {
                FORMAT_XZ => Self::init_xz(args.check, preset, args.filters, vm)?,
                // TODO: ALONE AND RAW
                _ => return Err(new_lzma_error("Invalid format", vm)),
            };
            Ok(Self {
                state: PyMutex::new(CompressState::new(CompressorInner::new(stream))),
            }
            .into_pyobject(vm))
        }
    }

    #[pyclass(with(Constructor))]
    impl LZMACompressor {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut state = self.state.lock();
            // TODO: Flush check
            state.compress(&data.borrow_buf(), vm)
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            // TODO: flush check
            let mut state = self.state.lock();
            // TODO: check if action is correct
            state.flush(Action::Finish, vm)
        }
    }
}
