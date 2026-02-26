// spell-checker:ignore ARMTHUMB memlimit

pub(crate) use _lzma::module_def;

#[pymodule]
mod _lzma {
    use crate::compression::{
        CompressFlushKind, CompressState, CompressStatusKind, Compressor, DecompressArgs,
        DecompressError, DecompressState, DecompressStatus, Decompressor,
    };
    use alloc::fmt;
    use liblzma::stream::{
        Action, Check, Error, Filters, LzmaOptions, MatchFinder, Mode, Status, Stream,
        TELL_ANY_CHECK, TELL_NO_CHECK,
    };
    // lzma_check, lzma_mode, lzma_match_finder have platform-dependent signedness
    // (i32 on Windows, u32 elsewhere). Define as fixed-type const to avoid mismatch.
    #[pyattr]
    use liblzma_sys::{
        LZMA_FILTER_ARM as FILTER_ARM, LZMA_FILTER_ARMTHUMB as FILTER_ARMTHUMB,
        LZMA_FILTER_DELTA as FILTER_DELTA, LZMA_FILTER_IA64 as FILTER_IA64,
        LZMA_FILTER_LZMA1 as FILTER_LZMA1, LZMA_FILTER_LZMA2 as FILTER_LZMA2,
        LZMA_FILTER_POWERPC as FILTER_POWERPC, LZMA_FILTER_SPARC as FILTER_SPARC,
        LZMA_FILTER_X86 as FILTER_X86,
    };
    #[pyattr]
    use liblzma_sys::{
        LZMA_PRESET_DEFAULT as PRESET_DEFAULT, LZMA_PRESET_EXTREME as PRESET_EXTREME,
    };
    use rustpython_common::lock::PyMutex;
    use rustpython_vm::builtins::{PyBaseExceptionRef, PyBytesRef, PyDict, PyType, PyTypeRef};
    use rustpython_vm::convert::ToPyException;
    use rustpython_vm::function::ArgBytesLike;
    use rustpython_vm::types::Constructor;
    use rustpython_vm::{Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};

    const BUFSIZ: usize = 8192;

    // liblzma_sys enum types have platform-dependent signedness; `as _` normalizes to i32
    #[pyattr]
    const CHECK_NONE: i32 = liblzma_sys::LZMA_CHECK_NONE as _;
    #[pyattr]
    const CHECK_CRC32: i32 = liblzma_sys::LZMA_CHECK_CRC32 as _;
    #[pyattr]
    const CHECK_CRC64: i32 = liblzma_sys::LZMA_CHECK_CRC64 as _;
    #[pyattr]
    const CHECK_SHA256: i32 = liblzma_sys::LZMA_CHECK_SHA256 as _;
    #[pyattr]
    const CHECK_ID_MAX: i32 = 15;
    #[pyattr]
    const CHECK_UNKNOWN: i32 = CHECK_ID_MAX + 1;

    #[pyattr]
    const MF_HC3: i32 = liblzma_sys::LZMA_MF_HC3 as _;
    #[pyattr]
    const MF_HC4: i32 = liblzma_sys::LZMA_MF_HC4 as _;
    #[pyattr]
    const MF_BT2: i32 = liblzma_sys::LZMA_MF_BT2 as _;
    #[pyattr]
    const MF_BT3: i32 = liblzma_sys::LZMA_MF_BT3 as _;
    #[pyattr]
    const MF_BT4: i32 = liblzma_sys::LZMA_MF_BT4 as _;

