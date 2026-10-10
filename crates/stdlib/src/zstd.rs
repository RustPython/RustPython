// spell-checker:ignore zstd Zstd ZSTD Zstandard dfast btlazy btultra btopt btultra2 btlazy2
// spell-checker:ignore ldm nbworkers windowlog hashlog chainlog searchlog minmatch dictid
// spell-checker:ignore checksumflag contentsizeflag dictidflag overlaplog jobsize dstream clevel
// spell-checker:ignore skippable pledged fastcover windowlogmax
// cspell:ignore zstd Zstd ZSTD Zstandard dfast btlazy btultra btopt btultra2 btlazy2 ldm nbworkers
// cspell:ignore windowlog hashlog chainlog searchlog minmatch dictid checksumflag skippable pledged

pub(crate) use _zstd::module_def;

#[pymodule]
mod _zstd {
    #![allow(non_upper_case_globals, clippy::upper_case_acronyms)]

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
    use core::ffi::{CStr, c_void};
    use core::fmt;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use libzstd_rs_sys::lib::zdict::{ZDICT_finalizeDictionary, ZDICT_params_t};
    use libzstd_rs_sys::lib::zstd::{ZSTD_e_continue, ZSTD_e_end, ZSTD_e_flush};
    use libzstd_rs_sys::{
        ZDICT_isError, ZDICT_trainFromBuffer, ZSTD_CCtx, ZSTD_CCtx_loadDictionary,
        ZSTD_CCtx_refCDict, ZSTD_CCtx_refPrefix, ZSTD_CCtx_reset, ZSTD_CCtx_setParameter,
        ZSTD_CCtx_setPledgedSrcSize, ZSTD_CONTENTSIZE_ERROR, ZSTD_CONTENTSIZE_UNKNOWN,
        ZSTD_DCtx_loadDictionary, ZSTD_DCtx_refDDict, ZSTD_DCtx_refPrefix, ZSTD_DCtx_reset,
        ZSTD_DCtx_setParameter, ZSTD_DStreamOutSize as dstream_out_size, ZSTD_ResetDirective,
        ZSTD_VERSION_NUMBER, ZSTD_cParam_getBounds, ZSTD_cParameter, ZSTD_compressBound,
        ZSTD_compressStream2, ZSTD_createCCtx, ZSTD_createCDict, ZSTD_createDCtx, ZSTD_createDDict,
        ZSTD_dParam_getBounds, ZSTD_dParameter, ZSTD_decompressStream,
        ZSTD_findFrameCompressedSize, ZSTD_freeCCtx, ZSTD_freeCDict, ZSTD_freeDCtx, ZSTD_freeDDict,
        ZSTD_getDictID_fromDict, ZSTD_getDictID_fromFrame, ZSTD_getErrorName,
        ZSTD_getFrameContentSize, ZSTD_inBuffer, ZSTD_isError, ZSTD_maxCLevel, ZSTD_minCLevel,
        ZSTD_outBuffer,
    };
    use num_traits::ToPrimitive;
    use std::collections::HashMap;

    const _: () = assert!(ZSTD_VERSION_NUMBER == 10_508);
    const _: () = assert!(dstream_out_size() == 131_072);

    const MODE_CONTINUE: i32 = 0;
    const MODE_BLOCK: i32 = 1;
    const MODE_FRAME: i32 = 2;

    const LEVEL_MIN: i32 = ZSTD_minCLevel();
    const LEVEL_MAX: i32 = ZSTD_maxCLevel();
    const DIGESTED: i32 = 0;
    const UNDIGESTED: i32 = 1;
    const PREFIX: i32 = 2;

    const OUTPUT_BLOCKS: [usize; 17] = [
        32 * 1024,
        64 * 1024,
        256 * 1024,
        1024 * 1024,
        4 * 1024 * 1024,
        8 * 1024 * 1024,
        16 * 1024 * 1024,
        16 * 1024 * 1024,
        32 * 1024 * 1024,
        32 * 1024 * 1024,
        32 * 1024 * 1024,
        32 * 1024 * 1024,
        64 * 1024 * 1024,
        64 * 1024 * 1024,
        128 * 1024 * 1024,
        128 * 1024 * 1024,
        256 * 1024 * 1024,
    ];

    const SAMPLE_MISMATCH: &str = "The samples size tuple doesn't match the concatenation's size.";
    const DICT_SIZE_NONPOSITIVE: &str = "dict_size argument should be positive number.";
    const DIGESTED_COMPRESS_FAILED: &str =
        "Failed to create a ZSTD_CDict instance from Zstandard dictionary content.";
    const DIGESTED_DECOMPRESS_FAILED: &str =
        "Failed to create a ZSTD_DDict instance from Zstandard dictionary content.";
    const BUFFER_FAILED: &str = "Unable to allocate output buffer.";

