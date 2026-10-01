// spell-checker:ignore zstd Zstd ZSTD Zstandard dfast btlazy btultra btopt btultra2 btlazy2
// spell-checker:ignore ldm nbworkers windowlog hashlog chainlog searchlog minmatch dictid
// spell-checker:ignore checksumflag contentsizeflag dictidflag overlaplog jobsize dstream clevel
// spell-checker:ignore skippable pledged fastcover windowlogmax
// cspell:ignore zstd Zstd ZSTD Zstandard dfast btlazy btultra btopt btultra2 btlazy2 ldm nbworkers
// cspell:ignore windowlog hashlog chainlog searchlog minmatch dictid checksumflag skippable pledged

pub(crate) use _zstd::module_def;

#[pymodule]
mod _zstd {
    #![allow(non_upper_case_globals)]

    use crate::compression::DecompressorArgs;
    use crate::vm::{
        AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBytesRef, PyDict, PyInt, PyModule, PyTuple, PyTupleRef, PyType,
            PyTypeRef,
        },
        common::lock::PyMutex,
        function::{ArgBytesLike, ItemDoc, OptionalOption},
        protocol::PyMappingMethods,
        types::{AsMapping, Constructor, Representable},
    };
    use alloc::boxed::Box;
    use core::fmt;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use num_traits::ToPrimitive;
    use rusty_zstd::{
        CompressOptions, Compressor, DEFAULT_CLEVEL, DEFAULT_WINDOW_MAX, DecompressOptions,
        Decompressor, Dictionary, Error, Flush, FrameKind, MAGIC, MAGIC_DICTIONARY,
        MAGIC_SKIPPABLE_MAX, MAGIC_SKIPPABLE_MIN, MAX_CLEVEL, MIN_CLEVEL, TrainOptions,
        compress_stream_out_size, compression_params, decompress_stream_out_size,
        find_frame_compressed_size, get_frame_header, train,
    };

    const MODE_CONTINUE: i32 = 0;
    const MODE_BLOCK: i32 = 1;
    const MODE_FRAME: i32 = 2;

    const LEVEL_MIN: i32 = -131_072;
    const LEVEL_MAX: i32 = 22;
    const DIGESTED: i32 = 0;
    const UNDIGESTED: i32 = 1;
    const PREFIX: i32 = 2;

    const WINDOW_LOG_MIN: i32 = 10;
    const WINDOW_LOG_MAX: i32 = if cfg!(target_pointer_width = "32") {
        30
    } else {
        31
    };
    const HASH_LOG_MIN: i32 = 6;
    const HASH_LOG_MAX: i32 = 30;
    const CHAIN_LOG_MAX: i32 = if cfg!(target_pointer_width = "32") {
        29
    } else {
        30
    };
    const SEARCH_LOG_MIN: i32 = 1;
    const SEARCH_LOG_MAX: i32 = WINDOW_LOG_MAX - 1;
    const LDM_HASH_RATE_MAX: i32 = WINDOW_LOG_MAX - HASH_LOG_MIN;
    const DEFAULT_WINDOW_LOG_MAX: i32 = 27;

    const SAMPLE_MISMATCH: &str = "The samples size tuple doesn't match the concatenation's size.";
    const DICT_SIZE_NONPOSITIVE: &str = "dict_size argument should be positive number.";
    const DIGESTED_COMPRESS_FAILED: &str =
        "Failed to create a ZSTD_CDict instance from Zstandard dictionary content.";
    const DIGESTED_DECOMPRESS_FAILED: &str =
        "Failed to create a ZSTD_DDict instance from Zstandard dictionary content.";

    #[pyattr]
    const zstd_version: &str = "1.5.7";
    #[pyattr]
    const zstd_version_number: i32 = 10_507;
    #[pyattr]
    const ZSTD_CLEVEL_DEFAULT: i32 = DEFAULT_CLEVEL;
    #[pyattr]
    const ZSTD_DStreamOutSize: i32 = 131_072;

    #[pyattr]
    const ZSTD_c_compressionLevel: i32 = 100;
    #[pyattr]
    const ZSTD_c_windowLog: i32 = 101;
    #[pyattr]
    const ZSTD_c_hashLog: i32 = 102;
    #[pyattr]
    const ZSTD_c_chainLog: i32 = 103;
    #[pyattr]
    const ZSTD_c_searchLog: i32 = 104;
    #[pyattr]
    const ZSTD_c_minMatch: i32 = 105;
    #[pyattr]
    const ZSTD_c_targetLength: i32 = 106;
    #[pyattr]
    const ZSTD_c_strategy: i32 = 107;
    #[pyattr]
    const ZSTD_c_enableLongDistanceMatching: i32 = 160;
    #[pyattr]
    const ZSTD_c_ldmHashLog: i32 = 161;
    #[pyattr]
    const ZSTD_c_ldmMinMatch: i32 = 162;
    #[pyattr]
    const ZSTD_c_ldmBucketSizeLog: i32 = 163;
    #[pyattr]
    const ZSTD_c_ldmHashRateLog: i32 = 164;
    #[pyattr]
    const ZSTD_c_contentSizeFlag: i32 = 200;
    #[pyattr]
    const ZSTD_c_checksumFlag: i32 = 201;
    #[pyattr]
    const ZSTD_c_dictIDFlag: i32 = 202;
    #[pyattr]
    const ZSTD_c_nbWorkers: i32 = 400;
    #[pyattr]
    const ZSTD_c_jobSize: i32 = 401;
    #[pyattr]
    const ZSTD_c_overlapLog: i32 = 402;
    #[pyattr]
    const ZSTD_d_windowLogMax: i32 = 100;

    #[pyattr]
    const ZSTD_fast: i32 = 1;
    #[pyattr]
    const ZSTD_dfast: i32 = 2;
    #[pyattr]
    const ZSTD_greedy: i32 = 3;
    #[pyattr]
    const ZSTD_lazy: i32 = 4;
    #[pyattr]
    const ZSTD_lazy2: i32 = 5;
    #[pyattr]
    const ZSTD_btlazy2: i32 = 6;
    #[pyattr]
    const ZSTD_btopt: i32 = 7;
    #[pyattr]
    const ZSTD_btultra: i32 = 8;
    #[pyattr]
    const ZSTD_btultra2: i32 = 9;

    #[pyattr(once, name = "ZstdError")]
    fn zstd_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type_with_doc(
            "compression.zstd",
            "ZstdError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
            const { ItemDoc::db("_zstd.ZstdError") },
        )
    }

    fn zstd_error(message: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_exception_msg(vm.class("_zstd", "ZstdError"), message.into())
    }

    fn compress_reason(err: Error) -> &'static str {
        match err {
            Error::ContentSizeMismatch | Error::UnexpectedEof => "Src size is incorrect",
            Error::BadMagic => "Unknown frame descriptor",
            Error::ChecksumMismatch => "Restored data doesn't match checksum",
            Error::WindowTooLarge | Error::ContentSizeTooLarge => {
                "Frame requires too much memory for decoding"
            }
            Error::ReservedBitSet | Error::UnusedBitSet => "Unsupported frame parameter",
            Error::DictionaryNeeded { .. } | Error::DictionaryMismatch { .. } => {
                "Dictionary mismatch"
            }
            Error::ReservedBlockType
            | Error::BlockTooLarge
            | Error::TrailingBytes
            | Error::Unimplemented
            | Error::Corruption
            | Error::InvalidLevel => "Data corruption detected",
        }
    }

    fn decompress_reason(err: Error) -> &'static str {
        match err {
            Error::BadMagic => "Unknown frame descriptor",
            Error::ChecksumMismatch => "Restored data doesn't match checksum",
            Error::WindowTooLarge | Error::ContentSizeTooLarge => {
                "Frame requires too much memory for decoding"
            }
            Error::ReservedBitSet | Error::UnusedBitSet => "Unsupported frame parameter",
            Error::DictionaryNeeded { .. } | Error::DictionaryMismatch { .. } => {
                "Dictionary mismatch"
            }
            Error::ContentSizeMismatch
            | Error::UnexpectedEof
            | Error::ReservedBlockType
            | Error::BlockTooLarge
            | Error::TrailingBytes
            | Error::Unimplemented
            | Error::Corruption
            | Error::InvalidLevel => "Data corruption detected",
        }
    }

    fn compress_error(err: Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        zstd_error(
            format!(
                "Unable to compress Zstandard data: {}",
                compress_reason(err)
            ),
            vm,
        )
    }

    fn decompress_error(err: Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        zstd_error(
            format!(
                "Unable to decompress Zstandard data: {}",
                decompress_reason(err)
            ),
            vm,
        )
    }

    fn frame_size_reason(err: Error) -> &'static str {
        match err {
            Error::BadMagic => "Unknown frame descriptor",
            Error::UnexpectedEof | Error::ContentSizeMismatch => "Src size is incorrect",
            other => decompress_reason(other),
        }
    }

    fn is_pylong(obj: &PyObject, vm: &VirtualMachine) -> bool {
        obj.downcast_ref::<PyInt>().is_some() || obj.fast_isinstance(vm.ctx.types.int_type)
    }

    fn type_name(obj: &PyObject, vm: &VirtualMachine) -> String {
        obj.class()
            .fully_qualified_name(vm)
            .unwrap_or_else(|_| "object".to_owned())
    }

    fn read_u32_at(buf: &[u8], offset: usize) -> Option<u32> {
        let end = offset.checked_add(4)?;
        let bytes = buf.get(offset..end)?;
        let chunk = <[u8; 4]>::try_from(bytes).ok()?;
        Some(u32::from_le_bytes(chunk))
    }

    fn read_dict_id(content: &[u8]) -> u32 {
        match read_u32_at(content, 0) {
            Some(magic) if magic == MAGIC_DICTIONARY => read_u32_at(content, 4).unwrap_or(0),
            _ => 0,
        }
    }

    fn is_skippable(magic: u32) -> bool {
        (MAGIC_SKIPPABLE_MIN..=MAGIC_SKIPPABLE_MAX).contains(&magic)
    }

    const BLOCKSIZE_MAX_BYTES: usize = 128 * 1024;

    fn codec_level(level: i32) -> i32 {
        level.clamp(MIN_CLEVEL, MAX_CLEVEL)
    }

    fn block_capacity(level: i32) -> usize {
        let log =
            compression_params(codec_level(level), None).map_or(17, |params| params.window_log);
        let window = 1usize.checked_shl(log.min(31)).unwrap_or(usize::MAX);
        window.min(BLOCKSIZE_MAX_BYTES)
    }

    fn window_max_bytes(log: i32) -> u64 {
        let Ok(log) = u32::try_from(log) else {
            return DEFAULT_WINDOW_MAX;
        };
        1u64.checked_shl(log).unwrap_or(u64::MAX)
    }

    fn flush_kind(mode: i32) -> Flush {
        match mode {
            MODE_BLOCK => Flush::Flush,
            MODE_FRAME => Flush::End,
            _ => Flush::Continue,
        }
    }

    fn check_compress_mode(mode: i32, vm: &VirtualMachine) -> PyResult<()> {
        if matches!(mode, MODE_CONTINUE | MODE_BLOCK | MODE_FRAME) {
            Ok(())
        } else {
            Err(vm.new_value_error(
                "mode argument wrong value, it should be one of ZstdCompressor.CONTINUE, \
                 ZstdCompressor.FLUSH_BLOCK, ZstdCompressor.FLUSH_FRAME.",
            ))
        }
    }

    fn check_flush_mode(mode: i32, vm: &VirtualMachine) -> PyResult<()> {
        if matches!(mode, MODE_BLOCK | MODE_FRAME) {
            Ok(())
        } else {
            Err(vm.new_value_error(
                "mode argument wrong value, it should be ZstdCompressor.FLUSH_FRAME or \
                 ZstdCompressor.FLUSH_BLOCK.",
            ))
        }
    }

    fn compress_bounds(id: i32) -> Option<(&'static str, i32, i32)> {
        match id {
            ZSTD_c_compressionLevel => Some(("compression_level", LEVEL_MIN, LEVEL_MAX)),
            ZSTD_c_windowLog => Some(("window_log", WINDOW_LOG_MIN, WINDOW_LOG_MAX)),
            ZSTD_c_hashLog => Some(("hash_log", HASH_LOG_MIN, HASH_LOG_MAX)),
            ZSTD_c_chainLog => Some(("chain_log", HASH_LOG_MIN, CHAIN_LOG_MAX)),
            ZSTD_c_searchLog => Some(("search_log", SEARCH_LOG_MIN, SEARCH_LOG_MAX)),
            ZSTD_c_minMatch => Some(("min_match", 3, 7)),
            ZSTD_c_targetLength => Some(("target_length", 0, 131_072)),
            ZSTD_c_strategy => Some(("strategy", ZSTD_fast, ZSTD_btultra2)),
            ZSTD_c_enableLongDistanceMatching => Some(("enable_long_distance_matching", 0, 2)),
            ZSTD_c_ldmHashLog => Some(("ldm_hash_log", HASH_LOG_MIN, HASH_LOG_MAX)),
            ZSTD_c_ldmMinMatch => Some(("ldm_min_match", 4, 4096)),
            ZSTD_c_ldmBucketSizeLog => Some(("ldm_bucket_size_log", 1, 8)),
            ZSTD_c_ldmHashRateLog => Some(("ldm_hash_rate_log", 0, LDM_HASH_RATE_MAX)),
            ZSTD_c_contentSizeFlag => Some(("content_size_flag", 0, 1)),
            ZSTD_c_checksumFlag => Some(("checksum_flag", 0, 1)),
            ZSTD_c_dictIDFlag => Some(("dict_id_flag", 0, 1)),
            ZSTD_c_nbWorkers => Some(("nb_workers", 0, 256)),
            ZSTD_c_jobSize => Some(("job_size", 0, 1 << 30)),
            ZSTD_c_overlapLog => Some(("overlap_log", 0, 9)),
            _ => None,
        }
    }

    fn decompress_bounds(id: i32) -> Option<(&'static str, i32, i32)> {
        match id {
            ZSTD_d_windowLogMax => Some(("window_log_max", WINDOW_LOG_MIN, WINDOW_LOG_MAX)),
            _ => None,
        }
    }

    struct CompressSettings {
        level: i32,
        checksum: bool,
        content_size: bool,
        dict_id: bool,
    }

    impl Default for CompressSettings {
        fn default() -> Self {
            Self {
                level: DEFAULT_CLEVEL,
                checksum: false,
                content_size: true,
                dict_id: true,
            }
        }
    }

    struct DecompressSettings {
        window_log_max: i32,
    }

    impl Default for DecompressSettings {
        fn default() -> Self {
            Self {
                window_log_max: DEFAULT_WINDOW_LOG_MAX,
            }
        }
    }

    fn apply_ranged(
        kind: &str,
        id: i32,
        value: i32,
        bounds: Option<(&'static str, i32, i32)>,
        vm: &VirtualMachine,
    ) -> PyResult<i32> {
        let Some((name, lo, hi)) = bounds else {
            return Err(vm.new_value_error(format!(
                "invalid {kind} parameter 'unknown parameter (key {id})'"
            )));
        };
        if value < lo || value > hi {
            return Err(vm.new_value_error(format!(
                "{kind} parameter '{name}' received an illegal value {value}; \
                 the valid range is [{lo}, {hi}]"
            )));
        }
        Ok(id)
    }

    fn parse_level(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i32> {
        if !is_pylong(obj, vm) {
            return Err(vm.new_type_error("invalid type for level, expected int"));
        }
        match obj.try_to_value::<i32>(vm) {
            Ok(level) if (LEVEL_MIN..=LEVEL_MAX).contains(&level) => Ok(level),
            Ok(level) => Err(vm.new_value_error(format!(
                "illegal compression level {level}; the valid range is [{LEVEL_MIN}, {LEVEL_MAX}]"
            ))),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.overflow_error) => Err(vm
                .new_value_error(format!(
                    "illegal compression level; the valid range is [{LEVEL_MIN}, {LEVEL_MAX}]"
                ))),
            Err(err) => Err(err),
        }
    }

    fn module_state(vm: &VirtualMachine) -> Option<PyRef<ZstdState>> {
        let modules = vm.sys_module.get_attr("modules", vm).ok()?;
        let module = modules.get_item("_zstd", vm).ok()?;
        let state = module.get_attr("_state", vm).ok()?;
        state.downcast::<ZstdState>().ok()
    }

    fn registered_parameter_types(vm: &VirtualMachine) -> (Option<PyTypeRef>, Option<PyTypeRef>) {
        let Some(state) = module_state(vm) else {
            return (None, None);
        };
        let compression = {
            let guard = state.compression.lock();
            guard.as_ref().cloned()
        };
        let decompression = {
            let guard = state.decompression.lock();
            guard.as_ref().cloned()
        };
        (compression, decompression)
    }

    fn reject_foreign_key(key: &PyObject, compress: bool, vm: &VirtualMachine) -> PyResult<()> {
        let (compression, decompression) = registered_parameter_types(vm);
        if compress {
            if let Some(ty) = decompression.as_ref()
                && key.class().is(ty)
            {
                return Err(vm.new_type_error(
                    "compression options dictionary key must not be a DecompressionParameter \
                     attribute",
                ));
            }
        } else if let Some(ty) = compression.as_ref()
            && key.class().is(ty)
        {
            return Err(vm.new_type_error(
                "decompression options dictionary key must not be a CompressionParameter attribute",
            ));
        }
        Ok(())
    }

    fn options_dict<'a>(
        obj: &'a PyObject,
        compress: bool,
        vm: &VirtualMachine,
    ) -> PyResult<&'a Py<PyDict>> {
        obj.downcast_ref::<PyDict>().ok_or_else(|| {
            let which = if compress {
                "ZstdCompressor"
            } else {
                "ZstdDecompressor"
            };
            vm.new_type_error(format!(
                "{which}() argument 'options' must be dict, not {}",
                type_name(obj, vm)
            ))
        })
    }

    fn for_each_option(
        dict: &Py<PyDict>,
        compress: bool,
        mut apply: impl FnMut(i32, i32) -> PyResult<()>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mut pos = 0;
        while let Some((next, key, value)) = dict.next_entry(pos) {
            pos = next;
            reject_foreign_key(&key, compress, vm)?;
            let key_id = key.try_to_value::<i32>(vm)?;
            let value_id = value.try_to_value::<i32>(vm)?;
            apply(key_id, value_id)?;
        }
        Ok(())
    }

    fn parse_compress_options(obj: &PyObject, vm: &VirtualMachine) -> PyResult<CompressSettings> {
        let dict = options_dict(obj, true, vm)?;
        let mut settings = CompressSettings::default();
        for_each_option(
            dict,
            true,
            |id, value| {
                if id == ZSTD_c_compressionLevel {
                    if !(LEVEL_MIN..=LEVEL_MAX).contains(&value) {
                        return Err(vm.new_value_error(format!(
                            "illegal compression level {value}; the valid range is [{LEVEL_MIN}, {LEVEL_MAX}]"
                        )));
                    }
                    settings.level = value;
                    return Ok(());
                }
                let id = apply_ranged("compression", id, value, compress_bounds(id), vm)?;
                match id {
                    ZSTD_c_checksumFlag => settings.checksum = value != 0,
                    ZSTD_c_contentSizeFlag => settings.content_size = value != 0,
                    ZSTD_c_dictIDFlag => settings.dict_id = value != 0,
                    _ => {}
                }
                Ok(())
            },
            vm,
        )?;
        Ok(settings)
    }

    fn parse_decompress_options(
        obj: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<DecompressSettings> {
        let dict = options_dict(obj, false, vm)?;
        let mut settings = DecompressSettings::default();
        for_each_option(
            dict,
            false,
            |id, value| {
                let id = apply_ranged("decompression", id, value, decompress_bounds(id), vm)?;
                if id == ZSTD_d_windowLogMax {
                    settings.window_log_max = value;
                }
                Ok(())
            },
            vm,
        )?;
        Ok(settings)
    }

    #[pyclass(no_attr, module = "_zstd", name = "_zstd_state", traverse)]
    #[derive(Debug, PyPayload)]
    struct ZstdState {
        compression: PyMutex<Option<PyTypeRef>>,
        decompression: PyMutex<Option<PyTypeRef>>,
    }

    #[pyclass]
    impl ZstdState {}

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        let state = ZstdState {
            compression: PyMutex::new(None),
            decompression: PyMutex::new(None),
        }
        .into_ref(&vm.ctx);
        module.set_attr("_state", state, vm)?;
        Ok(())
    }

    #[pyfunction]
    fn set_parameter_types(
        c_parameter_type: PyTypeRef,
        d_parameter_type: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(state) = module_state(vm) else {
            return Err(vm.new_runtime_error("_zstd state is missing"));
        };
        {
            let mut slot = state.compression.lock();
            *slot = Some(c_parameter_type);
        }
        {
            let mut slot = state.decompression.lock();
            *slot = Some(d_parameter_type);
        }
        Ok(())
    }

    #[derive(FromArgs)]
    struct ParamBoundsArgs {
        #[pyarg(any)]
        parameter: i32,
        #[pyarg(any)]
        is_compress: bool,
    }

    #[pyfunction]
    fn get_param_bounds(args: ParamBoundsArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let ParamBoundsArgs {
            parameter,
            is_compress,
        } = args;
        let bounds = if is_compress {
            compress_bounds(parameter)
        } else {
            decompress_bounds(parameter)
        };
        let Some((_, lo, hi)) = bounds else {
            let kind = if is_compress {
                "compression"
            } else {
                "decompression"
            };
            return Err(zstd_error(
                format!("Unable to get zstd {kind} parameter bounds: Unsupported parameter"),
                vm,
            ));
        };
        Ok(vm
            .ctx
            .new_tuple(vec![vm.ctx.new_int(lo).into(), vm.ctx.new_int(hi).into()]))
    }

    #[derive(FromArgs)]
    struct ZstdDictArgs {
        #[pyarg(positional)]
        dict_content: ArgBytesLike,
        #[pyarg(named, default = false)]
        is_raw: bool,
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdDict", traverse)]
    #[derive(PyPayload)]
    struct ZstdDict {
        dict_content: PyBytesRef,
        #[pytraverse(skip)]
        dict_id: u32,
    }

    impl fmt::Debug for ZstdDict {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "<ZstdDict dict_id={} dict_size={}>",
                self.dict_id,
                self.dict_content.as_bytes().len()
            )
        }
    }

    impl Constructor for ZstdDict {
        type Args = ZstdDictArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let content = args.dict_content.with_ref(<[u8]>::to_vec);
            if content.len() < 8 {
                return Err(vm.new_value_error(
                    "Zstandard dictionary content too short (must have at least eight bytes)",
                ));
            }
            let dict_id = read_dict_id(&content);
            if !args.is_raw && dict_id == 0 {
                return Err(vm.new_value_error("invalid Zstandard dictionary"));
            }
            Ok(Self {
                dict_content: vm.ctx.new_bytes(content),
                dict_id,
            })
        }
    }

    impl Representable for ZstdDict {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<ZstdDict dict_id={} dict_size={}>",
                zelf.dict_id,
                zelf.dict_content.as_bytes().len()
            ))
        }
    }

    impl AsMapping for ZstdDict {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                length: Some(|mapping, _vm| {
                    Ok(ZstdDict::mapping_downcast(mapping)
                        .dict_content
                        .as_bytes()
                        .len())
                }),
                subscript: None,
                ass_subscript: None,
            };
            &AS_MAPPING
        }
    }

    #[pyclass(with(Constructor, Representable, AsMapping))]
    impl ZstdDict {
        #[pygetset]
        fn dict_content(zelf: &Py<Self>) -> PyBytesRef {
            zelf.dict_content.clone()
        }

        #[pygetset]
        fn dict_id(zelf: &Py<Self>) -> u32 {
            zelf.dict_id
        }

        fn marker_tuple(zelf: PyRef<Self>, marker: i32, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(marker).into()])
        }

        #[pygetset]
        fn as_digested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::marker_tuple(zelf, DIGESTED, vm)
        }

        #[pygetset]
        fn as_undigested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::marker_tuple(zelf, UNDIGESTED, vm)
        }

        #[pygetset]
        fn as_prefix(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::marker_tuple(zelf, PREFIX, vm)
        }
    }

    fn zstd_dict_type_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_type_error("zstd_dict argument should be a ZstdDict object.")
    }

    enum DictArg {
        Bare(PyRef<ZstdDict>),
        Marked(PyRef<ZstdDict>, i32),
    }

    fn parse_dict_arg(obj: &PyObject, vm: &VirtualMachine) -> PyResult<DictArg> {
        if obj.downcast_ref::<ZstdDict>().is_some() {
            let dict = obj
                .to_owned()
                .downcast::<ZstdDict>()
                .map_err(|_| zstd_dict_type_error(vm))?;
            return Ok(DictArg::Bare(dict));
        }
        if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm) {
            let items = tuple.as_slice();
            if items.len() == 2 && items[0].downcast_ref::<ZstdDict>().is_some() {
                let dict = items[0]
                    .to_owned()
                    .downcast::<ZstdDict>()
                    .map_err(|_| zstd_dict_type_error(vm))?;
                let marker_obj = &items[1];
                if is_pylong(marker_obj, vm) {
                    let marker = match marker_obj.try_to_value::<i32>(vm) {
                        Ok(marker) => marker,
                        Err(err) if err.fast_isinstance(vm.ctx.exceptions.overflow_error) => {
                            return Err(err);
                        }
                        Err(_) => return Err(zstd_dict_type_error(vm)),
                    };
                    if matches!(marker, DIGESTED | UNDIGESTED | PREFIX) {
                        return Ok(DictArg::Marked(dict, marker));
                    }
                }
            }
        }
        Err(zstd_dict_type_error(vm))
    }

    enum LoadedDict {
        Dict(Box<Dictionary>),
        Prefix(Vec<u8>),
    }

    fn load_dict(spec: DictArg, compress: bool, vm: &VirtualMachine) -> PyResult<LoadedDict> {
        let (dict, marker) = match spec {
            DictArg::Bare(dict) => {
                let marker = if compress { UNDIGESTED } else { DIGESTED };
                (dict, marker)
            }
            DictArg::Marked(dict, marker) => (dict, marker),
        };
        let content = dict.dict_content.as_bytes();
        let loaded = match marker {
            PREFIX => LoadedDict::Prefix(content.to_vec()),
            DIGESTED if dict.dict_id == 0 => {
                LoadedDict::Dict(Box::new(Dictionary::raw(content.to_vec())))
            }
            DIGESTED => match Dictionary::from_bytes(content) {
                Ok(parsed) => LoadedDict::Dict(Box::new(parsed)),
                Err(_) => {
                    let message = if compress {
                        DIGESTED_COMPRESS_FAILED
                    } else {
                        DIGESTED_DECOMPRESS_FAILED
                    };
                    return Err(zstd_error(message.to_owned(), vm));
                }
            },
            _ => {
                let parsed = match Dictionary::from_bytes(content) {
                    Ok(parsed) => parsed,
                    Err(_) => Dictionary::raw(content.to_vec()),
                };
                LoadedDict::Dict(Box::new(parsed))
            }
        };
        Ok(loaded)
    }

    enum AppliedDict {
        None,
        Dict {
            dict: Box<Dictionary>,
            write_id: bool,
        },
        Prefix(Vec<u8>),
    }

    struct Session {
        comp: Box<Compressor>,
        produced: u64,
    }

    struct CompState {
        settings: CompressSettings,
        applied: AppliedDict,
        pledge: Option<u64>,
        session: Option<Session>,
        pending: Vec<u8>,
        block_max: usize,
        last_mode_value: i32,
    }

    impl CompState {
        fn fail_session(&mut self) {
            self.session = None;
            self.pending.clear();
            self.last_mode_value = MODE_FRAME;
        }

        fn finish_frame(&mut self) {
            self.session = None;
            self.pending.clear();
            self.pledge = None;
            if matches!(self.applied, AppliedDict::Prefix(_)) {
                self.applied = AppliedDict::None;
            }
        }

        fn frame_size_header(&self, len: usize) -> Option<u64> {
            if self.settings.content_size {
                Some(u64::try_from(len).unwrap_or(u64::MAX))
            } else {
                None
            }
        }

        fn stream_header(&self) -> Option<u64> {
            if self.settings.content_size {
                self.pledge
            } else {
                None
            }
        }

        fn apply_dict(&self, comp: &mut Compressor) -> Result<(), Error> {
            match &self.applied {
                AppliedDict::Dict { dict, write_id } => {
                    comp.set_dictionary(dict)?;
                    comp.set_write_dict_id(*write_id && dict.id() != 0);
                }
                AppliedDict::Prefix(prefix) => {
                    comp.set_prefix(prefix)?;
                }
                AppliedDict::None => {}
            }
            Ok(())
        }

        fn open_session(&mut self, header: Option<u64>) -> Result<(), Error> {
            let mut comp = Compressor::with_options(
                CompressOptions {
                    level: codec_level(self.settings.level),
                    checksum: self.settings.checksum,
                },
                None,
            )?;
            if let Some(size) = header {
                comp.set_pledged_src_size(size);
            }
            self.apply_dict(&mut comp)?;
            self.session = Some(Session {
                comp: Box::new(comp),
                produced: 0,
            });
            Ok(())
        }

        // Empty input with nothing buffered must not start a frame, except for
        // an explicit end, which is a one-shot empty frame. A one-shot end
        // writes the input length into the header and ignores a stored pledge.
        // Short CONTINUE input stays buffered: the first stream call emits a
        // header, and a short write has to leave the underlying file untouched.
        fn compress(&mut self, input: &[u8], mode: i32) -> Result<Vec<u8>, Error> {
            if input.is_empty()
                && self.session.is_none()
                && self.pending.is_empty()
                && mode != MODE_FRAME
            {
                self.last_mode_value = mode;
                return Ok(Vec::new());
            }
            let oneshot = self.session.is_none() && self.pending.is_empty() && mode == MODE_FRAME;
            if !input.is_empty() {
                self.pending.extend_from_slice(input);
            }
            if self.session.is_none()
                && mode == MODE_CONTINUE
                && self.pending.len() < self.block_max
            {
                self.last_mode_value = mode;
                return Ok(Vec::new());
            }
            if self.session.is_none() {
                let header_len = self.pending.len();
                let header = if oneshot {
                    self.frame_size_header(header_len)
                } else {
                    self.stream_header()
                };
                if let Err(err) = self.open_session(header) {
                    self.fail_session();
                    return Err(err);
                }
            }
            let flush = flush_kind(mode);
            let pending = core::mem::take(&mut self.pending);
            let pumped = match self.session.as_mut() {
                Some(session) => pump(session, &pending, flush),
                None => Err(Error::Corruption),
            };
            let bytes = match pumped {
                Ok(bytes) => bytes,
                Err(err) => {
                    self.fail_session();
                    return Err(err);
                }
            };
            if mode == MODE_FRAME {
                let Some(produced) = self.session.as_ref().map(|session| session.produced) else {
                    self.fail_session();
                    return Err(Error::Corruption);
                };
                if !oneshot && self.pledge.is_some_and(|pledged| pledged != produced) {
                    self.fail_session();
                    return Err(Error::ContentSizeMismatch);
                }
                self.finish_frame();
            }
            self.last_mode_value = mode;
            Ok(bytes)
        }
    }

    fn pump(session: &mut Session, input: &[u8], flush: Flush) -> Result<Vec<u8>, Error> {
        let added = u64::try_from(input.len()).unwrap_or(u64::MAX);
        session.produced = session.produced.saturating_add(added);
        let mut out = Vec::new();
        let mut buf = vec![0u8; compress_stream_out_size()];
        let mut pending = input;
        loop {
            let status = session.comp.stream(pending, &mut buf, flush)?;
            pending = &[];
            out.extend_from_slice(&buf[..status.output_produced]);
            if status.done || status.output_produced < buf.len() {
                break;
            }
        }
        Ok(out)
    }

    #[derive(FromArgs)]
    struct ZstdCompressorArgs {
        #[pyarg(any, optional, py_default = "None")]
        level: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional, py_default = "None")]
        options: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional, py_default = "None")]
        zstd_dict: OptionalOption<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdCompressor", traverse)]
    #[derive(PyPayload)]
    struct ZstdCompressor {
        dict_ref: Option<PyRef<ZstdDict>>,
        #[pytraverse(skip)]
        last_mode: AtomicI32,
        #[pytraverse(skip)]
        state: PyMutex<CompState>,
    }

    impl fmt::Debug for ZstdCompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("compression.zstd.ZstdCompressor")
        }
    }

    impl Constructor for ZstdCompressor {
        type Args = ZstdCompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let level = args.level.flatten();
            let options = args.options.flatten();
            let zstd_dict = args.zstd_dict.flatten();
            if level.is_some() && options.is_some() {
                return Err(vm.new_type_error("Only one of level or options should be used."));
            }
            let mut settings = CompressSettings::default();
            if let Some(level) = level.as_ref() {
                settings.level = parse_level(level, vm)?;
            }
            if let Some(options) = options.as_ref() {
                settings = parse_compress_options(options, vm)?;
            }
            let (dict_ref, applied) = match zstd_dict.as_ref() {
                Some(obj) => {
                    let spec = parse_dict_arg(obj, vm)?;
                    let dict_ref = match &spec {
                        DictArg::Bare(dict) | DictArg::Marked(dict, _) => dict.clone(),
                    };
                    let loaded = load_dict(spec, true, vm)?;
                    let applied = match loaded {
                        LoadedDict::Dict(dict) => AppliedDict::Dict {
                            dict,
                            write_id: settings.dict_id,
                        },
                        LoadedDict::Prefix(bytes) => AppliedDict::Prefix(bytes),
                    };
                    (Some(dict_ref), applied)
                }
                None => (None, AppliedDict::None),
            };
            Ok(Self {
                dict_ref,
                last_mode: AtomicI32::new(MODE_FRAME),
                state: PyMutex::new(CompState {
                    block_max: block_capacity(settings.level),
                    settings,
                    applied,
                    pledge: None,
                    session: None,
                    pending: Vec::new(),
                    last_mode_value: MODE_FRAME,
                }),
            })
        }
    }

    #[derive(FromArgs)]
    struct CompressCallArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(any, default = 0)]
        mode: i32,
    }

    #[derive(FromArgs)]
    struct FlushCallArgs {
        #[pyarg(any, default = 2)]
        mode: i32,
    }

    #[pyclass(with(Constructor))]
    impl ZstdCompressor {
        fn locked_compress(zelf: &Py<Self>, input: &[u8], mode: i32) -> Result<Vec<u8>, Error> {
            let mut state = zelf.state.lock();
            let result = state.compress(input, mode);
            zelf.last_mode
                .store(state.last_mode_value, Ordering::Relaxed);
            result
        }

        #[pymethod]
        fn compress(
            zelf: &Py<Self>,
            args: CompressCallArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let CompressCallArgs { data, mode } = args;
            check_compress_mode(mode, vm)?;
            let input = data.with_ref(<[u8]>::to_vec);
            vm.allow_threads(|| Self::locked_compress(zelf, &input, mode))
                .map_err(|err| compress_error(err, vm))
        }

        #[pymethod]
        fn flush(zelf: &Py<Self>, args: FlushCallArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let FlushCallArgs { mode } = args;
            check_flush_mode(mode, vm)?;
            vm.allow_threads(|| Self::locked_compress(zelf, &[], mode))
                .map_err(|err| compress_error(err, vm))
        }

        #[pymethod]
        fn set_pledged_input_size(
            zelf: &Py<Self>,
            size: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let pledge = parse_pledge(&size, vm)?;
            let bad_mode = vm.allow_threads(|| {
                let mut state = zelf.state.lock();
                if zelf.last_mode.load(Ordering::Relaxed) != MODE_FRAME {
                    true
                } else {
                    state.pledge = pledge;
                    false
                }
            });
            if bad_mode {
                Err(vm.new_value_error(
                    "set_pledged_input_size() method must be called when last_mode == FLUSH_FRAME",
                ))
            } else {
                Ok(())
            }
        }

        #[pygetset]
        fn last_mode(zelf: &Py<Self>) -> i32 {
            zelf.last_mode.load(Ordering::Relaxed)
        }

        #[extend_class]
        fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
            class.set_attr(
                ctx.intern_str("CONTINUE"),
                ctx.new_int(MODE_CONTINUE).into(),
            );
            class.set_attr(
                ctx.intern_str("FLUSH_BLOCK"),
                ctx.new_int(MODE_BLOCK).into(),
            );
            class.set_attr(
                ctx.intern_str("FLUSH_FRAME"),
                ctx.new_int(MODE_FRAME).into(),
            );
        }
    }

    fn parse_pledge(size: &PyObject, vm: &VirtualMachine) -> PyResult<Option<u64>> {
        if vm.is_none(size) {
            return Ok(None);
        }
        if !is_pylong(size, vm) {
            return Err(pledge_error(vm));
        }
        let int_obj = size.try_index(vm)?;
        match int_obj.as_bigint().to_u64() {
            Some(value) if value < u64::MAX - 1 => Ok(Some(value)),
            _ => Err(pledge_error(vm)),
        }
    }

    fn pledge_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        let limit = u64::MAX - 1;
        vm.new_value_error(format!(
            "size argument should be a positive int less than {limit}"
        ))
    }

    enum DecApplied {
        None,
        Dict(Box<Dictionary>),
        Prefix(Vec<u8>),
    }

    struct DecState {
        window_max: u64,
        applied: DecApplied,
        src: Vec<u8>,
        fed: usize,
        dec: Option<Box<Decompressor>>,
        frame_len: Option<usize>,
        out: Vec<u8>,
        out_pos: usize,
        finished: bool,
        probed: usize,
        needs_input: bool,
    }

    impl DecState {
        fn reset_session(&mut self) {
            self.src.clear();
            self.fed = 0;
            self.dec = None;
            self.frame_len = None;
            self.out.clear();
            self.out_pos = 0;
            self.finished = false;
            self.probed = 0;
            self.needs_input = true;
        }

        fn is_eof(&self) -> bool {
            self.finished && self.out.is_empty()
        }

        fn unused(&self) -> &[u8] {
            // Bytes after this frame stay hidden until its output is drained.
            if !self.is_eof() {
                return &[];
            }
            let start = self.frame_len.unwrap_or(self.src.len()).min(self.src.len());
            &self.src[start..]
        }

        fn clear_prefix(&mut self) {
            if matches!(self.applied, DecApplied::Prefix(_)) {
                self.applied = DecApplied::None;
            }
        }

        fn make_decoder(&self) -> Box<Decompressor> {
            let mut dec = Decompressor::with_options(DecompressOptions {
                window_max: self.window_max,
                force_ignore_checksum: false,
            });
            match &self.applied {
                DecApplied::Dict(dict) => dec.set_dictionary(dict.as_ref().clone()),
                DecApplied::Prefix(prefix) => dec.set_prefix(prefix),
                DecApplied::None => {}
            }
            Box::new(dec)
        }

        fn classify(&mut self) -> Result<(), Error> {
            if self.dec.is_some() || self.finished || self.src.len() < 4 {
                return Ok(());
            }
            let Some(magic) = read_u32_at(&self.src, 0) else {
                return Err(Error::Corruption);
            };
            if is_skippable(magic) {
                if self.src.len() < 8 {
                    return Ok(());
                }
                let Some(body_len) = read_u32_at(&self.src, 4) else {
                    return Err(Error::Corruption);
                };
                let Ok(body) = usize::try_from(body_len) else {
                    return Err(Error::Corruption);
                };
                let Some(total) = 8usize.checked_add(body) else {
                    return Err(Error::Corruption);
                };
                if self.src.len() < total {
                    return Ok(());
                }
                self.frame_len = Some(total);
                self.finished = true;
                self.clear_prefix();
                return Ok(());
            }
            if magic != MAGIC {
                return Err(Error::BadMagic);
            }
            let dec = self.make_decoder();
            self.dec = Some(dec);
            Ok(())
        }

        fn probe(&mut self) -> Result<(), Error> {
            if self.frame_len.is_some() || self.probed == self.src.len() {
                return Ok(());
            }
            match find_frame_compressed_size(&self.src) {
                Ok(0) => Err(Error::Corruption),
                Ok(len) => {
                    self.frame_len = Some(len);
                    self.probed = self.src.len();
                    Ok(())
                }
                Err(Error::UnexpectedEof) => {
                    self.probed = self.src.len();
                    Ok(())
                }
                Err(err) => Err(err),
            }
        }

        fn drain(&mut self, chunk: Vec<u8>, end: bool) -> Result<(), Error> {
            let mut dec = match self.dec.take() {
                Some(dec) => dec,
                None => return Err(Error::Corruption),
            };
            let mut buf = vec![0u8; decompress_stream_out_size()];
            let mut input: &[u8] = &chunk;
            let result = loop {
                match dec.stream(input, &mut buf, end) {
                    Ok(status) => {
                        input = &[];
                        self.out.extend_from_slice(&buf[..status.output_produced]);
                        if status.done {
                            self.finished = true;
                            break Ok(());
                        }
                        if status.output_produced < buf.len() {
                            break Ok(());
                        }
                    }
                    Err(err) => break Err(err),
                }
            };
            self.dec = Some(dec);
            if self.finished {
                self.clear_prefix();
            }
            result
        }

        fn feed(&mut self) -> Result<(), Error> {
            if self.dec.is_none() || self.finished {
                return Ok(());
            }
            let (input_end, end_flag) = match self.frame_len {
                Some(len) => (len.min(self.src.len()), self.src.len() >= len),
                None => (self.src.len(), false),
            };
            if self.fed < input_end {
                let chunk = self.src[self.fed..input_end].to_vec();
                self.fed = input_end;
                self.drain(chunk, end_flag)
            } else if end_flag {
                self.drain(Vec::new(), true)
            } else {
                Ok(())
            }
        }

        fn drive(&mut self) -> Result<(), Error> {
            if self.finished {
                return Ok(());
            }
            self.classify()?;
            if self.finished || self.dec.is_none() {
                return Ok(());
            }
            self.probe()?;
            self.feed()
        }

        // needs_input stays true unless this call filled max_length, or the
        // frame is already finished. Returning b'' with needs_input false and
        // eof false makes the reader spin.
        fn take(&mut self, max_length: Option<usize>) -> Vec<u8> {
            let available = self.out.len().saturating_sub(self.out_pos);
            let take = match max_length {
                Some(max) => available.min(max),
                None => available,
            };
            let start = self.out_pos;
            let end = start + take;
            let bytes = self.out[start..end].to_vec();
            self.out_pos = end;
            if self.out_pos == self.out.len() {
                self.out.clear();
                self.out_pos = 0;
            }
            self.needs_input = if self.finished {
                false
            } else {
                match max_length {
                    Some(max) => take != max,
                    None => true,
                }
            };
            bytes
        }

        fn pull(&mut self, input: &[u8], max_length: Option<usize>) -> Result<Vec<u8>, Error> {
            self.src.extend_from_slice(input);
            if !self.finished {
                self.drive()?;
            }
            Ok(self.take(max_length))
        }
    }

    enum DecFail {
        Eof,
        Codec(Error),
    }

    #[derive(FromArgs)]
    struct ZstdDecompressorArgs {
        #[pyarg(any, optional, py_default = "None")]
        zstd_dict: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional, py_default = "None")]
        options: OptionalOption<PyObjectRef>,
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdDecompressor", traverse)]
    #[derive(PyPayload)]
    struct ZstdDecompressor {
        dict_ref: Option<PyRef<ZstdDict>>,
        #[pymember]
        #[pytraverse(skip)]
        eof: AtomicBool,
        #[pymember]
        #[pytraverse(skip)]
        needs_input: AtomicBool,
        #[pytraverse(skip)]
        state: PyMutex<DecState>,
    }

    impl fmt::Debug for ZstdDecompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("compression.zstd.ZstdDecompressor")
        }
    }

    impl Constructor for ZstdDecompressor {
        type Args = ZstdDecompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let zstd_dict = args.zstd_dict.flatten();
            let options = args.options.flatten();
            let (dict_ref, applied) = match zstd_dict.as_ref() {
                Some(obj) => {
                    let spec = parse_dict_arg(obj, vm)?;
                    let dict_ref = match &spec {
                        DictArg::Bare(dict) | DictArg::Marked(dict, _) => dict.clone(),
                    };
                    let loaded = load_dict(spec, false, vm)?;
                    let applied = match loaded {
                        LoadedDict::Dict(dict) => DecApplied::Dict(dict),
                        LoadedDict::Prefix(bytes) => DecApplied::Prefix(bytes),
                    };
                    (Some(dict_ref), applied)
                }
                None => (None, DecApplied::None),
            };
            let window_log_max = match options.as_ref() {
                Some(options) => parse_decompress_options(options, vm)?.window_log_max,
                None => DEFAULT_WINDOW_LOG_MAX,
            };
            Ok(Self {
                dict_ref,
                eof: AtomicBool::new(false),
                needs_input: AtomicBool::new(true),
                state: PyMutex::new(DecState {
                    window_max: window_max_bytes(window_log_max),
                    applied,
                    src: Vec::new(),
                    fed: 0,
                    dec: None,
                    frame_len: None,
                    out: Vec::new(),
                    out_pos: 0,
                    finished: false,
                    probed: 0,
                    needs_input: true,
                }),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl ZstdDecompressor {
        #[pymethod]
        fn decompress(
            zelf: &Py<Self>,
            args: DecompressorArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            if zelf.eof.load(Ordering::Relaxed) {
                return Err(vm.new_eof_error("Already at the end of a Zstandard frame."));
            }
            let max_length = args.max_length();
            let data = args.data().to_vec();
            let outcome = vm.allow_threads(|| {
                let mut state = zelf.state.lock();
                if state.is_eof() {
                    return Err(DecFail::Eof);
                }
                match state.pull(&data, max_length) {
                    Ok(bytes) => {
                        zelf.eof.store(state.is_eof(), Ordering::Relaxed);
                        zelf.needs_input.store(state.needs_input, Ordering::Relaxed);
                        Ok(bytes)
                    }
                    Err(err) => {
                        state.reset_session();
                        zelf.eof.store(false, Ordering::Relaxed);
                        zelf.needs_input.store(true, Ordering::Relaxed);
                        Err(DecFail::Codec(err))
                    }
                }
            });
            match outcome {
                Ok(bytes) => Ok(bytes),
                Err(DecFail::Eof) => {
                    Err(vm.new_eof_error("Already at the end of a Zstandard frame."))
                }
                Err(DecFail::Codec(err)) => Err(decompress_error(err, vm)),
            }
        }

        #[pygetset]
        fn unused_data(zelf: &Py<Self>, vm: &VirtualMachine) -> PyBytesRef {
            let unused = {
                let state = zelf.state.lock();
                state.unused().to_vec()
            };
            if unused.is_empty() {
                vm.ctx.empty_bytes.clone()
            } else {
                vm.ctx.new_bytes(unused)
            }
        }
    }

    enum SampleSize {
        Value(usize),
        Mismatch,
    }

    fn sample_size(obj: &PyObject, vm: &VirtualMachine) -> PyResult<SampleSize> {
        let int_obj = match obj.try_index(vm) {
            Ok(int_obj) => int_obj,
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.overflow_error) => {
                return Ok(SampleSize::Mismatch);
            }
            Err(err) => return Err(err),
        };
        match int_obj.try_to_primitive_in_range::<usize>(vm) {
            Ok(value) => Ok(SampleSize::Value(value)),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.overflow_error) => {
                Ok(SampleSize::Mismatch)
            }
            Err(err) => Err(err),
        }
    }

    fn split_samples<'a>(
        bytes: &'a [u8],
        sizes: &PyTupleRef,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<&'a [u8]>> {
        if u32::try_from(sizes.as_slice().len()).is_err() {
            return Err(
                vm.new_value_error(format!("The number of samples should be <= {}.", u32::MAX))
            );
        }
        let mut samples = Vec::with_capacity(sizes.as_slice().len());
        let mut offset = 0usize;
        for item in sizes.as_slice() {
            let size = match sample_size(item, vm)? {
                SampleSize::Mismatch => return Err(vm.new_value_error(SAMPLE_MISMATCH)),
                SampleSize::Value(size) => size,
            };
            let Some(next) = offset.checked_add(size) else {
                return Err(vm.new_value_error(SAMPLE_MISMATCH));
            };
            if next > bytes.len() {
                return Err(vm.new_value_error(SAMPLE_MISMATCH));
            }
            samples.push(&bytes[offset..next]);
            offset = next;
        }
        if offset != bytes.len() {
            return Err(vm.new_value_error(SAMPLE_MISMATCH));
        }
        Ok(samples)
    }

    fn dictionary_failure(finalize: bool, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let verb = if finalize { "finalize" } else { "train" };
        zstd_error(
            format!("Unable to {verb} the Zstandard dictionary: Src size is incorrect"),
            vm,
        )
    }

    fn forced_dict_id(custom: &[u8]) -> u32 {
        let id = read_dict_id(custom).wrapping_add(1);
        if id == 0 { 1 } else { id }
    }

    fn train_or_finalize(
        custom: Option<&[u8]>,
        samples_bytes: &[u8],
        samples_sizes: &PyTupleRef,
        dict_size: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        if dict_size <= 0 {
            return Err(vm.new_value_error(DICT_SIZE_NONPOSITIVE));
        }
        let samples = split_samples(samples_bytes, samples_sizes, vm)?;
        let Ok(max_dict) = usize::try_from(dict_size) else {
            return Err(vm.new_value_error(DICT_SIZE_NONPOSITIVE));
        };
        let dict_id = custom.map(forced_dict_id);
        let opts = TrainOptions {
            max_dict,
            dict_id,
            ..TrainOptions::fastcover()
        };
        match vm.allow_threads(|| train(&samples, opts)) {
            Ok(bytes) => Ok(bytes),
            Err(_) => Err(dictionary_failure(custom.is_some(), vm)),
        }
    }

    #[pyfunction]
    fn train_dict(
        samples_bytes: PyBytesRef,
        samples_sizes: PyTupleRef,
        dict_size: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        train_or_finalize(
            None,
            samples_bytes.as_bytes(),
            &samples_sizes,
            dict_size,
            vm,
        )
    }

    #[pyfunction]
    fn finalize_dict(
        custom_dict_bytes: PyBytesRef,
        samples_bytes: PyBytesRef,
        samples_sizes: PyTupleRef,
        dict_size: isize,
        compression_level: i32,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let _ = compression_level;
        train_or_finalize(
            Some(custom_dict_bytes.as_bytes()),
            samples_bytes.as_bytes(),
            &samples_sizes,
            dict_size,
            vm,
        )
    }

    fn frame_info_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        zstd_error(
            "Error when getting information from the header of a Zstandard frame. \
             Ensure the frame_buffer argument starts from the beginning of a frame, \
             and its length is not less than the frame header (6~18 bytes)."
                .to_owned(),
            vm,
        )
    }

    #[pyfunction]
    fn get_frame_info(frame_buffer: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let kind = frame_buffer.with_ref(get_frame_header);
        let kind = kind.map_err(|_| frame_info_error(vm))?;
        let (size, dict_id) = match kind {
            FrameKind::Zstd(header) => {
                let size = match header.content_size {
                    Some(value) => vm.ctx.new_int(value).into(),
                    None => vm.ctx.none(),
                };
                (size, header.dict_id.unwrap_or(0))
            }
            FrameKind::Skippable { .. } => (vm.ctx.none(), 0),
        };
        Ok(vm.ctx.new_tuple(vec![size, vm.ctx.new_int(dict_id).into()]))
    }

    #[pyfunction]
    fn get_frame_size(frame_buffer: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let result = frame_buffer.with_ref(find_frame_compressed_size);
        match result {
            Ok(size) => Ok(size),
            Err(err) => Err(zstd_error(
                format!(
                    "Error when finding the compressed size of a Zstandard frame. \
                     Ensure the frame_buffer argument starts from the beginning of a frame, \
                     and its length is not less than this complete frame. \
                     Zstd error message: {}.",
                    frame_size_reason(err)
                ),
                vm,
            )),
        }
    }
}