    #[pyattr]
    const MODE_FAST: i32 = liblzma_sys::LZMA_MODE_FAST as _;
    #[pyattr]
    const MODE_NORMAL: i32 = liblzma_sys::LZMA_MODE_NORMAL as _;

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
        let msg: String = message.into();
        vm.new_exception_msg(vm.class("lzma", "LZMAError"), msg.into())
    }

    fn catch_lzma_error(err: Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match err {
            Error::UnsupportedCheck => new_lzma_error("Unsupported integrity check", vm),
            Error::Mem => vm.new_memory_error(""),
            Error::MemLimit => new_lzma_error("Memory usage limit exceeded", vm),
            Error::Format => new_lzma_error("Input format not supported by decoder", vm),
            Error::Options => new_lzma_error("Invalid or unsupported options", vm),
            Error::Data => new_lzma_error("Corrupt input data", vm),
            Error::Program => new_lzma_error("Internal error", vm),
            Error::NoCheck => new_lzma_error("Corrupt input data", vm),
        }
    }

    fn int_to_check(check: i32) -> Option<Check> {
        if check == -1 {
            return Some(Check::Crc64);
        }
        match check {
            CHECK_NONE => Some(Check::None),
            CHECK_CRC32 => Some(Check::Crc32),
            CHECK_CRC64 => Some(Check::Crc64),
            CHECK_SHA256 => Some(Check::Sha256),
            _ => None,
        }
    }

    fn u32_to_mode(val: u32) -> Option<Mode> {
        match val as i32 {
            MODE_FAST => Some(Mode::Fast),
            MODE_NORMAL => Some(Mode::Normal),
            _ => None,
        }
    }

    fn u32_to_mf(val: u32) -> Option<MatchFinder> {
        match val as i32 {
            MF_HC3 => Some(MatchFinder::HashChain3),
            MF_HC4 => Some(MatchFinder::HashChain4),
            MF_BT2 => Some(MatchFinder::BinaryTree2),
            MF_BT3 => Some(MatchFinder::BinaryTree3),
            MF_BT4 => Some(MatchFinder::BinaryTree4),
            _ => None,
        }
    }

    struct LzmaStream {
        stream: Stream,
        check: i32,
        header_buf: [u8; 8],
        header_collected: u8,
        track_header: bool,
    }

    impl LzmaStream {
        fn new(stream: Stream, check: i32, track_header: bool) -> Self {
            Self {
                stream,
                check,
                header_buf: [0u8; 8],
                header_collected: 0,
                track_header,
            }
        }
    }

    impl Decompressor for LzmaStream {
        type Flush = ();
        type Status = Status;
        type Error = Error;

        fn total_in(&self) -> u64 {
            self.stream.total_in()
        }

        fn decompress_vec(
            &mut self,
            input: &[u8],
            output: &mut Vec<u8>,
            (): Self::Flush,
        ) -> Result<Self::Status, Self::Error> {
            if self.track_header && self.header_collected < 8 {
                let need = (8 - self.header_collected) as usize;
                let n = need.min(input.len());
                self.header_buf[self.header_collected as usize..][..n].copy_from_slice(&input[..n]);
                self.header_collected += n as u8;
            }

            match self.stream.process_vec(input, output, Action::Run) {
                Ok(Status::GetCheck) => {
                    if self.header_collected >= 8 {
                        self.check = (self.header_buf[7] & 0x0F) as i32;
                    }
                    Ok(Status::Ok)
                }
                Err(Error::NoCheck) => {
                    self.check = CHECK_NONE;
                    Ok(Status::Ok)
                }
                other => other,
            }
        }
    }

    impl DecompressStatus for Status {
        fn is_stream_end(&self) -> bool {
            *self == Status::StreamEnd
        }
    }

    fn get_dict_opt_u32(
        spec: &PyObjectRef,
        key: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Option<u32>> {
        let dict = spec.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error("Filter specifier must be a dict or dict-like object")
        })?;
        match dict.get_item_opt(key, vm)? {
            Some(obj) => Ok(Some(obj.try_into_value::<u32>(vm)?)),
            None => Ok(None),
        }
    }

    fn get_dict_opt_u64(
        spec: &PyObjectRef,
        key: &str,
        vm: &VirtualMachine,
    ) -> PyResult<Option<u64>> {
        let dict = spec.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error("Filter specifier must be a dict or dict-like object")
        })?;
        match dict.get_item_opt(key, vm)? {
            Some(obj) => Ok(Some(obj.try_into_value::<u64>(vm)?)),
            None => Ok(None),
        }
    }

    fn parse_filter_spec_lzma(spec: &PyObjectRef, vm: &VirtualMachine) -> PyResult<LzmaOptions> {
        let preset = get_dict_opt_u32(spec, "preset", vm)?.unwrap_or(PRESET_DEFAULT);

        let mut opts = LzmaOptions::new_preset(preset)
            .map_err(|_| new_lzma_error(format!("Invalid compression preset: {preset}"), vm))?;

        if let Some(v) = get_dict_opt_u32(spec, "dict_size", vm)? {
            opts.dict_size(v);
        }
        if let Some(v) = get_dict_opt_u32(spec, "lc", vm)? {
            opts.literal_context_bits(v);
        }
        if let Some(v) = get_dict_opt_u32(spec, "lp", vm)? {
            opts.literal_position_bits(v);
        }
        if let Some(v) = get_dict_opt_u32(spec, "pb", vm)? {
            opts.position_bits(v);
        }
        if let Some(v) = get_dict_opt_u32(spec, "mode", vm)? {
            let mode = u32_to_mode(v)
                .ok_or_else(|| vm.new_value_error("Invalid filter specifier for LZMA filter"))?;
            opts.mode(mode);
        }
        if let Some(v) = get_dict_opt_u32(spec, "nice_len", vm)? {
            opts.nice_len(v);
        }
        if let Some(v) = get_dict_opt_u32(spec, "mf", vm)? {
            let mf = u32_to_mf(v)
                .ok_or_else(|| vm.new_value_error("Invalid filter specifier for LZMA filter"))?;
            opts.match_finder(mf);
        }
        if let Some(v) = get_dict_opt_u32(spec, "depth", vm)? {
            opts.depth(v);
        }

        Ok(opts)
    }

    fn parse_filter_spec_delta(spec: &PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        let dist = get_dict_opt_u32(spec, "dist", vm)?.unwrap_or(1);
        if dist == 0 || dist > 256 {
            return Err(vm.new_value_error("Invalid filter specifier for delta filter"));
        }
        Ok(dist)
    }

    fn parse_filter_spec_bcj(spec: &PyObjectRef, vm: &VirtualMachine) -> PyResult<u32> {
        Ok(get_dict_opt_u32(spec, "start_offset", vm)?.unwrap_or(0))
    }

    fn add_bcj_filter(
        filters: &mut Filters,
        filter_id: u64,
        start_offset: u32,
    ) -> Result<(), Error> {
        if start_offset == 0 {
            match filter_id {
                FILTER_X86 => {
                    filters.x86();
                }
                FILTER_POWERPC => {
                    filters.powerpc();
                }
                FILTER_IA64 => {
                    filters.ia64();
                }
                FILTER_ARM => {
                    filters.arm();
                }
                FILTER_ARMTHUMB => {
                    filters.arm_thumb();
                }
                FILTER_SPARC => {
                    filters.sparc();
                }
                _ => unreachable!(),
            }
            Ok(())
        } else {
            let props = start_offset.to_le_bytes();
            match filter_id {
                FILTER_X86 => {
                    filters.x86_properties(&props)?;
                }
                FILTER_POWERPC => {
                    filters.powerpc_properties(&props)?;
                }
                FILTER_IA64 => {
                    filters.ia64_properties(&props)?;
                }
                FILTER_ARM => {
                    filters.arm_properties(&props)?;
                }
                FILTER_ARMTHUMB => {
                    filters.arm_thumb_properties(&props)?;
                }
                FILTER_SPARC => {
                    filters.sparc_properties(&props)?;
                }
                _ => unreachable!(),
            }
            Ok(())
        }
    }

    fn parse_filter_chain_spec(
        filter_specs: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Filters> {
        const LZMA_FILTERS_MAX: usize = 4;
        if filter_specs.len() > LZMA_FILTERS_MAX {
            return Err(new_lzma_error(
                format!("Too many filters - liblzma supports a maximum of {LZMA_FILTERS_MAX}"),
                vm,
            ));
        }

        let mut filters = Filters::new();
        for spec in &filter_specs {
            let filter_id = get_dict_opt_u64(spec, "id", vm)?
                .ok_or_else(|| vm.new_value_error("Filter specifier must have an \"id\" entry"))?;

            match filter_id {
                FILTER_LZMA1 => {
                    let opts = parse_filter_spec_lzma(spec, vm)?;
                    filters.lzma1(&opts);
                }
                FILTER_LZMA2 => {
                    let opts = parse_filter_spec_lzma(spec, vm)?;
                    filters.lzma2(&opts);
                }
                FILTER_DELTA => {
                    let dist = parse_filter_spec_delta(spec, vm)?;
                    filters
                        .delta_properties(&[(dist - 1) as u8])
                        .map_err(|e| catch_lzma_error(e, vm))?;
                }
                FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
                | FILTER_SPARC => {
                    let start_offset = parse_filter_spec_bcj(spec, vm)?;
                    add_bcj_filter(&mut filters, filter_id, start_offset)
                        .map_err(|e| catch_lzma_error(e, vm))?;
                }
                _ => {
                    return Err(vm.new_value_error(format!("Invalid filter ID: {filter_id}")));
                }
            }
        }

        Ok(filters)
    }

    const DEFAULT_LC: u32 = liblzma_sys::LZMA_LC_DEFAULT;
    const DEFAULT_LP: u32 = liblzma_sys::LZMA_LP_DEFAULT;
    const DEFAULT_PB: u32 = liblzma_sys::LZMA_PB_DEFAULT;
    const DICT_POW2: [u8; 10] = [18, 20, 21, 22, 22, 23, 23, 24, 25, 26];

    fn preset_dict_size(preset: u32) -> u32 {
        let level = (preset & liblzma_sys::LZMA_PRESET_LEVEL_MASK) as usize;
        if level > 9 {
            return 0;
        }
        1u32 << DICT_POW2[level]
    }

    fn lzma2_dict_size_from_prop(prop: u8) -> u32 {
        if prop > 40 {
            return u32::MAX;
        }
        if prop == 40 {
            return u32::MAX;
        }
        let prop = prop as u32;
        (2 | (prop & 1)) << (prop / 2 + 11)
    }

    fn lzma2_prop_from_dict_size(dict_size: u32) -> u8 {
        if dict_size == u32::MAX {
            return 40;
        }
        for i in 0u8..40 {
            if lzma2_dict_size_from_prop(i) >= dict_size {
                return i;
            }
        }
        40
    }

    fn encode_lzma1_properties(lc: u32, lp: u32, pb: u32, dict_size: u32) -> Vec<u8> {
        let mut result = vec![0u8; 5];
        result[0] = ((pb * 5 + lp) * 9 + lc) as u8;
        result[1..5].copy_from_slice(&dict_size.to_le_bytes());
        result
    }

    fn decode_lzma1_properties(props: &[u8]) -> Option<(u32, u32, u32, u32)> {
        if props.len() < 5 {
            return None;
        }
        let mut d = props[0] as u32;
        let lc = d % 9;
        d /= 9;
        let lp = d % 5;
        let pb = d / 5;
        let dict_size = u32::from_le_bytes([props[1], props[2], props[3], props[4]]);
        Some((lc, lp, pb, dict_size))
    }

    fn build_filter_spec(
        filter_id: u64,
        props: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let dict = vm.ctx.new_dict();
        dict.set_item("id", vm.new_pyobj(filter_id), vm)?;

        match filter_id {
            FILTER_LZMA1 => {
                let (lc, lp, pb, dict_size) = decode_lzma1_properties(props)
                    .ok_or_else(|| new_lzma_error("Invalid or unsupported options", vm))?;
                dict.set_item("lc", vm.new_pyobj(lc), vm)?;
                dict.set_item("lp", vm.new_pyobj(lp), vm)?;
                dict.set_item("pb", vm.new_pyobj(pb), vm)?;
                dict.set_item("dict_size", vm.new_pyobj(dict_size), vm)?;
            }
            FILTER_LZMA2 => {
                if props.len() != 1 {
                    return Err(new_lzma_error("Invalid or unsupported options", vm));
                }
                let dict_size = lzma2_dict_size_from_prop(props[0]);
                dict.set_item("dict_size", vm.new_pyobj(dict_size), vm)?;
            }
            FILTER_DELTA => {
                if props.len() != 1 {
                    return Err(new_lzma_error("Invalid or unsupported options", vm));
                }
                let dist = props[0] as u32 + 1;
                dict.set_item("dist", vm.new_pyobj(dist), vm)?;
            }
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                if props.is_empty() {
                    // default: no start_offset
                } else if props.len() == 4 {
                    let start_offset = u32::from_le_bytes([props[0], props[1], props[2], props[3]]);
                    dict.set_item("start_offset", vm.new_pyobj(start_offset), vm)?;
                } else {
                    return Err(new_lzma_error("Invalid or unsupported options", vm));
                }
            }
            _ => {
                return Err(vm.new_value_error(format!("Invalid filter ID: {filter_id}")));
            }
        }

        Ok(dict.into())
    }

    #[pyfunction]
    fn is_check_supported(check_id: i32) -> bool {
        unsafe { liblzma_sys::lzma_check_is_supported(check_id as _) != 0 }
    }

    #[pyfunction]
    fn _encode_filter_properties(
        filter_spec: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let filter_id = get_dict_opt_u64(&filter_spec, "id", vm)?
            .ok_or_else(|| vm.new_value_error("Filter specifier must have an \"id\" entry"))?;

        match filter_id {
            FILTER_LZMA1 => {
                let preset =
                    get_dict_opt_u32(&filter_spec, "preset", vm)?.unwrap_or(PRESET_DEFAULT);
                let lc = get_dict_opt_u32(&filter_spec, "lc", vm)?.unwrap_or(DEFAULT_LC);
                let lp = get_dict_opt_u32(&filter_spec, "lp", vm)?.unwrap_or(DEFAULT_LP);
                let pb = get_dict_opt_u32(&filter_spec, "pb", vm)?.unwrap_or(DEFAULT_PB);
                let dict_size = get_dict_opt_u32(&filter_spec, "dict_size", vm)?
                    .unwrap_or_else(|| preset_dict_size(preset));
                Ok(encode_lzma1_properties(lc, lp, pb, dict_size))
            }
            FILTER_LZMA2 => {
                let preset =
                    get_dict_opt_u32(&filter_spec, "preset", vm)?.unwrap_or(PRESET_DEFAULT);
                let dict_size = get_dict_opt_u32(&filter_spec, "dict_size", vm)?
                    .unwrap_or_else(|| preset_dict_size(preset));
                Ok(vec![lzma2_prop_from_dict_size(dict_size)])
            }
            FILTER_DELTA => {
                let dist = get_dict_opt_u32(&filter_spec, "dist", vm)?.unwrap_or(1);
                if dist == 0 || dist > 256 {
                    return Err(vm.new_value_error("Invalid filter specifier for delta filter"));
                }
                Ok(vec![(dist - 1) as u8])
            }
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                let start_offset = get_dict_opt_u32(&filter_spec, "start_offset", vm)?.unwrap_or(0);
                if start_offset == 0 {
                    Ok(vec![])
                } else {
                    Ok(start_offset.to_le_bytes().to_vec())
                }
            }
            _ => Err(vm.new_value_error(format!("Invalid filter ID: {filter_id}"))),
        }
    }

    #[pyfunction]
    fn _decode_filter_properties(
        filter_id: u64,
        encoded_props: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let props = encoded_props.borrow_buf();
        build_filter_spec(filter_id, &props, vm)
    }

    #[pyattr]
    #[pyclass(name = "LZMADecompressor")]
    #[derive(PyPayload)]
    struct LZMADecompressor {
        state: PyMutex<DecompressState<LzmaStream>>,
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
        filters: Option<Vec<PyObjectRef>>,
    }

    impl Constructor for LZMADecompressor {
        type Args = LZMADecompressorConstructorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.format == FORMAT_RAW && args.memlimit.is_some() {
                return Err(vm.new_value_error("Cannot specify memory limit with FORMAT_RAW"));
            }

            if args.format == FORMAT_RAW && args.filters.is_none() {
                return Err(vm.new_value_error("Must specify filters for FORMAT_RAW"));
            }
            if args.format != FORMAT_RAW && args.filters.is_some() {
                return Err(vm.new_value_error("Cannot specify filters except with FORMAT_RAW"));
            }

            let memlimit = args.memlimit.unwrap_or(u64::MAX);
            let decoder_flags = TELL_ANY_CHECK | TELL_NO_CHECK;

            let lzma_stream = match args.format {
                FORMAT_AUTO => {
                    let stream = Stream::new_auto_decoder(memlimit, decoder_flags)
                        .map_err(|e| catch_lzma_error(e, vm))?;
                    LzmaStream::new(stream, CHECK_UNKNOWN, true)
                }
                FORMAT_XZ => {
                    let stream = Stream::new_stream_decoder(memlimit, decoder_flags)
                        .map_err(|e| catch_lzma_error(e, vm))?;
                    LzmaStream::new(stream, CHECK_UNKNOWN, true)
                }
                FORMAT_ALONE => {
                    let stream =
                        Stream::new_lzma_decoder(memlimit).map_err(|e| catch_lzma_error(e, vm))?;
                    LzmaStream::new(stream, CHECK_NONE, false)
                }
                FORMAT_RAW => {
                    let filter_specs = args.filters.unwrap(); // safe: checked above
                    let filters = parse_filter_chain_spec(filter_specs, vm)?;
                    let stream =
                        Stream::new_raw_decoder(&filters).map_err(|e| catch_lzma_error(e, vm))?;
                    LzmaStream::new(stream, CHECK_NONE, false)
                }
                _ => {
                    return Err(
                        vm.new_value_error(format!("Invalid container format: {}", args.format))
                    );
                }
            };

            Ok(Self {
                state: PyMutex::new(DecompressState::new(lzma_stream, vm)),
            })
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
                    DecompressError::Decompress(err) => catch_lzma_error(err, vm),
                    DecompressError::Eof(err) => err.to_pyexception(vm),
                })
        }

        #[pygetset]
        fn check(&self) -> i32 {
            self.state.lock().decompressor().check
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
            self.state.lock().needs_input()
        }
    }

    struct CompressorInner {
        stream: Stream,
    }

    impl CompressStatusKind for Status {
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
                .map_err(|e| catch_lzma_error(e, vm))
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

    impl LZMACompressor {
        fn init_xz(
            check: i32,
            preset: u32,
            filters: Option<Vec<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<Stream> {
            let real_check =
                int_to_check(check).ok_or_else(|| vm.new_value_error("Invalid check value"))?;
            if let Some(filter_specs) = filters {
                let filters = parse_filter_chain_spec(filter_specs, vm)?;
                Stream::new_stream_encoder(&filters, real_check)
                    .map_err(|e| catch_lzma_error(e, vm))
            } else {
                Stream::new_easy_encoder(preset, real_check).map_err(|e| catch_lzma_error(e, vm))
            }
        }

        fn init_alone(
            preset: u32,
            filter_specs: Option<Vec<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<Stream> {
            if let Some(_filter_specs) = filter_specs {
                // TODO: validate single LZMA1 filter and use its options
                let options = LzmaOptions::new_preset(preset).map_err(|_| {
                    new_lzma_error(format!("Invalid compression preset: {preset}"), vm)
                })?;
                Stream::new_lzma_encoder(&options).map_err(|e| catch_lzma_error(e, vm))
            } else {
                let options = LzmaOptions::new_preset(preset).map_err(|_| {
                    new_lzma_error(format!("Invalid compression preset: {preset}"), vm)
                })?;
                Stream::new_lzma_encoder(&options).map_err(|e| catch_lzma_error(e, vm))
            }
        }

        fn init_raw(
            filter_specs: Option<Vec<PyObjectRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<Stream> {
            let filter_specs = filter_specs
                .ok_or_else(|| vm.new_value_error("Must specify filters for FORMAT_RAW"))?;
            let filters = parse_filter_chain_spec(filter_specs, vm)?;
            Stream::new_raw_encoder(&filters).map_err(|e| catch_lzma_error(e, vm))
        }
    }

    #[derive(FromArgs)]
    pub struct LZMACompressorConstructorArgs {
        #[pyarg(any, default = FORMAT_XZ)]
        format: i32,
        #[pyarg(any, default = -1)]
        check: i32,
        #[pyarg(any, optional)]
        preset: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        filters: Option<Vec<PyObjectRef>>,
    }

    impl Constructor for LZMACompressor {
        type Args = LZMACompressorConstructorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.format != FORMAT_XZ && args.check != -1 && args.check != CHECK_NONE {
                return Err(new_lzma_error(
                    "Integrity checks are only supported by FORMAT_XZ",
                    vm,
                ));
            }

            if args.preset.is_some() && args.filters.is_some() {
                return Err(new_lzma_error(
                    "Cannot specify both preset and filter chain",
                    vm,
                ));
            }

            let preset: u32 = match &args.preset {
                Some(obj) => obj.clone().try_into_value(vm)?,
                None => PRESET_DEFAULT,
            };

            let stream = match args.format {
                FORMAT_XZ => Self::init_xz(args.check, preset, args.filters, vm)?,
                FORMAT_ALONE => Self::init_alone(preset, args.filters, vm)?,
                FORMAT_RAW => Self::init_raw(args.filters, vm)?,
                _ => {
                    return Err(
                        vm.new_value_error(format!("Invalid container format: {}", args.format))
                    );
                }
            };

            Ok(Self {
                state: PyMutex::new(CompressState::new(CompressorInner::new(stream))),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl LZMACompressor {
        #[pymethod]
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut state = self.state.lock();
            state.compress(&data.borrow_buf(), vm)
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut state = self.state.lock();
            state.flush(Action::Finish, vm)
        }
    }
}