    #[pyattr]
    const zstd_version: &str = "1.5.8";
    #[pyattr]
    const zstd_version_number: i32 = ZSTD_VERSION_NUMBER as i32;
    #[pyattr]
    const ZSTD_CLEVEL_DEFAULT: i32 = libzstd_rs_sys::ZSTD_CLEVEL_DEFAULT;
    #[pyattr]
    const ZSTD_DStreamOutSize: i32 = dstream_out_size() as i32;

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

    const _: () = assert!(
        c_parameter_id(libzstd_rs_sys::ZSTD_cParameter::ZSTD_c_compressionLevel)
            == ZSTD_c_compressionLevel as u32
    );
    const _: () = assert!(
        c_parameter_id(libzstd_rs_sys::ZSTD_cParameter::ZSTD_c_nbWorkers)
            == ZSTD_c_nbWorkers as u32
    );
    const _: () = assert!(
        d_parameter_id(libzstd_rs_sys::ZSTD_dParameter::ZSTD_d_windowLogMax)
            == ZSTD_d_windowLogMax as u32
    );

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

    enum CallKind {
        Compress,
        Decompress,
        Pledge,
        LoadCompress,
        LoadDecompress,
        Level,
        Train,
        Finalize,
        BoundsCompress,
        BoundsDecompress,
        FrameSize,
    }

    enum CodecError {
        NoMemory,
        Buffer,
        Static(&'static str),
        Named(CallKind, usize),
        Eof,
    }

    fn error_name(code: usize) -> String {
        let ptr = ZSTD_getErrorName(code);
        if ptr.is_null() {
            return String::new();
        }
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn raise_codec(err: CodecError, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match err {
            CodecError::NoMemory => vm.no_memory_error(),
            CodecError::Buffer => vm.new_memory_error(BUFFER_FAILED),
            CodecError::Static(message) => zstd_error(message.to_owned(), vm),
            CodecError::Eof => vm.new_eof_error("Already at the end of a Zstandard frame."),
            CodecError::Named(kind, code) => {
                let name = error_name(code);
                let message = match kind {
                    CallKind::Compress => format!("Unable to compress Zstandard data: {name}"),
                    CallKind::Decompress => {
                        format!("Unable to decompress Zstandard data: {name}")
                    }
                    CallKind::Pledge => {
                        format!("Unable to set pledged uncompressed content size: {name}")
                    }
                    CallKind::LoadCompress => format!(
                        "Unable to load Zstandard dictionary or prefix for compression: {name}"
                    ),
                    CallKind::LoadDecompress => format!(
                        "Unable to load Zstandard dictionary or prefix for decompression: {name}"
                    ),
                    CallKind::Level => format!("Unable to set zstd compression level: {name}"),
                    CallKind::Train => {
                        format!("Unable to train the Zstandard dictionary: {name}")
                    }
                    CallKind::Finalize => {
                        format!("Unable to finalize the Zstandard dictionary: {name}")
                    }
                    CallKind::BoundsCompress => {
                        format!("Unable to get zstd compression parameter bounds: {name}")
                    }
                    CallKind::BoundsDecompress => {
                        format!("Unable to get zstd decompression parameter bounds: {name}")
                    }
                    CallKind::FrameSize => format!(
                        "Error when finding the compressed size of a Zstandard frame. \
                         Ensure the frame_buffer argument starts from the beginning of a frame, \
                         and its length is not less than this complete frame. \
                         Zstd error message: {name}."
                    ),
                };
                zstd_error(message, vm)
            }
        }
    }

    fn is_error(code: usize) -> bool {
        ZSTD_isError(code) != 0
    }

    const fn c_parameter_id(param: ZSTD_cParameter) -> u32 {
        unsafe { core::mem::transmute(param) }
    }

    const fn d_parameter_id(param: ZSTD_dParameter) -> u32 {
        unsafe { core::mem::transmute(param) }
    }

    fn c_parameter(id: i32) -> ZSTD_cParameter {
        unsafe { core::mem::transmute(id as u32) }
    }

    fn d_parameter(id: i32) -> ZSTD_dParameter {
        unsafe { core::mem::transmute(id as u32) }
    }

    fn end_op(mode: i32) -> libzstd_rs_sys::ZSTD_EndDirective {
        match mode {
            MODE_BLOCK => ZSTD_e_flush,
            MODE_FRAME => ZSTD_e_end,
            _ => ZSTD_e_continue,
        }
    }

    fn ffi_bytes(bytes: &[u8], scratch: &u8) -> (*const c_void, usize) {
        if bytes.is_empty() {
            (core::ptr::from_ref(scratch).cast(), 0)
        } else {
            (bytes.as_ptr().cast(), bytes.len())
        }
    }

    fn in_buffer(src: &[u8], scratch: &u8) -> ZSTD_inBuffer {
        let (ptr, size) = ffi_bytes(src, scratch);
        ZSTD_inBuffer {
            src: ptr,
            size,
            pos: 0,
        }
    }

    struct CCtx(*mut ZSTD_CCtx);
    // The context moves between threads; the owning mutex is what serializes use.
    unsafe impl Send for CCtx {}
    impl Drop for CCtx {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    ZSTD_freeCCtx(self.0);
                }
                self.0 = core::ptr::null_mut();
            }
        }
    }

    struct DCtx(*mut libzstd_rs_sys::ZSTD_DCtx);
    unsafe impl Send for DCtx {}
    impl Drop for DCtx {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    ZSTD_freeDCtx(self.0);
                }
                self.0 = core::ptr::null_mut();
            }
        }
    }

    struct CDict(*mut libzstd_rs_sys::ZSTD_CDict);
    unsafe impl Send for CDict {}
    impl Drop for CDict {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    ZSTD_freeCDict(self.0);
                }
                self.0 = core::ptr::null_mut();
            }
        }
    }

    struct DDict(*mut libzstd_rs_sys::ZSTD_DDict);
    unsafe impl Send for DDict {}
    impl Drop for DDict {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    ZSTD_freeDDict(self.0);
                }
                self.0 = core::ptr::null_mut();
            }
        }
    }

    fn known_parameter_name(compress: bool, key: i32) -> Option<&'static str> {
        if compress {
            let name = match key {
                ZSTD_c_compressionLevel => "compression_level",
                ZSTD_c_windowLog => "window_log",
                ZSTD_c_hashLog => "hash_log",
                ZSTD_c_chainLog => "chain_log",
                ZSTD_c_searchLog => "search_log",
                ZSTD_c_minMatch => "min_match",
                ZSTD_c_targetLength => "target_length",
                ZSTD_c_strategy => "strategy",
                ZSTD_c_enableLongDistanceMatching => "enable_long_distance_matching",
                ZSTD_c_ldmHashLog => "ldm_hash_log",
                ZSTD_c_ldmMinMatch => "ldm_min_match",
                ZSTD_c_ldmBucketSizeLog => "ldm_bucket_size_log",
                ZSTD_c_ldmHashRateLog => "ldm_hash_rate_log",
                ZSTD_c_contentSizeFlag => "content_size_flag",
                ZSTD_c_checksumFlag => "checksum_flag",
                ZSTD_c_dictIDFlag => "dict_id_flag",
                ZSTD_c_nbWorkers => "nb_workers",
                ZSTD_c_jobSize => "job_size",
                ZSTD_c_overlapLog => "overlap_log",
                _ => return None,
            };
            Some(name)
        } else if key == ZSTD_d_windowLogMax {
            Some("window_log_max")
        } else {
            None
        }
    }

    fn query_bounds(compress: bool, key: i32) -> Result<(i32, i32), usize> {
        let (error, lower, upper) = if compress {
            let bounds = ZSTD_cParam_getBounds(c_parameter(key));
            (bounds.error, bounds.lowerBound, bounds.upperBound)
        } else {
            let bounds = ZSTD_dParam_getBounds(d_parameter(key));
            (bounds.error, bounds.lowerBound, bounds.upperBound)
        };
        if is_error(error) {
            Err(error)
        } else {
            Ok((lower, upper))
        }
    }

    fn parameter_value_error(
        compress: bool,
        key: i32,
        value: i32,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let kind = if compress {
            "compression"
        } else {
            "decompression"
        };
        let name = match known_parameter_name(compress, key) {
            Some(name) => name.to_owned(),
            None => format!("unknown parameter (key {key})"),
        };
        match query_bounds(compress, key) {
            Err(_) => vm.new_value_error(format!("invalid {kind} parameter '{name}'")),
            Ok((lower, upper)) => vm.new_value_error(format!(
                "{kind} parameter '{name}' received an illegal value {value}; \
                 the valid range is [{lower}, {upper}]"
            )),
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

    fn parse_level(obj: &PyObject, vm: &VirtualMachine) -> PyResult<i32> {
        if !is_pylong(obj, vm) {
            return Err(vm.new_type_error("invalid type for level, expected int"));
        }
        match obj.try_to_value::<i32>(vm) {
            Ok(level) => Ok(level),
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
        let (lower, upper) = query_bounds(is_compress, parameter).map_err(|code| {
            let kind = if is_compress {
                CallKind::BoundsCompress
            } else {
                CallKind::BoundsDecompress
            };
            raise_codec(CodecError::Named(kind, code), vm)
        })?;
        Ok(vm.ctx.new_tuple(vec![
            vm.ctx.new_int(lower).into(),
            vm.ctx.new_int(upper).into(),
        ]))
    }

    struct DictCache {
        cdicts: HashMap<i32, CDict>,
        ddict: Option<DDict>,
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
        // Freed before `dict_content` so digested dictionaries do not outlive the bytes.
        #[pytraverse(skip)]
        cache: PyMutex<DictCache>,
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
            let dict_id =
                unsafe { ZSTD_getDictID_fromDict(content.as_ptr().cast(), content.len()) };
            if !args.is_raw && dict_id == 0 {
                return Err(vm.new_value_error("invalid Zstandard dictionary"));
            }
            Ok(Self {
                cache: PyMutex::new(DictCache {
                    cdicts: HashMap::new(),
                    ddict: None,
                }),
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

    impl DictArg {
        fn parts(self, compress: bool) -> (PyRef<ZstdDict>, i32) {
            match self {
                Self::Bare(dict) => {
                    let marker = if compress { UNDIGESTED } else { DIGESTED };
                    (dict, marker)
                }
                Self::Marked(dict, marker) => (dict, marker),
            }
        }
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

    fn cdict_at_level(
        dict: &ZstdDict,
        level: i32,
    ) -> Result<*const libzstd_rs_sys::ZSTD_CDict, CodecError> {
        {
            let cache = dict.cache.lock();
            if let Some(existing) = cache.cdicts.get(&level) {
                return Ok(existing.0);
            }
        }
        let bytes = dict.dict_content.as_bytes();
        let created = unsafe { ZSTD_createCDict(bytes.as_ptr().cast(), bytes.len(), level) };
        if created.is_null() {
            return Err(CodecError::Static(DIGESTED_COMPRESS_FAILED));
        }
        let mut cache = dict.cache.lock();
        if let Some(existing) = cache.cdicts.get(&level) {
            unsafe {
                ZSTD_freeCDict(created);
            }
            return Ok(existing.0);
        }
        let ptr = created as *const libzstd_rs_sys::ZSTD_CDict;
        cache.cdicts.insert(level, CDict(created));
        Ok(ptr)
    }

    fn ddict_of(dict: &ZstdDict) -> Result<*const libzstd_rs_sys::ZSTD_DDict, CodecError> {
        {
            let cache = dict.cache.lock();
            if let Some(existing) = cache.ddict.as_ref() {
                return Ok(existing.0);
            }
        }
        let bytes = dict.dict_content.as_bytes();
        let created = unsafe { ZSTD_createDDict(bytes.as_ptr().cast(), bytes.len()) };
        if created.is_null() {
            return Err(CodecError::Static(DIGESTED_DECOMPRESS_FAILED));
        }
        let mut cache = dict.cache.lock();
        if let Some(existing) = cache.ddict.as_ref() {
            unsafe {
                ZSTD_freeDDict(created);
            }
            return Ok(existing.0);
        }
        let ptr = created as *const libzstd_rs_sys::ZSTD_DDict;
        cache.ddict = Some(DDict(created));
        Ok(ptr)
    }

    struct OutBuf {
        data: Vec<u8>,
        filled: usize,
        block_size: usize,
        written_in_block: usize,
        blocks: usize,
        max_length: Option<usize>,
    }

    impl OutBuf {
        fn new(block: usize, max_length: Option<usize>) -> Result<Self, CodecError> {
            let mut buf = Self {
                data: Vec::new(),
                filled: 0,
                block_size: block,
                written_in_block: 0,
                blocks: 1,
                max_length,
            };
            buf.reserve()?;
            Ok(buf)
        }

        fn reserve(&mut self) -> Result<(), CodecError> {
            let need = self.filled.saturating_add(self.block_size);
            if self.data.capacity() < need {
                let extra = need - self.data.capacity();
                self.data
                    .try_reserve(extra)
                    .map_err(|_| CodecError::Buffer)?;
            }
            Ok(())
        }

        fn as_zstd(&mut self, scratch: &mut u8) -> ZSTD_outBuffer {
            let dst = if self.block_size == 0 {
                core::ptr::from_mut(scratch).cast()
            } else {
                unsafe { self.data.as_mut_ptr().add(self.filled).cast() }
            };
            ZSTD_outBuffer {
                dst,
                size: self.block_size,
                pos: self.written_in_block,
            }
        }

        fn sync(&mut self, pos: usize) {
            let pos = pos.min(self.block_size);
            self.written_in_block = pos;
            let new_len = self.filled + pos;
            if new_len > self.data.len() {
                // `pos` bytes at the end of the current block were written by zstd.
                unsafe { self.data.set_len(new_len) };
            } else if new_len < self.data.len() {
                self.data.truncate(new_len);
            }
        }

        fn grow(&mut self) -> Result<(), CodecError> {
            self.filled += self.written_in_block;
            self.written_in_block = 0;
            self.block_size = next_block(self.blocks, self.max_length, self.filled)?;
            self.blocks += 1;
            self.reserve()
        }

        fn into_vec(self) -> Vec<u8> {
            self.data
        }
    }

    fn next_block(
        index: usize,
        max_length: Option<usize>,
        allocated: usize,
    ) -> Result<usize, CodecError> {
        let mut block = OUTPUT_BLOCKS
            .get(index)
            .copied()
            .unwrap_or(OUTPUT_BLOCKS[OUTPUT_BLOCKS.len() - 1]);
        if let Some(max) = max_length {
            let rest = max.saturating_sub(allocated);
            if rest == 0 {
                return Err(CodecError::Buffer);
            }
            block = block.min(rest);
        }
        let limit = isize::MAX as usize;
        if allocated > limit || block > limit - allocated {
            return Err(CodecError::Buffer);
        }
        Ok(block)
    }

    fn decompressor_out(max_length: Option<usize>) -> Result<OutBuf, CodecError> {
        let block = match max_length {
            Some(max) if max < OUTPUT_BLOCKS[0] => max,
            _ => OUTPUT_BLOCKS[0],
        };
        OutBuf::new(block, max_length)
    }

    struct CompState {
        cctx: CCtx,
        use_multithread: bool,
        compression_level: i32,
    }

    impl CompState {
        fn new() -> Result<Self, CodecError> {
            let cctx = unsafe { ZSTD_createCCtx() };
            if cctx.is_null() {
                return Err(CodecError::Static("Unable to create ZSTD_CCtx instance."));
            }
            Ok(Self {
                cctx: CCtx(cctx),
                use_multithread: false,
                compression_level: ZSTD_CLEVEL_DEFAULT,
            })
        }

        fn reset_session(&mut self) {
            unsafe {
                ZSTD_CCtx_reset(self.cctx.0, ZSTD_ResetDirective::ZSTD_reset_session_only);
            }
        }

        fn set_level(&mut self, level: i32, vm: &VirtualMachine) -> PyResult<()> {
            if !(LEVEL_MIN..=LEVEL_MAX).contains(&level) {
                return Err(vm.new_value_error(format!(
                    "illegal compression level {level}; the valid range is [{LEVEL_MIN}, {LEVEL_MAX}]"
                )));
            }
            self.compression_level = level;
            let code = unsafe {
                ZSTD_CCtx_setParameter(self.cctx.0, c_parameter(ZSTD_c_compressionLevel), level)
            };
            if is_error(code) {
                return Err(raise_codec(CodecError::Named(CallKind::Level, code), vm));
            }
            Ok(())
        }

        fn apply_options(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let dict = options_dict(obj, true, vm)?;
            for_each_option(
                dict,
                true,
                |key, value| {
                    if key == ZSTD_c_compressionLevel {
                        return self.set_level(value, vm);
                    }
                    if key == ZSTD_c_nbWorkers && value != 0 {
                        self.use_multithread = true;
                    }
                    let code =
                        unsafe { ZSTD_CCtx_setParameter(self.cctx.0, c_parameter(key), value) };
                    if is_error(code) {
                        return Err(parameter_value_error(true, key, value, vm));
                    }
                    Ok(())
                },
                vm,
            )
        }

        fn load_dict(&mut self, dict: &ZstdDict, marker: i32) -> Result<(), CodecError> {
            let code = match marker {
                DIGESTED => {
                    let cdict = cdict_at_level(dict, self.compression_level)?;
                    unsafe { ZSTD_CCtx_refCDict(self.cctx.0, cdict) }
                }
                PREFIX => {
                    let bytes = dict.dict_content.as_bytes();
                    unsafe { ZSTD_CCtx_refPrefix(self.cctx.0, bytes.as_ptr().cast(), bytes.len()) }
                }
                _ => {
                    let bytes = dict.dict_content.as_bytes();
                    unsafe {
                        ZSTD_CCtx_loadDictionary(self.cctx.0, bytes.as_ptr().cast(), bytes.len())
                    }
                }
            };
            if is_error(code) {
                Err(CodecError::Named(CallKind::LoadCompress, code))
            } else {
                Ok(())
            }
        }
    }

    fn compress_locked(
        state: &mut CompState,
        src: &[u8],
        mode: i32,
        mt_continue: bool,
    ) -> Result<Vec<u8>, CodecError> {
        let in_scratch = 0u8;
        let mut input = in_buffer(src, &in_scratch);
        let mut output = if mt_continue {
            OutBuf::new(OUTPUT_BLOCKS[0], None)?
        } else {
            let bound = ZSTD_compressBound(src.len());
            if is_error(bound) || bound > isize::MAX as usize {
                return Err(CodecError::NoMemory);
            }
            OutBuf::new(bound, None)?
        };
        let directive = end_op(mode);
        loop {
            let mut out_scratch = 0u8;
            let mut out = output.as_zstd(&mut out_scratch);
            let before = (input.pos, out.pos);
            let ret = if mt_continue {
                loop {
                    let ret = unsafe {
                        ZSTD_compressStream2(state.cctx.0, &mut out, &mut input, directive)
                    };
                    if is_error(ret) || out.pos == out.size || input.pos == input.size {
                        break ret;
                    }
                }
            } else {
                unsafe { ZSTD_compressStream2(state.cctx.0, &mut out, &mut input, directive) }
            };
            let pos = out.pos;
            let filled = pos == out.size;
            output.sync(pos);
            if is_error(ret) {
                return Err(CodecError::Named(CallKind::Compress, ret));
            }
            if !mt_continue && ret == 0 {
                break;
            }
            if filled {
                output.grow()?;
                continue;
            }
            if mt_continue {
                break;
            }
            if (input.pos, pos) == before {
                return Err(CodecError::Named(CallKind::Compress, ret));
            }
        }
        Ok(output.into_vec())
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
        // `state` is first so the context is freed before the dictionary it references.
        #[pytraverse(skip)]
        state: PyMutex<CompState>,
        dict_ref: Option<PyRef<ZstdDict>>,
        #[pytraverse(skip)]
        last_mode: AtomicI32,
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
            let mut dict_ref: Option<PyRef<ZstdDict>> = None;
            let mut comp = CompState::new().map_err(|err| raise_codec(err, vm))?;
            if let Some(level) = level.as_ref() {
                let level = parse_level(level, vm)?;
                comp.set_level(level, vm)?;
            }
            if let Some(options) = options.as_ref() {
                comp.apply_options(options, vm)?;
            }
            if let Some(obj) = zstd_dict.as_ref() {
                let (dict, marker) = parse_dict_arg(obj, vm)?.parts(true);
                let loaded = vm.allow_threads(|| comp.load_dict(&dict, marker));
                loaded.map_err(|err| raise_codec(err, vm))?;
                dict_ref = Some(dict);
            }
            Ok(Self {
                state: PyMutex::new(comp),
                dict_ref,
                last_mode: AtomicI32::new(MODE_FRAME),
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

    fn drive_compress(
        zelf: &Py<ZstdCompressor>,
        src: &[u8],
        mode: i32,
        allow_mt: bool,
    ) -> Result<Vec<u8>, CodecError> {
        let mut state = zelf.state.lock();
        let mt_continue = allow_mt && state.use_multithread && mode == MODE_CONTINUE;
        match compress_locked(&mut state, src, mode, mt_continue) {
            Ok(bytes) => {
                zelf.last_mode.store(mode, Ordering::Release);
                Ok(bytes)
            }
            Err(err) => {
                state.reset_session();
                zelf.last_mode.store(MODE_FRAME, Ordering::Release);
                Err(err)
            }
        }
    }

    #[pyclass(with(Constructor))]
    impl ZstdCompressor {
        #[pymethod]
        fn compress(
            zelf: &Py<Self>,
            args: CompressCallArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let CompressCallArgs { data, mode } = args;
            check_compress_mode(mode, vm)?;
            let input = data.with_ref(<[u8]>::to_vec);
            vm.allow_threads(|| drive_compress(zelf, &input, mode, true))
                .map_err(|err| raise_codec(err, vm))
        }

        #[pymethod]
        fn flush(zelf: &Py<Self>, args: FlushCallArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let FlushCallArgs { mode } = args;
            check_flush_mode(mode, vm)?;
            vm.allow_threads(|| drive_compress(zelf, &[], mode, false))
                .map_err(|err| raise_codec(err, vm))
        }

        #[pymethod]
        fn set_pledged_input_size(
            zelf: &Py<Self>,
            size: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let pledged = parse_pledge(&size, vm)?;
            let result = vm.allow_threads(|| {
                let state = zelf.state.lock();
                if zelf.last_mode.load(Ordering::Acquire) != MODE_FRAME {
                    return Err(None);
                }
                let code = unsafe { ZSTD_CCtx_setPledgedSrcSize(state.cctx.0, pledged) };
                if is_error(code) {
                    Err(Some(code))
                } else {
                    Ok(())
                }
            });
            match result {
                Ok(()) => Ok(()),
                Err(None) => Err(vm.new_value_error(
                    "set_pledged_input_size() method must be called when last_mode == FLUSH_FRAME",
                )),
                Err(Some(code)) => Err(raise_codec(CodecError::Named(CallKind::Pledge, code), vm)),
            }
        }

        #[pygetset]
        fn last_mode(zelf: &Py<Self>) -> i32 {
            zelf.last_mode.load(Ordering::Acquire)
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

    fn parse_pledge(size: &PyObject, vm: &VirtualMachine) -> PyResult<u64> {
        if vm.is_none(size) {
            return Ok(ZSTD_CONTENTSIZE_UNKNOWN);
        }
        if !is_pylong(size, vm) {
            return Err(vm.new_type_error("an integer is required"));
        }
        let int_obj = size.try_index(vm)?;
        match int_obj.as_bigint().to_u64() {
            Some(value) if value < ZSTD_CONTENTSIZE_ERROR => Ok(value),
            _ => Err(vm.new_value_error(format!(
                "size argument should be a positive int less than {ZSTD_CONTENTSIZE_ERROR}"
            ))),
        }
    }

    struct DecState {
        dctx: DCtx,
        pending: Vec<u8>,
        eof: bool,
        needs_input: bool,
    }

    impl DecState {
        fn new() -> Result<Self, CodecError> {
            let dctx = unsafe { ZSTD_createDCtx() };
            if dctx.is_null() {
                return Err(CodecError::Static("Unable to create ZSTD_DCtx instance."));
            }
            Ok(Self {
                dctx: DCtx(dctx),
                pending: Vec::new(),
                eof: false,
                needs_input: true,
            })
        }

        fn reset_session(&mut self) {
            self.pending.clear();
            self.eof = false;
            self.needs_input = true;
            unsafe {
                ZSTD_DCtx_reset(self.dctx.0, ZSTD_ResetDirective::ZSTD_reset_session_only);
            }
        }

        fn publish(&self, eof: &AtomicBool, needs_input: &AtomicBool) {
            eof.store(self.eof, Ordering::Release);
            needs_input.store(self.needs_input, Ordering::Release);
        }

        fn apply_options(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let dict = options_dict(obj, false, vm)?;
            for_each_option(
                dict,
                false,
                |key, value| {
                    let code =
                        unsafe { ZSTD_DCtx_setParameter(self.dctx.0, d_parameter(key), value) };
                    if is_error(code) {
                        return Err(parameter_value_error(false, key, value, vm));
                    }
                    Ok(())
                },
                vm,
            )
        }

        fn load_dict(&mut self, dict: &ZstdDict, marker: i32) -> Result<(), CodecError> {
            let code = match marker {
                UNDIGESTED => {
                    let bytes = dict.dict_content.as_bytes();
                    unsafe {
                        ZSTD_DCtx_loadDictionary(self.dctx.0, bytes.as_ptr().cast(), bytes.len())
                    }
                }
                PREFIX => {
                    let bytes = dict.dict_content.as_bytes();
                    unsafe { ZSTD_DCtx_refPrefix(self.dctx.0, bytes.as_ptr().cast(), bytes.len()) }
                }
                _ => {
                    let ddict = ddict_of(dict)?;
                    unsafe { ZSTD_DCtx_refDDict(self.dctx.0, ddict) }
                }
            };
            if is_error(code) {
                Err(CodecError::Named(CallKind::LoadDecompress, code))
            } else {
                Ok(())
            }
        }

        fn decompress(
            &mut self,
            data: &[u8],
            max_length: Option<usize>,
        ) -> Result<Vec<u8>, CodecError> {
            if self.eof {
                return Err(CodecError::Eof);
            }
            self.pending
                .try_reserve(data.len())
                .map_err(|_| CodecError::Buffer)?;
            self.pending.extend_from_slice(data);

            let in_scratch = 0u8;
            let mut input = in_buffer(&self.pending, &in_scratch);
            let mut output = decompressor_out(max_length)?;
            loop {
                let mut out_scratch = 0u8;
                let mut out = output.as_zstd(&mut out_scratch);
                let ret = unsafe { ZSTD_decompressStream(self.dctx.0, &mut out, &mut input) };
                let pos = out.pos;
                let filled = pos == out.size;
                output.sync(pos);
                if is_error(ret) {
                    return Err(CodecError::Named(CallKind::Decompress, ret));
                }
                if ret == 0 {
                    self.eof = true;
                    break;
                }
                if filled {
                    let produced = self_produced(&output);
                    if max_length.is_some_and(|max| produced == max) {
                        break;
                    }
                    output.grow()?;
                    continue;
                }
                if input.pos == input.size {
                    break;
                }
            }

            let produced = output.into_vec();
            if input.pos == input.size {
                self.pending.clear();
                let hit = max_length.is_some_and(|max| produced.len() == max);
                self.needs_input = !(hit || self.eof);
            } else {
                self.needs_input = false;
                let pos = input.pos;
                if pos > 0 {
                    self.pending.drain(..pos);
                }
            }
            Ok(produced)
        }
    }

    fn self_produced(output: &OutBuf) -> usize {
        output.filled + output.written_in_block
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
        // `state` is first so the context is freed before the dictionary it references.
        #[pytraverse(skip)]
        state: PyMutex<DecState>,
        dict_ref: Option<PyRef<ZstdDict>>,
        #[pymember]
        #[pytraverse(skip)]
        eof: AtomicBool,
        #[pymember]
        #[pytraverse(skip)]
        needs_input: AtomicBool,
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
            let mut dict_ref: Option<PyRef<ZstdDict>> = None;
            let mut dec = DecState::new().map_err(|err| raise_codec(err, vm))?;
            if let Some(obj) = zstd_dict.as_ref() {
                let (dict, marker) = parse_dict_arg(obj, vm)?.parts(false);
                let loaded = vm.allow_threads(|| dec.load_dict(&dict, marker));
                loaded.map_err(|err| raise_codec(err, vm))?;
                dict_ref = Some(dict);
            }
            if let Some(options) = options.as_ref() {
                dec.apply_options(options, vm)?;
            }
            let eof = dec.eof;
            let needs_input = dec.needs_input;
            Ok(Self {
                state: PyMutex::new(dec),
                dict_ref,
                eof: AtomicBool::new(eof),
                needs_input: AtomicBool::new(needs_input),
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
            let max_length = args.max_length();
            let data = args.data().to_vec();
            let outcome = vm.allow_threads(|| {
                let mut state = zelf.state.lock();
                match state.decompress(&data, max_length) {
                    Ok(bytes) => {
                        state.publish(&zelf.eof, &zelf.needs_input);
                        Ok(bytes)
                    }
                    Err(CodecError::Eof) => Err(CodecError::Eof),
                    Err(err) => {
                        state.reset_session();
                        state.publish(&zelf.eof, &zelf.needs_input);
                        Err(err)
                    }
                }
            });
            outcome.map_err(|err| raise_codec(err, vm))
        }

        #[pygetset]
        fn unused_data(zelf: &Py<Self>, vm: &VirtualMachine) -> PyBytesRef {
            let unused = {
                let state = zelf.state.lock();
                if state.eof {
                    state.pending.clone()
                } else {
                    Vec::new()
                }
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

    fn sample_sizes(bytes: &[u8], sizes: &PyTupleRef, vm: &VirtualMachine) -> PyResult<Vec<usize>> {
        let items = sizes.as_slice();
        if u32::try_from(items.len()).is_err() {
            return Err(
                vm.new_value_error(format!("The number of samples should be <= {}.", u32::MAX))
            );
        }
        let mut chunks = Vec::with_capacity(items.len());
        let mut offset = 0usize;
        for item in items {
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
            chunks.push(size);
            offset = next;
        }
        if offset != bytes.len() {
            return Err(vm.new_value_error(SAMPLE_MISMATCH));
        }
        Ok(chunks)
    }

    fn build_dict(
        custom: Option<&[u8]>,
        samples: &[u8],
        sizes: &PyTupleRef,
        dict_size: isize,
        compression_level: i32,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        if dict_size <= 0 {
            return Err(vm.new_value_error(DICT_SIZE_NONPOSITIVE));
        }
        let chunks = sample_sizes(samples, sizes, vm)?;
        let Ok(cap) = usize::try_from(dict_size) else {
            return Err(vm.new_value_error(DICT_SIZE_NONPOSITIVE));
        };
        let mut dst = Vec::new();
        dst.try_reserve_exact(cap)
            .map_err(|_| vm.no_memory_error())?;
        dst.resize(cap, 0);
        let nb = u32::try_from(chunks.len()).unwrap_or(0);
        let code = vm.allow_threads(|| {
            let scratch = 0u8;
            let (samples_ptr, _) = ffi_bytes(samples, &scratch);
            let sizes_ptr = if chunks.is_empty() {
                core::ptr::null()
            } else {
                chunks.as_ptr()
            };
            unsafe {
                if let Some(custom) = custom {
                    let (custom_ptr, custom_len) = ffi_bytes(custom, &scratch);
                    ZDICT_finalizeDictionary(
                        dst.as_mut_ptr().cast(),
                        cap,
                        custom_ptr,
                        custom_len,
                        samples_ptr,
                        sizes_ptr,
                        nb,
                        ZDICT_params_t {
                            compressionLevel: compression_level,
                            notificationLevel: 0,
                            dictID: 0,
                        },
                    )
                } else {
                    ZDICT_trainFromBuffer(dst.as_mut_ptr().cast(), cap, samples_ptr, sizes_ptr, nb)
                }
            }
        });
        if ZDICT_isError(code) != 0 {
            let kind = if custom.is_some() {
                CallKind::Finalize
            } else {
                CallKind::Train
            };
            return Err(raise_codec(CodecError::Named(kind, code), vm));
        }
        if code > cap {
            return Err(vm.no_memory_error());
        }
        dst.truncate(code);
        Ok(dst)
    }

    #[pyfunction]
    fn train_dict(
        samples_bytes: PyBytesRef,
        samples_sizes: PyTupleRef,
        dict_size: isize,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        build_dict(
            None,
            samples_bytes.as_bytes(),
            &samples_sizes,
            dict_size,
            0,
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
        build_dict(
            Some(custom_dict_bytes.as_bytes()),
            samples_bytes.as_bytes(),
            &samples_sizes,
            dict_size,
            compression_level,
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
        let bytes = frame_buffer.with_ref(<[u8]>::to_vec);
        let scratch = 0u8;
        let (ptr, len) = ffi_bytes(&bytes, &scratch);
        let size = unsafe { ZSTD_getFrameContentSize(ptr, len) };
        if size == ZSTD_CONTENTSIZE_ERROR {
            return Err(frame_info_error(vm));
        }
        let dict_id = unsafe { ZSTD_getDictID_fromFrame(ptr, len) };
        let size_obj = if size == ZSTD_CONTENTSIZE_UNKNOWN {
            vm.ctx.none()
        } else {
            vm.ctx.new_int(size).into()
        };
        Ok(vm
            .ctx
            .new_tuple(vec![size_obj, vm.ctx.new_int(dict_id).into()]))
    }

    #[pyfunction]
    fn get_frame_size(frame_buffer: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let bytes = frame_buffer.with_ref(<[u8]>::to_vec);
        let scratch = 0u8;
        let (ptr, len) = ffi_bytes(&bytes, &scratch);
        let size = unsafe { ZSTD_findFrameCompressedSize(ptr, len) };
        if is_error(size) {
            Err(raise_codec(
                CodecError::Named(CallKind::FrameSize, size),
                vm,
            ))
        } else {
            Ok(size)
        }
    }
}
