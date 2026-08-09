// spell-checker:ignore cctx dctx CCTX DCTX ldm cdict ddict windowlog hashlog chainlog searchlog
// spell-checker:ignore minmatch dictid checksumflag dstream cstream pyobj zstandard btopt btultra
// spell-checker:ignore btlazy dfast nbworkers windowlogmax windowlog overlap targetcblock
// spell-checker:ignore srcsize zdict refprefix refcdict refddict pledgedsrcsize getframecontentsize
// spell-checker:ignore Zstd Zstandard pylib RFC
// spell-checker:ignore CLEVEL zstdmodule cparameter dparameter maxl c2rust

//! The `_zstd` extension module. Backs the pure-Python `compression.zstd`
//! package by exposing the same classes, functions and constants that
//! CPython's `Modules/_zstd/` exposes. The Python wrapper at
//! `Lib/compression/zstd/__init__.py` imports from this module unconditionally,
//! so the names and call signatures here must stay in sync with CPython.
//!
//! Backend: the `libzstd-rs-sys` crate (Trifecta Tech Foundation's pure-Rust
//! c2rust translation of Facebook's libzstd, the C library CPython links
//! against), used directly through its `unsafe extern "C"` API surface.

pub(crate) use _zstd::module_def;

// The compression/decompression parameter and strategy constants below use
// CPython's `ZSTD_c_camelCase` / `ZSTD_d_camelCase` naming convention so the
// pure-Python `compression.zstd` package, which references them by those exact
// names, keeps working unchanged.
#[allow(non_upper_case_globals)]
#[pymodule]
mod _zstd {
    use core::ffi::{CStr, c_int};
    use libzstd_rs_sys::lib::compress::zstd_compress::{ZSTD_e_continue, ZSTD_e_end, ZSTD_e_flush};
    use libzstd_rs_sys::lib::zdict::ZDICT_finalizeDictionary;
    use libzstd_rs_sys::{
        ZDICT_getErrorName, ZDICT_isError, ZDICT_params_t, ZDICT_trainFromBuffer, ZSTD_CCtx,
        ZSTD_CCtx_loadDictionary, ZSTD_CCtx_refCDict, ZSTD_CCtx_refPrefix, ZSTD_CCtx_setParameter,
        ZSTD_CCtx_setPledgedSrcSize, ZSTD_CDict, ZSTD_CONTENTSIZE_ERROR, ZSTD_CONTENTSIZE_UNKNOWN,
        ZSTD_CStreamOutSize, ZSTD_DCtx, ZSTD_DCtx_loadDictionary, ZSTD_DCtx_refDDict,
        ZSTD_DCtx_refPrefix, ZSTD_DCtx_setParameter, ZSTD_DDict, ZSTD_DStreamOutSize,
        ZSTD_EndDirective, ZSTD_cParam_getBounds, ZSTD_cParameter, ZSTD_compressStream2,
        ZSTD_createCCtx, ZSTD_createCDict, ZSTD_createDCtx, ZSTD_createDDict,
        ZSTD_dParam_getBounds, ZSTD_dParameter, ZSTD_decompressStream,
        ZSTD_findFrameCompressedSize, ZSTD_freeCCtx, ZSTD_freeCDict, ZSTD_freeDCtx, ZSTD_freeDDict,
        ZSTD_getDictID_fromDict, ZSTD_getDictID_fromFrame, ZSTD_getErrorName,
        ZSTD_getFrameContentSize, ZSTD_inBuffer, ZSTD_isError, ZSTD_outBuffer, ZSTD_strategy,
        ZSTD_versionNumber, ZSTD_versionString,
    };
    use rustpython_common::lock::PyMutex;
    use rustpython_vm::builtins::{
        PyBaseExceptionRef, PyBytesRef, PyDict, PyTupleRef, PyType, PyTypeRef,
    };
    use rustpython_vm::function::{ArgBytesLike, OptionalOption};
    use rustpython_vm::types::{AsMapping, Constructor, Representable};
    use rustpython_vm::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    };

    #[pyattr]
    const ZSTD_CLEVEL_DEFAULT: i32 = libzstd_rs_sys::ZSTD_CLEVEL_DEFAULT;

    // Compression parameter identifiers. Values match the `ZSTD_cParameter`
    // enum in libzstd, which is what the public `CompressionParameter` IntEnum
    // in `Lib/compression/zstd/__init__.py` derives its members from.
    // libzstd-rs-sys models the enum as a newtype whose inner value is
    // crate-private, so the ids — frozen C ABI values from zstd.h — are
    // spelled out as literals here.
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

    // Decompression parameter identifiers. libzstd only exposes one non-
    // experimental decompression parameter.
    #[pyattr]
    const ZSTD_d_windowLogMax: i32 = 100;

    // Strategy enum members ordered from fastest to strongest. These power
    // the `Strategy` IntEnum in `Lib/compression/zstd/__init__.py`.
    #[pyattr]
    const ZSTD_fast: i32 = libzstd_rs_sys::lib::zstd::ZSTD_fast as i32;
    #[pyattr]
    const ZSTD_dfast: i32 = libzstd_rs_sys::lib::zstd::ZSTD_dfast as i32;
    #[pyattr]
    const ZSTD_greedy: i32 = libzstd_rs_sys::lib::zstd::ZSTD_greedy as i32;
    #[pyattr]
    const ZSTD_lazy: i32 = libzstd_rs_sys::lib::zstd::ZSTD_lazy as i32;
    #[pyattr]
    const ZSTD_lazy2: i32 = libzstd_rs_sys::lib::zstd::ZSTD_lazy2 as i32;
    #[pyattr]
    const ZSTD_btlazy2: i32 = libzstd_rs_sys::lib::zstd::ZSTD_btlazy2 as i32;
    #[pyattr]
    const ZSTD_btopt: i32 = libzstd_rs_sys::lib::zstd::ZSTD_btopt as i32;
    #[pyattr]
    const ZSTD_btultra: i32 = libzstd_rs_sys::lib::zstd::ZSTD_btultra as i32;
    #[pyattr]
    const ZSTD_btultra2: i32 = libzstd_rs_sys::lib::zstd::ZSTD_btultra2 as i32;

    #[pyattr(once, name = "zstd_version")]
    fn zstd_version(_vm: &VirtualMachine) -> String {
        // SAFETY: `ZSTD_versionString` returns a pointer to libzstd's static,
        // NUL-terminated version string.
        unsafe { CStr::from_ptr(ZSTD_versionString()) }
            .to_string_lossy()
            .into_owned()
    }

    #[pyattr(once, name = "zstd_version_number")]
    fn zstd_version_number(_vm: &VirtualMachine) -> u32 {
        ZSTD_versionNumber()
    }

    #[pyattr(once, name = "ZSTD_DStreamOutSize")]
    fn zstd_dstream_out_size(_vm: &VirtualMachine) -> usize {
        ZSTD_DStreamOutSize()
    }

    // Dictionary load type markers. The `ZstdDict.as_*` properties wrap the
    // dictionary in a `(zdict, marker)` tuple so the compressor or decompressor
    // constructor knows which load mode to apply. Numbering matches CPython's
    // `Modules/_zstd/_zstdmodule.h::DictType`.
    const DICT_TYPE_DIGESTED: i32 = 0;
    const DICT_TYPE_UNDIGESTED: i32 = 1;
    const DICT_TYPE_PREFIX: i32 = 2;

    #[pyattr(once, name = "ZstdError")]
    fn zstd_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_zstd",
            "ZstdError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    fn new_zstd_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let msg: String = message.into();
        vm.new_exception_msg(vm.class("_zstd", "ZstdError"), msg.into())
    }

    /// Convert a libzstd error code (the `usize` returned by most ZSTD_*
    /// functions when `ZSTD_isError(code)` is non-zero) into a `ZstdError`
    /// carrying the human-readable message from `ZSTD_getErrorName`.
    fn catch_zstd_error(code: usize, vm: &VirtualMachine) -> PyBaseExceptionRef {
        // SAFETY: `ZSTD_getErrorName` returns a pointer to a static,
        // NUL-terminated error string from libzstd's error table.
        let name = unsafe { CStr::from_ptr(ZSTD_getErrorName(code)) };
        new_zstd_error(name.to_string_lossy(), vm)
    }

    /// Reject an options-dict `key` whose class is the parameter enum that is
    /// invalid for the caller's context (a `CompressionParameter` passed to a
    /// decompressor, or vice versa) with a `TypeError` naming the type.
    ///
    /// `forbidden` is that invalid enum class, resolved once by the caller from
    /// the types the pure-Python wrapper registers via [`set_parameter_types`];
    /// `None` (the wrapper never ran) skips the check, matching CPython's NULL
    /// module-state pointers. The comparison is by identity, mirroring
    /// CPython's `Py_TYPE(key) == ...` check.
    fn check_wrong_param_kind(
        key: &PyObjectRef,
        forbidden: Option<&PyObjectRef>,
        kind: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(forbidden) = forbidden else {
            return Ok(());
        };
        if key.class().is(forbidden) {
            // `key`'s class is `forbidden` here, so name it directly (the same
            // string CPython formats from `Py_TYPE(key)->tp_name`).
            let name = key.class().name();
            return Err(vm.new_type_error(format!(
                "{kind} options dictionary key must not be a {name} attribute"
            )));
        }
        Ok(())
    }

    /// Map a compression parameter id to its libzstd `ZSTD_cParameter`
    /// constant. Returns `None` for unknown ids so callers can surface a
    /// targeted `ValueError`. Done with an explicit match rather than
    /// `mem::transmute` so passing junk like `ZSTD_cParameter(42)` cannot
    /// be triggered from Python.
    fn c_param_enum(param: i32) -> Option<ZSTD_cParameter> {
        Some(match param {
            ZSTD_c_compressionLevel => ZSTD_cParameter::ZSTD_c_compressionLevel,
            ZSTD_c_windowLog => ZSTD_cParameter::ZSTD_c_windowLog,
            ZSTD_c_hashLog => ZSTD_cParameter::ZSTD_c_hashLog,
            ZSTD_c_chainLog => ZSTD_cParameter::ZSTD_c_chainLog,
            ZSTD_c_searchLog => ZSTD_cParameter::ZSTD_c_searchLog,
            ZSTD_c_minMatch => ZSTD_cParameter::ZSTD_c_minMatch,
            ZSTD_c_targetLength => ZSTD_cParameter::ZSTD_c_targetLength,
            ZSTD_c_strategy => ZSTD_cParameter::ZSTD_c_strategy,
            ZSTD_c_enableLongDistanceMatching => ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching,
            ZSTD_c_ldmHashLog => ZSTD_cParameter::ZSTD_c_ldmHashLog,
            ZSTD_c_ldmMinMatch => ZSTD_cParameter::ZSTD_c_ldmMinMatch,
            ZSTD_c_ldmBucketSizeLog => ZSTD_cParameter::ZSTD_c_ldmBucketSizeLog,
            ZSTD_c_ldmHashRateLog => ZSTD_cParameter::ZSTD_c_ldmHashRateLog,
            ZSTD_c_contentSizeFlag => ZSTD_cParameter::ZSTD_c_contentSizeFlag,
            ZSTD_c_checksumFlag => ZSTD_cParameter::ZSTD_c_checksumFlag,
            ZSTD_c_dictIDFlag => ZSTD_cParameter::ZSTD_c_dictIDFlag,
            ZSTD_c_nbWorkers => ZSTD_cParameter::ZSTD_c_nbWorkers,
            ZSTD_c_jobSize => ZSTD_cParameter::ZSTD_c_jobSize,
            ZSTD_c_overlapLog => ZSTD_cParameter::ZSTD_c_overlapLog,
            _ => return None,
        })
    }

    /// Map a decompression parameter id to its libzstd `ZSTD_dParameter`
    /// constant. See [`c_param_enum`] for rationale.
    fn d_param_enum(param: i32) -> Option<ZSTD_dParameter> {
        match param {
            ZSTD_d_windowLogMax => Some(ZSTD_dParameter::ZSTD_d_windowLogMax),
            _ => None,
        }
    }

    /// Validate a compression-parameter id and value pair, returning the
    /// parameter constant on success. Used by the compressor's `options=`
    /// constructor argument. The value is only validated here for the
    /// strategy parameter (which libzstd does not bounds-check on its own in
    /// a way CPython can rely on); everything else is guarded by the bounds
    /// pre-validation in `apply_options` and passed through to libzstd
    /// untouched, like CPython does.
    fn cparameter_from_int(
        param: i32,
        value: i32,
        vm: &VirtualMachine,
    ) -> PyResult<ZSTD_cParameter> {
        let p = c_param_enum(param).ok_or_else(|| {
            vm.new_value_error(format!(
                "invalid compression parameter 'unknown parameter (key {param})'"
            ))
        })?;
        if param == ZSTD_c_strategy && strategy_from_int(value).is_none() {
            return Err(new_zstd_error(
                format!("invalid strategy value: {value}"),
                vm,
            ));
        }
        Ok(p)
    }

    /// Validate a decompression-parameter id, returning the parameter
    /// constant on success. Used by the decompressor's `options=` constructor
    /// argument.
    fn dparameter_from_int(param: i32, vm: &VirtualMachine) -> PyResult<ZSTD_dParameter> {
        d_param_enum(param).ok_or_else(|| {
            vm.new_value_error(format!(
                "invalid decompression parameter 'unknown parameter (key {param})'"
            ))
        })
    }

    /// Build the CPython-compatible "<kind> parameter 'name' received an
    /// illegal value V; the valid range is [lo, hi]" `ValueError` for a
    /// parameter that was rejected by libzstd or that fell outside the
    /// documented bounds.
    fn param_value_error_for(
        param: i32,
        value: i32,
        is_compress: bool,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let kind = if is_compress {
            "compression"
        } else {
            "decompression"
        };
        let name = parameter_name(param, is_compress);
        match lookup_param_bounds(param, is_compress) {
            Some((lo, hi)) => vm.new_value_error(format!(
                "{kind} parameter '{name}' received an illegal value {value}; \
                 the valid range is [{lo}, {hi}]"
            )),
            None => vm.new_value_error(format!(
                "{kind} parameter '{name}' received an illegal value {value}"
            )),
        }
    }

    /// Return the valid `(lower, upper)` bounds for the libzstd compression
    /// level. Used when validating the `level=` argument upfront because
    /// libzstd silently clamps out-of-range values rather than surfacing
    /// them as errors.
    fn level_bounds() -> (i32, i32) {
        lookup_param_bounds(ZSTD_c_compressionLevel, true)
            .expect("compressionLevel always has valid bounds")
    }

    /// Look up parameter bounds for a known compression or decompression
    /// parameter id. Returns `None` if the id is not recognized (callers
    /// validate the id separately).
    fn lookup_param_bounds(param: i32, is_compress: bool) -> Option<(i32, i32)> {
        // The helpers above validated that `param` maps to a real parameter
        // constant before `ZSTD_*Param_getBounds` inspects it. The two
        // functions return distinct (but layout-identical) `ZSTD_bounds`
        // types, so destructure into a tuple right away.
        let (error, lo, hi) = if is_compress {
            let p = c_param_enum(param)?;
            let b = ZSTD_cParam_getBounds(p);
            (b.error, b.lowerBound, b.upperBound)
        } else {
            let p = d_param_enum(param)?;
            let b = ZSTD_dParam_getBounds(p);
            (b.error, b.lowerBound, b.upperBound)
        };
        if ZSTD_isError(error) != 0 {
            return None;
        }
        Some((lo, hi))
    }

    /// Map a parameter integer id back to the Python-visible enum member
    /// name. Used for error messages that pin-point the parameter that went
    /// out of range. Returns `"unknown"` for unrecognized ids.
    fn parameter_name(param: i32, is_compress: bool) -> &'static str {
        if is_compress {
            match param {
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
                _ => "unknown",
            }
        } else {
            match param {
                ZSTD_d_windowLogMax => "window_log_max",
                _ => "unknown",
            }
        }
    }

    /// Map a strategy integer (as exposed via the `Strategy` IntEnum) back
    /// to the underlying `ZSTD_strategy` C value. Done via an explicit
    /// match for the same reason as [`c_param_enum`]: an untrusted int
    /// might not correspond to any real strategy value.
    fn strategy_from_int(v: i32) -> Option<ZSTD_strategy> {
        use libzstd_rs_sys::lib::zstd as s;
        Some(match v {
            ZSTD_fast => s::ZSTD_fast,
            ZSTD_dfast => s::ZSTD_dfast,
            ZSTD_greedy => s::ZSTD_greedy,
            ZSTD_lazy => s::ZSTD_lazy,
            ZSTD_lazy2 => s::ZSTD_lazy2,
            ZSTD_btlazy2 => s::ZSTD_btlazy2,
            ZSTD_btopt => s::ZSTD_btopt,
            ZSTD_btultra => s::ZSTD_btultra,
            ZSTD_btultra2 => s::ZSTD_btultra2,
            _ => return None,
        })
    }

    /// Decode the `zstd_dict=` constructor argument. Accepts either a
    /// `ZstdDict` instance (treated as the default digested form) or a
    /// `(ZstdDict, marker)` tuple produced by one of `ZstdDict.as_*`.
    fn parse_zstd_dict_arg(
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<(PyRef<ZstdDict>, i32)> {
        // The first downcast clones `obj` because we fall through to the
        // tuple branch if it fails. The second downcast (the tuple one) is
        // the last use of `obj`, so we let it move directly.
        if let Ok(d) = obj.clone().downcast::<ZstdDict>() {
            return Ok((d, DICT_TYPE_DIGESTED));
        }
        if let Ok(tuple) = obj.downcast::<rustpython_vm::builtins::PyTuple>() {
            let items = tuple.as_slice();
            // Reject any tuple shape that is not (ZstdDict, int_marker) so the
            // test suite's bad-args coverage (`(zd, 1.0)`, `(zd,)`, `(zd, 3)`,
            // etc.) raises TypeError. Marker bounds match the three documented
            // `as_*` properties.
            if items.len() != 2 {
                return Err(vm.new_type_error("zstd_dict argument should be a ZstdDict object"));
            }
            let d = items[0]
                .clone()
                .downcast::<ZstdDict>()
                .map_err(|_| vm.new_type_error("zstd_dict argument should be a ZstdDict object"))?;
            // The marker must be a plain int (not float/etc); overflow on
            // `2**1000` propagates as OverflowError via `try_index`.
            let marker_obj = &items[1];
            let marker: i32 = marker_obj.try_to_value(vm).map_err(|e| {
                // Preserve OverflowError; everything else becomes TypeError so
                // callers see a consistent "should be a ZstdDict" message.
                if e.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                    e
                } else {
                    vm.new_type_error("zstd_dict argument should be a ZstdDict object")
                }
            })?;
            if !(DICT_TYPE_DIGESTED..=DICT_TYPE_PREFIX).contains(&marker) {
                return Err(vm.new_type_error("zstd_dict argument should be a ZstdDict object"));
            }
            return Ok((d, marker));
        }
        Err(vm.new_type_error("zstd_dict argument should be a ZstdDict object"))
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdDictArgs {
        #[pyarg(positional)]
        dict_content: ArgBytesLike,
        #[pyarg(named, default = false)]
        is_raw: bool,
    }

    #[pyattr]
    #[pyclass(name = "ZstdDict")]
    #[derive(PyPayload)]
    struct ZstdDict {
        dict_content: PyBytesRef,
        dict_id: u32,
    }

    impl core::fmt::Debug for ZstdDict {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(
                f,
                "<ZstdDict dict_id={} dict_size={}>",
                self.dict_id,
                self.dict_content.len()
            )
        }
    }

    impl Constructor for ZstdDict {
        type Args = ZstdDictArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let dict_content = args.dict_content.with_ref(|b| b.to_vec());
            // libzstd's `ZSTD_getDictID_fromDict` returns 0 either when
            // the content is too small to contain a valid header or when it
            // does not carry the dictionary magic. Both are runtime errors
            // when `is_raw=False`, matching CPython's behavior of raising
            // `ValueError` on a non-conformant dictionary.
            // SAFETY: `dict_content` is a live slice for the duration of the
            // call.
            let parsed_id = unsafe {
                ZSTD_getDictID_fromDict(dict_content.as_ptr().cast(), dict_content.len())
            };
            if !args.is_raw && parsed_id == 0 {
                return Err(vm.new_value_error(
                    "ZSTD_DICT_MAGIC_NUMBER not found, dict_content cannot be a 'raw content' \
                     dictionary. To create a raw content dictionary, pass is_raw=True.",
                ));
            }
            // Raw dictionaries still get a non-zero `dict_id` whenever their
            // contents happen to look like a valid dict (this is the
            // documented behavior tested in `test_is_raw`).
            Ok(Self {
                dict_content: vm.ctx.new_bytes(dict_content),
                dict_id: parsed_id,
            })
        }
    }

    impl Representable for ZstdDict {
        #[inline]
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!(
                "<ZstdDict dict_id={} dict_size={}>",
                zelf.dict_id,
                zelf.dict_content.len()
            ))
        }
    }

    impl rustpython_vm::types::AsMapping for ZstdDict {
        fn as_mapping() -> &'static rustpython_vm::protocol::PyMappingMethods {
            static AS_MAPPING: rustpython_vm::protocol::PyMappingMethods =
                rustpython_vm::protocol::PyMappingMethods {
                    length: Some(|mapping, _vm| {
                        Ok(ZstdDict::mapping_downcast(mapping).dict_content.len())
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
        fn dict_content(&self) -> PyBytesRef {
            self.dict_content.clone()
        }

        #[pygetset]
        fn dict_id(&self) -> u32 {
            self.dict_id
        }

        #[pygetset]
        fn as_digested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(DICT_TYPE_DIGESTED).into()])
        }

        #[pygetset]
        fn as_undigested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx.new_tuple(vec![
                zelf.into(),
                vm.ctx.new_int(DICT_TYPE_UNDIGESTED).into(),
            ])
        }

        #[pygetset]
        fn as_prefix(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(DICT_TYPE_PREFIX).into()])
        }
    }

    // The three flush modes for `ZstdCompressor.compress()`, mirrored as
    // class attributes via `extend_class` below. Values are positional and
    // chosen to match what CPython exposes.
    const COMP_MODE_CONTINUE: i32 = 0;
    const COMP_MODE_FLUSH_BLOCK: i32 = 1;
    const COMP_MODE_FLUSH_FRAME: i32 = 2;

    /// Owning wrapper around a raw libzstd compression context. Frees the
    /// context with `ZSTD_freeCCtx` on drop.
    struct CCtx(*mut ZSTD_CCtx);

    // SAFETY: the wrapped context is only ever accessed while holding the
    // owning pyclass's `PyMutex` (`CompressorState`), so no concurrent
    // access can occur.
    unsafe impl Send for CCtx {}

    impl CCtx {
        fn create() -> Self {
            // SAFETY: `ZSTD_createCCtx` has no preconditions; it returns NULL
            // only on allocation failure.
            let ptr = unsafe { ZSTD_createCCtx() };
            assert!(!ptr.is_null(), "ZSTD_createCCtx failed");
            Self(ptr)
        }

        /// Forward a `(parameter, value)` pair to libzstd, passing the value
        /// straight through like CPython does. The error code is returned on
        /// failure so the caller can pick the right error mapping.
        fn set_parameter(&mut self, param: ZSTD_cParameter, value: c_int) -> Result<(), usize> {
            // SAFETY: `self.0` is a live context owned by this wrapper.
            let code = unsafe { ZSTD_CCtx_setParameter(self.0, param, value) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }

        fn set_pledged_src_size(&mut self, pledged: Option<u64>) -> Result<(), usize> {
            // libzstd represents "unknown" as `ZSTD_CONTENTSIZE_UNKNOWN`.
            let pledged = pledged.map_or(ZSTD_CONTENTSIZE_UNKNOWN, |v| v);
            // SAFETY: `self.0` is a live context owned by this wrapper.
            let code = unsafe { ZSTD_CCtx_setPledgedSrcSize(self.0, pledged) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
    }

    impl Drop for CCtx {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createCCtx` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeCCtx(self.0) };
        }
    }

    /// Owning wrapper around a raw libzstd decompression context. Frees the
    /// context with `ZSTD_freeDCtx` on drop.
    struct DCtx(*mut ZSTD_DCtx);

    // SAFETY: same reasoning as `CCtx`: access is serialized by the owning
    // pyclass's `PyMutex` (`DecompressorState`).
    unsafe impl Send for DCtx {}

    impl DCtx {
        fn create() -> Self {
            // SAFETY: `ZSTD_createDCtx` has no preconditions; it returns NULL
            // only on allocation failure.
            let ptr = unsafe { ZSTD_createDCtx() };
            assert!(!ptr.is_null(), "ZSTD_createDCtx failed");
            Self(ptr)
        }

        /// See [`CCtx::set_parameter`].
        fn set_parameter(&mut self, param: ZSTD_dParameter, value: c_int) -> Result<(), usize> {
            // SAFETY: `self.0` is a live context owned by this wrapper.
            let code = unsafe { ZSTD_DCtx_setParameter(self.0, param, value) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
    }

    impl Drop for DCtx {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createDCtx` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeDCtx(self.0) };
        }
    }

    /// Owning wrapper around a digested compression dictionary. Frees the
    /// dictionary with `ZSTD_freeCDict` on drop.
    struct CDict(*mut ZSTD_CDict);

    // SAFETY: the dictionary is only referenced by the `CCtx` stored
    // alongside it in `CompressorState`, which the pyclass's `PyMutex`
    // serializes access to.
    unsafe impl Send for CDict {}

    impl CDict {
        /// Build a digested dictionary; libzstd copies the bytes into its own
        /// storage. Returns `None` when libzstd rejects the content (NULL),
        /// e.g. for a corrupted dictionary.
        fn try_create(bytes: &[u8], level: c_int) -> Option<Self> {
            // SAFETY: `bytes` is a live slice for the duration of the call,
            // and `ZSTD_createCDict` copies what it needs.
            let ptr = unsafe { ZSTD_createCDict(bytes.as_ptr().cast(), bytes.len(), level) };
            if ptr.is_null() { None } else { Some(Self(ptr)) }
        }
    }

    impl Drop for CDict {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createCDict` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeCDict(self.0) };
        }
    }

    /// Owning wrapper around a digested decompression dictionary. Frees the
    /// dictionary with `ZSTD_freeDDict` on drop.
    struct DDict(*mut ZSTD_DDict);

    // SAFETY: same reasoning as `CDict` (guarded by the decompressor's
    // `PyMutex` via `DecompressorState`).
    unsafe impl Send for DDict {}

    impl DDict {
        /// See [`CDict::try_create`].
        fn try_create(bytes: &[u8]) -> Option<Self> {
            // SAFETY: `bytes` is a live slice for the duration of the call,
            // and `ZSTD_createDDict` copies what it needs.
            let ptr = unsafe { ZSTD_createDDict(bytes.as_ptr().cast(), bytes.len()) };
            if ptr.is_null() { None } else { Some(Self(ptr)) }
        }
    }

    impl Drop for DDict {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createDDict` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeDDict(self.0) };
        }
    }

    /// Internal state of a `ZstdCompressor`. Holds the libzstd context, the
    /// last mode used (for `last_mode` and `set_pledged_input_size`
    /// validation), and the dictionary handles that the context may reference
    /// internally. Field order matters here: Rust drops in declaration order,
    /// so `cctx` is freed first; the held `CDict` (if any) and the source
    /// `PyRef<ZstdDict>` go away afterwards, which is the safe order for
    /// teardown.
    struct CompressorState {
        cctx: CCtx,
        /// Cached digested dictionary. The CCtx references this via
        /// `ZSTD_CCtx_refCDict`, so it must outlive the CCtx (handled by
        /// Rust's field drop order: `cctx` drops first).
        _cdict: Option<CDict>,
        /// Keeps the ZstdDict's bytes alive for `ZSTD_CCtx_refPrefix` mode.
        _dict: Option<PyRef<ZstdDict>>,
        last_mode: i32,
    }

    #[pyattr]
    #[pyclass(name = "ZstdCompressor")]
    #[derive(PyPayload)]
    struct ZstdCompressor {
        state: PyMutex<CompressorState>,
    }

    impl core::fmt::Debug for ZstdCompressor {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "_zstd.ZstdCompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdCompressorArgs {
        #[pyarg(any, optional)]
        level: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional)]
        options: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional)]
        zstd_dict: OptionalOption<PyObjectRef>,
    }

    /// Translate the public `mode` int to the libzstd `ZSTD_EndDirective`
    /// the streaming API takes.
    fn end_directive_from_mode(mode: i32, vm: &VirtualMachine) -> PyResult<ZSTD_EndDirective> {
        match mode {
            COMP_MODE_CONTINUE => Ok(ZSTD_e_continue),
            COMP_MODE_FLUSH_BLOCK => Ok(ZSTD_e_flush),
            COMP_MODE_FLUSH_FRAME => Ok(ZSTD_e_end),
            _ => Err(vm.new_value_error(format!(
                "mode argument wrong value, it should be one of \
                 ZstdCompressor.CONTINUE ({COMP_MODE_CONTINUE}), \
                 ZstdCompressor.FLUSH_BLOCK ({COMP_MODE_FLUSH_BLOCK}), or \
                 ZstdCompressor.FLUSH_FRAME ({COMP_MODE_FLUSH_FRAME})"
            ))),
        }
    }

    impl Constructor for ZstdCompressor {
        type Args = ZstdCompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let level_opt = args.level.flatten();
            let options_opt = args.options.flatten();
            let dict_opt = args.zstd_dict.flatten();

            if level_opt.is_some() && options_opt.is_some() {
                return Err(vm.new_type_error("Only one of level or options should be used."));
            }

            let mut cctx = CCtx::create();

            if let Some(level_obj) = level_opt {
                let level = parse_compression_level(&level_obj, vm)?;
                cctx.set_parameter(ZSTD_cParameter::ZSTD_c_compressionLevel, level)
                    .map_err(|_| param_value_error_for(ZSTD_c_compressionLevel, level, true, vm))?;
            }

            if let Some(options_obj) = options_opt {
                apply_options(&mut cctx, options_obj, true, vm)?;
            }

            let state = build_compressor_state(cctx, dict_opt, COMP_MODE_FLUSH_FRAME, vm)?;
            Ok(Self {
                state: PyMutex::new(state),
            })
        }
    }

    /// Parse and validate a compression `level` argument. libzstd silently
    /// clamps out-of-range levels rather than erroring, but CPython surfaces
    /// them as `ValueError`, and bigints become `ValueError` (not
    /// `OverflowError`) for the same reason. This helper centralizes both
    /// conversions so the constructor stays linear.
    fn parse_compression_level(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        let (lo, hi) = level_bounds();
        let level: i32 = obj.try_to_value(vm).map_err(|e| {
            if e.fast_isinstance(vm.ctx.exceptions.overflow_error) {
                vm.new_value_error(format!(
                    "illegal compression level; the valid range is [{lo}, {hi}]"
                ))
            } else {
                e
            }
        })?;
        if level < lo || level > hi {
            return Err(vm.new_value_error(format!(
                "illegal compression level {level}; the valid range is [{lo}, {hi}]"
            )));
        }
        Ok(level)
    }

    /// Drain an `options=` dict onto either a `CCtx` or a `DCtx`. Validates
    /// each key/value pair (rejects wrong enum kind, rejects floats, rejects
    /// out-of-range values) so the constructor's flow stays a single line.
    fn apply_options(
        ctx: &mut dyn ParamSetter,
        options_obj: PyObjectRef,
        is_compress: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = options_obj
            .downcast::<PyDict>()
            .map_err(|_| vm.new_type_error("options must be a dict"))?;
        // Resolve the parameter enum class that is invalid for this direction
        // (registered by the pure-Python wrapper via set_parameter_types) once,
        // so a key from the wrong family yields a clear TypeError naming the
        // type. `None` when the wrapper never ran (e.g. `_zstd` used directly).
        let (wrong_kind_attr, kind) = if is_compress {
            ("_decompression_parameter_type", "compression")
        } else {
            ("_compression_parameter_type", "decompression")
        };
        let wrong_kind = vm.get_attribute_opt(vm.import("_zstd", 0)?, wrong_kind_attr)?;
        for (k, v) in dict {
            // Reject a key from the wrong parameter family before any numeric
            // coercion, so the error names the type rather than giving a
            // generic out-of-range message.
            check_wrong_param_kind(&k, wrong_kind.as_ref(), kind, vm)?;
            let key_int: i32 = k.try_to_value(vm)?;
            let val_int: i32 = v.try_to_value(vm)?;
            // libzstd silently clamps out-of-range values for some
            // parameters (notably compression_level) rather than rejecting
            // them, so validate against the documented bounds upfront.
            if let Some((lo, hi)) = lookup_param_bounds(key_int, is_compress)
                && (val_int < lo || val_int > hi)
            {
                return Err(param_value_error_for(key_int, val_int, is_compress, vm));
            }
            ctx.apply(key_int, val_int, vm)?;
        }
        Ok(())
    }

    /// Trait wrapper over `CCtx::set_parameter` and `DCtx::set_parameter` so
    /// `apply_options` can drive either context without duplicated code.
    /// `apply` validates the (id, value) pair via `cparameter_from_int` /
    /// `dparameter_from_int`, then forwards to libzstd.
    trait ParamSetter {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()>;
    }

    impl ParamSetter for CCtx {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            let p = cparameter_from_int(param, value, vm)?;
            self.set_parameter(p, value)
                .map_err(|_| param_value_error_for(param, value, true, vm))?;
            Ok(())
        }
    }

    impl ParamSetter for DCtx {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            let p = dparameter_from_int(param, vm)?;
            self.set_parameter(p, value)
                .map_err(|_| param_value_error_for(param, value, false, vm))?;
            Ok(())
        }
    }

    /// Trait that captures the only differences between how the compressor
    /// and decompressor consume a dictionary: the name of the type that
    /// appears in error messages, the eager-validation constructor for the
    /// digested variant, and the three ways of attaching it to the context.
    /// The `Err` payload of each method is a libzstd error code.
    trait DictLoader {
        type Digested;
        const KIND_NAME: &'static str;
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested>;
        fn ref_digested(&mut self, dict: &Self::Digested) -> Result<(), usize>;
        fn load_undigested(&mut self, bytes: &[u8]) -> Result<(), usize>;
        fn ref_prefix(&mut self, bytes: &[u8]) -> Result<(), usize>;
    }

    impl DictLoader for CCtx {
        type Digested = CDict;
        const KIND_NAME: &'static str = "ZSTD_CDict";
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested> {
            CDict::try_create(bytes, libzstd_rs_sys::ZSTD_CLEVEL_DEFAULT)
        }
        fn ref_digested(&mut self, dict: &Self::Digested) -> Result<(), usize> {
            // SAFETY: both handles are live; `load_dict`'s safety contract
            // keeps `dict` alive at least as long as `self`.
            let code = unsafe { ZSTD_CCtx_refCDict(self.0, dict.0.cast_const()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
        fn load_undigested(&mut self, bytes: &[u8]) -> Result<(), usize> {
            // SAFETY: `bytes` is a live slice for the duration of the call;
            // its contents are copied into the context.
            let code =
                unsafe { ZSTD_CCtx_loadDictionary(self.0, bytes.as_ptr().cast(), bytes.len()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
        fn ref_prefix(&mut self, bytes: &[u8]) -> Result<(), usize> {
            // SAFETY: `bytes` is a live slice for the duration of the call.
            // libzstd keeps a raw pointer to the bytes afterwards;
            // `load_dict`'s safety contract keeps them alive at least as
            // long as `self`.
            let code = unsafe { ZSTD_CCtx_refPrefix(self.0, bytes.as_ptr().cast(), bytes.len()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
    }

    impl DictLoader for DCtx {
        type Digested = DDict;
        const KIND_NAME: &'static str = "ZSTD_DDict";
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested> {
            DDict::try_create(bytes)
        }
        fn ref_digested(&mut self, dict: &Self::Digested) -> Result<(), usize> {
            // SAFETY: both handles are live; `load_dict`'s safety contract
            // keeps `dict` alive at least as long as `self`.
            let code = unsafe { ZSTD_DCtx_refDDict(self.0, dict.0.cast_const()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
        fn load_undigested(&mut self, bytes: &[u8]) -> Result<(), usize> {
            // SAFETY: `bytes` is a live slice for the duration of the call;
            // its contents are copied into the context.
            let code =
                unsafe { ZSTD_DCtx_loadDictionary(self.0, bytes.as_ptr().cast(), bytes.len()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
        fn ref_prefix(&mut self, bytes: &[u8]) -> Result<(), usize> {
            // SAFETY: `bytes` is a live slice for the duration of the call.
            // libzstd keeps a raw pointer to the bytes afterwards;
            // `load_dict`'s safety contract keeps them alive at least as
            // long as `self`.
            let code = unsafe { ZSTD_DCtx_refPrefix(self.0, bytes.as_ptr().cast(), bytes.len()) };
            if ZSTD_isError(code) != 0 {
                Err(code)
            } else {
                Ok(())
            }
        }
    }

    /// Return value of `load_dict`: the digested `CDict`/`DDict` (if any)
    /// and the `PyRef<ZstdDict>` we hold to keep the dictionary bytes alive
    /// while `ref_prefix` may point into them.
    type DictLoadResult<D> = PyResult<(Option<D>, Option<PyRef<ZstdDict>>)>;

    /// Common path for attaching a dictionary to either context type. Returns
    /// the digested `CDict`/`DDict` (if the caller used digested mode) plus
    /// the `PyRef<ZstdDict>` whose bytes libzstd's `ref_prefix` may point into.
    ///
    /// # Safety
    ///
    /// libzstd stores the dictionary as a raw pointer that bypasses Rust's
    /// lifetime tracking. The caller must keep both returned values alive at
    /// least as long as `ctx`:
    ///
    /// - In `digested` mode, `ctx` holds a raw pointer to the returned
    ///   `L::Digested`; dropping it before `ctx` is use-after-free.
    /// - In `prefix` mode, `ctx` holds a raw pointer into the bytes owned by
    ///   the returned `PyRef<ZstdDict>`; dropping the `PyRef` before `ctx`
    ///   is use-after-free.
    ///
    /// In `undigested` mode the bytes are copied into `ctx`, so neither
    /// return value carries a safety obligation — but the caller cannot tell
    /// the modes apart, so it must keep both alive regardless.
    unsafe fn load_dict<L: DictLoader>(
        ctx: &mut L,
        dict_obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> DictLoadResult<L::Digested> {
        let Some(dict_obj) = dict_obj else {
            return Ok((None, None));
        };
        let (zdict, marker) = parse_zstd_dict_arg(dict_obj, vm)?;
        let bad_dict_err = || -> PyBaseExceptionRef {
            new_zstd_error(
                format!(
                    "Failed to load the {} instance from corrupted Zstandard dictionary content.",
                    L::KIND_NAME
                ),
                vm,
            )
        };
        let dict_bytes = zdict.dict_content.as_bytes();
        let mut digested = None;
        match marker {
            DICT_TYPE_PREFIX => {
                ctx.ref_prefix(dict_bytes).map_err(|_| bad_dict_err())?;
            }
            DICT_TYPE_DIGESTED => {
                // Build the digested dict eagerly so a corrupted dictionary
                // surfaces as a `ZstdError` at construction time, not when
                // the first compress/decompress call runs.
                let d = L::try_create_digested(dict_bytes).ok_or_else(bad_dict_err)?;
                ctx.ref_digested(&d).map_err(|_| bad_dict_err())?;
                digested = Some(d);
            }
            _ => {
                // Undigested: copy the bytes into the context. Validation
                // happens lazily at the first stream call in this mode.
                ctx.load_undigested(dict_bytes)
                    .map_err(|_| bad_dict_err())?;
            }
        }
        Ok((digested, Some(zdict)))
    }

    /// Build a fully-initialized `CompressorState` from a freshly-created
    /// `CCtx` and an optional dictionary argument. This is the safe interface
    /// that `unsafe fn load_dict` was waiting for: by assembling the struct
    /// here, both invariants `load_dict` documents become structural and a
    /// safe-Rust caller cannot split the pieces apart.
    fn build_compressor_state(
        mut cctx: CCtx,
        dict_obj: Option<PyObjectRef>,
        last_mode: i32,
        vm: &VirtualMachine,
    ) -> PyResult<CompressorState> {
        // SAFETY: `load_dict` requires its two return values to outlive `ctx`.
        // We satisfy that by moving `cctx` and both return values into
        // `CompressorState` in one expression — Rust drops the struct's
        // fields in declaration order, so on teardown `cctx` is dropped
        // first, releasing its raw pointers before `_cdict` (digested mode)
        // and `_dict` (prefix mode) are freed. `CompressorState` is private
        // to this module and is never destructured, so no safe caller can
        // reorder the drops.
        let (cdict, dict) = unsafe { load_dict::<CCtx>(&mut cctx, dict_obj, vm) }?;
        Ok(CompressorState {
            cctx,
            _cdict: cdict,
            _dict: dict,
            last_mode,
        })
    }

    /// Build a fully-initialized `DecompressorState`. See
    /// [`build_compressor_state`] for the safety reasoning;
    /// `DecompressorState`'s field order plays the same role here.
    fn build_decompressor_state(
        mut dctx: DCtx,
        dict_obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<DecompressorState> {
        // SAFETY: see [`build_compressor_state`].
        let (ddict, dict) = unsafe { load_dict::<DCtx>(&mut dctx, dict_obj, vm) }?;
        Ok(DecompressorState {
            dctx,
            _ddict: ddict,
            _dict: dict,
            eof: false,
            needs_input: true,
            unused_data: vm.ctx.empty_bytes.clone(),
            input_buffer: Vec::new(),
        })
    }

    /// Drive `ZSTD_compressStream2` until the input is fully consumed and,
    /// for flush/end directives, the internal buffers report zero remaining
    /// bytes. Grows the output `Vec` by `ZSTD_CStreamOutSize` chunks as
    /// needed.
    fn do_compress(
        state: &mut CompressorState,
        data: &[u8],
        end_op: ZSTD_EndDirective,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        // Release the GIL for the duration of the compression loop. Safety:
        // `data` is an immutable borrow of a local `Vec` in the caller,
        // `state` is held under the compressor's `PyMutex` (no other Python
        // thread can touch it), the `_dict` bytes referenced by libzstd are
        // an immutable `Vec` inside a `PyRef<ZstdDict>` (other readers fine),
        // and the output `Vec` is local to this function. No Python object
        // access happens inside the closure — error codes are surfaced as
        // `usize` and converted into exceptions after re-attaching.
        let is_end = end_op != ZSTD_e_continue;
        let chunk_size = ZSTD_CStreamOutSize().max(1);
        let result: Result<Vec<u8>, usize> = vm.allow_threads(|| {
            let mut output: Vec<u8> = Vec::new();
            let mut input = ZSTD_inBuffer {
                src: data.as_ptr().cast(),
                size: data.len(),
                pos: 0,
            };
            loop {
                let prev_len = output.len();
                output.reserve(chunk_size);
                let remaining = {
                    let spare = output.spare_capacity_mut();
                    let mut out_buf = ZSTD_outBuffer {
                        dst: spare.as_mut_ptr().cast(),
                        size: spare.len(),
                        pos: 0,
                    };
                    // SAFETY: `state.cctx` is a live context; `out_buf`
                    // covers the Vec's spare capacity, which is valid for
                    // `spare.len()` bytes; `input` points to `data`, which
                    // outlives the call.
                    let code = unsafe {
                        ZSTD_compressStream2(state.cctx.0, &mut out_buf, &mut input, end_op)
                    };
                    if ZSTD_isError(code) != 0 {
                        return Err(code);
                    }
                    // SAFETY: libzstd wrote `out_buf.pos` bytes
                    // (`out_buf.pos <= spare.len()`) into the spare capacity.
                    unsafe { output.set_len(prev_len + out_buf.pos) };
                    code
                };
                let consumed_all = input.pos == input.size;
                // Stop when input is fully consumed and, for flush/end
                // directives, libzstd reports that all internal buffers have
                // been drained (remaining == 0). Otherwise loop; the next
                // `reserve` will grow the output if we hit the previous cap.
                if consumed_all && (!is_end || remaining == 0) {
                    break Ok(output);
                }
            }
        });
        result.map_err(|c| catch_zstd_error(c, vm))
    }

    #[pyclass(with(Constructor))]
    impl ZstdCompressor {
        #[pymethod]
        fn compress(&self, args: CompressMethodArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mode = args.mode.unwrap_or(COMP_MODE_CONTINUE);
            let end_op = end_directive_from_mode(mode, vm)?;
            let data = args.data.with_ref(|b| b.to_vec());
            let mut state = self.state.lock();
            let out = do_compress(&mut state, &data, end_op, vm)?;
            state.last_mode = mode;
            Ok(out)
        }

        #[pymethod]
        fn flush(&self, args: FlushMethodArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mode = args.mode.unwrap_or(COMP_MODE_FLUSH_FRAME);
            if mode != COMP_MODE_FLUSH_BLOCK && mode != COMP_MODE_FLUSH_FRAME {
                return Err(vm.new_value_error(format!(
                    "mode argument wrong value, it should be \
                     ZstdCompressor.FLUSH_FRAME ({COMP_MODE_FLUSH_FRAME}) or \
                     ZstdCompressor.FLUSH_BLOCK ({COMP_MODE_FLUSH_BLOCK})"
                )));
            }
            let end_op = end_directive_from_mode(mode, vm)?;
            let mut state = self.state.lock();
            let out = do_compress(&mut state, &[], end_op, vm)?;
            state.last_mode = mode;
            Ok(out)
        }

        #[pymethod]
        fn set_pledged_input_size(&self, size: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // Parse the argument *before* taking the lock: `try_index` can run a
            // Python `__index__`, and doing that while holding `self.state` would
            // let a re-entrant call into this compressor deadlock. CPython
            // likewise converts the argument before touching the compressor.
            //
            // Python passes `None` to mean "unknown"; libzstd represents that
            // internally as `ZSTD_CONTENTSIZE_UNKNOWN` (`u64::MAX`), which
            // `set_pledged_src_size` maps `None` to. libzstd also reserves
            // `ZSTD_CONTENTSIZE_ERROR` (`u64::MAX - 1`), so a concrete size must
            // be strictly less than that; reject anything else up front so
            // callers see the documented `ValueError`, not a libzstd-level error.
            let pledged: Option<u64> = if vm.is_none(&size) {
                None
            } else {
                const LIMIT: u64 = u64::MAX - 1;
                let err = || {
                    vm.new_value_error(format!(
                        "size argument should be a positive int less than {LIMIT}"
                    ))
                };
                // `try_to_primitive` fails (OverflowError) for negatives and for
                // values above `u64::MAX`; the explicit check covers the rest of
                // the reserved range.
                let v: u64 = size
                    .try_index(vm)?
                    .try_to_primitive(vm)
                    .map_err(|_| err())?;
                if v >= LIMIT {
                    return Err(err());
                }
                Some(v)
            };
            let mut state = self.state.lock();
            if state.last_mode != COMP_MODE_FLUSH_FRAME {
                return Err(vm.new_value_error(
                    "set_pledged_input_size() method must be called when last_mode == FLUSH_FRAME",
                ));
            }
            state
                .cctx
                .set_pledged_src_size(pledged)
                .map_err(|c| catch_zstd_error(c, vm))?;
            Ok(())
        }

        #[pygetset]
        fn last_mode(&self) -> i32 {
            self.state.lock().last_mode
        }

        // Install class-level constants `CONTINUE`, `FLUSH_BLOCK`, and
        // `FLUSH_FRAME` so callers can reference them as
        // `ZstdCompressor.FLUSH_FRAME` (as the Python `ZstdFile` wrapper
        // does).
        #[extend_class]
        fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
            class.set_attr(
                ctx.intern_str("CONTINUE"),
                ctx.new_int(COMP_MODE_CONTINUE).into(),
            );
            class.set_attr(
                ctx.intern_str("FLUSH_BLOCK"),
                ctx.new_int(COMP_MODE_FLUSH_BLOCK).into(),
            );
            class.set_attr(
                ctx.intern_str("FLUSH_FRAME"),
                ctx.new_int(COMP_MODE_FLUSH_FRAME).into(),
            );
        }
    }

    #[derive(FromArgs)]
    pub(super) struct CompressMethodArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, optional)]
        mode: Option<i32>,
    }

    #[derive(FromArgs)]
    pub(super) struct FlushMethodArgs {
        #[pyarg(any, optional)]
        mode: Option<i32>,
    }

    /// Internal state of a `ZstdDecompressor`. The CPython decompressor is
    /// single-frame: once we hit end-of-frame, additional bytes go into
    /// `unused_data` and further `decompress` calls raise `EOFError`. Field
    /// drop order matters here for the same reason as in `CompressorState`:
    /// the `dctx` is freed first and must give up its internal pointers
    /// before any referenced `DDict`/`PyRef<ZstdDict>` is dropped.
    struct DecompressorState {
        dctx: DCtx,
        /// Cached decompression dictionary referenced by the DCtx via
        /// `ZSTD_DCtx_refDDict`.
        _ddict: Option<DDict>,
        _dict: Option<PyRef<ZstdDict>>,
        eof: bool,
        needs_input: bool,
        /// Bytes that arrived after the end of the first frame.
        unused_data: PyBytesRef,
        /// Input bytes buffered because the previous `decompress` call ran
        /// into its `max_length` cap before consuming them all.
        input_buffer: Vec<u8>,
    }

    #[pyattr]
    #[pyclass(name = "ZstdDecompressor")]
    #[derive(PyPayload)]
    struct ZstdDecompressor {
        state: PyMutex<DecompressorState>,
    }

    impl core::fmt::Debug for ZstdDecompressor {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "_zstd.ZstdDecompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdDecompressorArgs {
        #[pyarg(any, optional)]
        zstd_dict: OptionalOption<PyObjectRef>,
        #[pyarg(any, optional)]
        options: OptionalOption<PyObjectRef>,
    }

    impl Constructor for ZstdDecompressor {
        type Args = ZstdDecompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let dict_opt = args.zstd_dict.flatten();
            let options_opt = args.options.flatten();

            let mut dctx = DCtx::create();

            if let Some(options_obj) = options_opt {
                apply_options(&mut dctx, options_obj, false, vm)?;
            }

            let state = build_decompressor_state(dctx, dict_opt, vm)?;
            Ok(Self {
                state: PyMutex::new(state),
            })
        }
    }

    #[derive(FromArgs)]
    pub(super) struct DecompressMethodArgs {
        #[pyarg(positional)]
        data: ArgBytesLike,
        #[pyarg(any, default = -1)]
        max_length: isize,
    }

    /// Drive `decompress_stream` until either the frame ends, the input is
    /// exhausted with no more output coming, or `max_length` bytes have been
    /// produced. Sets the various `state` flags to reflect the new situation.
    ///
    /// Loop control: we always keep going while either the input still has
    /// bytes to feed OR the previous call filled the output buffer (which
    /// indicates libzstd had more to emit but ran out of room). We only stop
    /// short of a frame boundary when the input is exhausted AND libzstd had
    /// room left in the output buffer, which means it is genuinely waiting
    /// for more compressed bytes.
    fn do_decompress(
        state: &mut DecompressorState,
        new_data: &[u8],
        max_length: Option<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        // Combine any buffered leftover input with the new data so the
        // decompressor sees one contiguous stream. `Cow` avoids the
        // allocation when there is no leftover.
        let work_data: alloc::borrow::Cow<'_, [u8]> = if state.input_buffer.is_empty() {
            alloc::borrow::Cow::Borrowed(new_data)
        } else {
            let mut combined = Vec::with_capacity(state.input_buffer.len() + new_data.len());
            combined.extend_from_slice(&state.input_buffer);
            combined.extend_from_slice(new_data);
            alloc::borrow::Cow::Owned(combined)
        };

        let chunk_size = ZSTD_DStreamOutSize().max(1);
        // Release the GIL for the streaming loop. Safety: see `do_compress`;
        // the closure captures only Rust-owned buffers and `&mut state` (held
        // under the decompressor's `PyMutex`), and surfaces error codes as
        // `usize` so we can build the exception after re-attaching.
        let loop_result: Result<(Vec<u8>, bool, usize), usize> = vm.allow_threads(|| {
            let mut input = ZSTD_inBuffer {
                src: work_data.as_ptr().cast(),
                size: work_data.len(),
                pos: 0,
            };
            let mut output: Vec<u8> = Vec::new();
            // Reusable scratch buffer for each decompress_stream call. We need
            // an exact-size output buffer because `Vec::reserve` may
            // over-allocate; reporting the full Vec capacity to libzstd would
            // let it write past `max_length`.
            let mut scratch: Vec<u8> = vec![0u8; chunk_size];
            let mut hit_max = false;
            let mut iteration = 0usize;

            let outcome = loop {
                iteration += 1;
                // Honor `max_length`: stop growing the output buffer once
                // we have produced enough. When the cap is zero, hand
                // libzstd a zero-size buffer instead: it then consumes
                // input without emitting, so zero-output frames (skippable
                // frame, empty content frame) still complete while real
                // output stays inside libzstd until a later call has room.
                // CPython's decompressor uses the same zero-size mechanism.
                let grow = match max_length {
                    Some(maxl) if output.len() >= maxl && iteration > 1 => {
                        hit_max = true;
                        break Ok(());
                    }
                    Some(maxl) if output.len() >= maxl => 0,
                    Some(maxl) => (maxl - output.len()).min(chunk_size),
                    None => chunk_size,
                };
                let code;
                let written;
                {
                    let slot = &mut scratch[..grow];
                    let mut out_buf = ZSTD_outBuffer {
                        dst: slot.as_mut_ptr().cast(),
                        size: slot.len(),
                        pos: 0,
                    };
                    // SAFETY: `state.dctx` is a live context; `out_buf`
                    // covers `slot`; `input` points into `work_data`, which
                    // outlives the call.
                    code = unsafe { ZSTD_decompressStream(state.dctx.0, &mut out_buf, &mut input) };
                    written = out_buf.pos;
                }
                output.extend_from_slice(&scratch[..written]);
                if ZSTD_isError(code) != 0 {
                    break Err(code);
                }
                if code == 0 {
                    // Frame fully decompressed; the decompressor is at EOF.
                    state.eof = true;
                    break Ok(());
                }
                // Only meaningful with a non-zero buffer: libzstd had more
                // to emit but ran out of room. A zero-size call (the
                // max_length == 0 probe) is not "full".
                let output_was_full = grow > 0 && written == grow;
                let input_consumed = input.pos == input.size;

                if let Some(maxl) = max_length
                    && output.len() >= maxl
                    && iteration > 1
                {
                    hit_max = true;
                    break Ok(());
                }

                // Input is gone and libzstd had room to write but did
                // not, which means the frame is incomplete and the
                // caller has to supply more input.
                if input_consumed && !output_was_full {
                    break Ok(());
                }
            };
            outcome.map(|()| (output, hit_max, input.pos))
        });

        let (output, hit_max, consumed) = loop_result.map_err(|c| catch_zstd_error(c, vm))?;

        let remaining = &work_data[consumed..];

        if state.eof {
            if !remaining.is_empty() {
                state.unused_data = vm.ctx.new_bytes(remaining.to_vec());
            }
            state.input_buffer.clear();
            state.needs_input = false;
        } else if hit_max {
            // Output cap reached with input still pending. Buffer the rest
            // and report `needs_input == false` so the caller knows to call
            // `decompress(b'', max_length=...)` to drain it.
            state.input_buffer = remaining.to_vec();
            state.needs_input = false;
        } else if max_length == Some(0) {
            // Caller explicitly asked for zero output bytes. Keep whatever
            // input is left around for the next call and signal that they
            // do not need to feed more right now. CPython's decompressor
            // treats `max_length=0` as "stop here without losing state".
            state.input_buffer = remaining.to_vec();
            state.needs_input = false;
        } else {
            // All input consumed but the frame is not complete; the caller
            // should provide more data on the next call.
            state.input_buffer.clear();
            state.needs_input = true;
        }

        Ok(output)
    }

    #[pyclass(with(Constructor))]
    impl ZstdDecompressor {
        #[pymethod]
        fn decompress(&self, args: DecompressMethodArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let data_vec = args.data.with_ref(|b| b.to_vec());
            let max_length = if args.max_length < 0 {
                None
            } else {
                Some(args.max_length as usize)
            };
            let mut state = self.state.lock();
            if state.eof {
                return Err(vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "Already at the end of a Zstandard frame.".to_owned().into(),
                ));
            }
            do_decompress(&mut state, &data_vec, max_length, vm)
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.state.lock().eof
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            self.state.lock().needs_input
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.state.lock().unused_data.clone()
        }
    }

    #[pyfunction]
    fn get_frame_size(frame_buffer: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let buf = frame_buffer.with_ref(|b| b.to_vec());
        // SAFETY: `buf` is a live slice for the duration of the call.
        let size = unsafe { ZSTD_findFrameCompressedSize(buf.as_ptr().cast(), buf.len()) };
        if ZSTD_isError(size) != 0 {
            return Err(new_zstd_error(
                "Error when finding the compressed size of a Zstandard frame. \
                 Ensure the frame_buffer argument starts from the beginning of a frame, \
                 and its length not less than this complete frame.",
                vm,
            ));
        }
        Ok(size)
    }

    #[pyfunction]
    fn get_frame_info(
        frame_buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, u32)> {
        let buf = frame_buffer.with_ref(|b| b.to_vec());
        // SAFETY: `buf` is a live slice for the duration of the call.
        let content_size = unsafe { ZSTD_getFrameContentSize(buf.as_ptr().cast(), buf.len()) };
        if content_size == ZSTD_CONTENTSIZE_ERROR {
            return Err(new_zstd_error(
                "Error when getting information from the header of a Zstandard frame. \
                 Ensure the frame_buffer argument starts from the beginning of a frame, \
                 and its length not less than the frame header (6~18 bytes).",
                vm,
            ));
        }
        let content_size_obj: PyObjectRef = if content_size == ZSTD_CONTENTSIZE_UNKNOWN {
            vm.ctx.none()
        } else {
            vm.ctx.new_int(content_size).into()
        };
        // SAFETY: `buf` is a live slice for the duration of the call.
        let dict_id = unsafe { ZSTD_getDictID_fromFrame(buf.as_ptr().cast(), buf.len()) };
        Ok((content_size_obj, dict_id))
    }

    #[derive(FromArgs)]
    pub(super) struct TrainDictArgs {
        /// Concatenated sample bytes. Must be a `bytes` object, not
        /// `bytearray` or another buffer type, to match CPython's strict
        /// type-checking on this argument.
        #[pyarg(positional)]
        samples_bytes: PyBytesRef,
        /// A tuple of integer sample sizes that partition `samples_bytes`.
        /// Lists and other iterables are not accepted.
        #[pyarg(positional)]
        samples_sizes: PyTupleRef,
        /// Maximum size of the returned dictionary, in bytes. Must be a
        /// positive `int`.
        #[pyarg(positional)]
        dict_size: PyObjectRef,
    }

    /// Collect the elements of `tuple` into a `Vec<usize>`, validating that
    /// each element is a non-negative int that fits in `usize`. Used by both
    /// `train_dict` and `finalize_dict` for the `samples_sizes` argument.
    ///
    /// Floats (and any object whose `__index__` slot is missing) raise
    /// `TypeError`; values that do not fit `usize` raise `ValueError` so
    /// the test suite's `(2**1000,)` / `(-1,)` coverage holds.
    fn parse_sample_sizes(tuple: PyTupleRef, vm: &VirtualMachine) -> PyResult<Vec<usize>> {
        let items = tuple.as_slice();
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let idx = item.try_index(vm)?;
            let v: usize = idx
                .try_to_primitive(vm)
                .map_err(|_| vm.new_value_error("sample size out of range for size_t"))?;
            out.push(v);
        }
        Ok(out)
    }

    /// Convert a Python `int` to a positive `isize`. Rejects floats (via
    /// `try_index`) and non-positive values; bigints that don't fit `isize`
    /// propagate as `OverflowError`. Used for the `dict_size` argument of
    /// `train_dict` and `finalize_dict`, which must always be a strictly
    /// positive int.
    fn parse_positive_dict_size(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
        let idx = obj.try_index(vm)?;
        // `try_to_primitive::<isize>` raises `OverflowError` on bigints out of
        // range; pass that through verbatim so the test suite's
        // `assertRaises(OverflowError)` coverage matches.
        let v: isize = idx.try_to_primitive(vm)?;
        if v <= 0 {
            return Err(vm.new_value_error("dict_size must be positive"));
        }
        Ok(v as usize)
    }

    /// Sum the per-sample sizes and check they exactly cover `expected_total`,
    /// rejecting overflow. Any call into libzstd with these sizes must not
    /// let a wrapping sum sneak past the equality check, since libzstd would
    /// then read past the samples buffer.
    fn check_sample_sizes_match(
        sizes: &[usize],
        expected_total: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let mismatch = || -> PyBaseExceptionRef {
            vm.new_value_error("The samples size tuple doesn't match the concatenation's size")
        };
        let total = sizes
            .iter()
            .try_fold(0usize, |a, &b| a.checked_add(b))
            .ok_or_else(mismatch)?;
        if total != expected_total {
            return Err(mismatch());
        }
        Ok(())
    }

    #[pyfunction]
    fn train_dict(args: TrainDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let dict_size = parse_positive_dict_size(&args.dict_size, vm)?;
        let samples_buffer = args.samples_bytes.as_bytes().to_vec();
        let sizes = parse_sample_sizes(args.samples_sizes, vm)?;
        check_sample_sizes_match(&sizes, samples_buffer.len(), vm)?;
        let mut dict_buffer: Vec<u8> = Vec::with_capacity(dict_size);
        // SAFETY: `dict_buffer`'s spare capacity is valid for writes of up to
        // `dict_size` bytes; the samples and sizes buffers are live slices
        // for the duration of the call.
        let written = unsafe {
            ZDICT_trainFromBuffer(
                dict_buffer.as_mut_ptr().cast(),
                dict_size,
                samples_buffer.as_ptr().cast(),
                sizes.as_ptr(),
                sizes.len() as u32,
            )
        };
        if ZDICT_isError(written) != 0 {
            // SAFETY: `ZDICT_getErrorName` returns a pointer to a static,
            // NUL-terminated error string from libzstd's error table.
            let name = unsafe { CStr::from_ptr(ZDICT_getErrorName(written)) };
            return Err(new_zstd_error(name.to_string_lossy(), vm));
        }
        // SAFETY: `ZDICT_trainFromBuffer` wrote `written` (<= dict_size)
        // bytes into the buffer's spare capacity.
        unsafe { dict_buffer.set_len(written) };
        Ok(dict_buffer)
    }

    #[derive(FromArgs)]
    pub(super) struct FinalizeDictArgs {
        /// Raw "starting" dictionary content to finalize. Must be `bytes`
        /// (not `bytearray`) to match CPython.
        #[pyarg(positional)]
        custom_dict_bytes: PyBytesRef,
        /// Concatenated sample bytes used to derive the dictionary's
        /// statistics tables. Must be `bytes`.
        #[pyarg(positional)]
        samples_bytes: PyBytesRef,
        /// Tuple of integer sample sizes partitioning `samples_bytes`.
        #[pyarg(positional)]
        samples_sizes: PyTupleRef,
        /// Maximum size of the finalized dictionary, in bytes. Positive int.
        #[pyarg(positional)]
        dict_size: PyObjectRef,
        /// Compression level the dictionary will be tuned for. Must be int.
        #[pyarg(positional)]
        compression_level: PyObjectRef,
    }

    #[pyfunction]
    fn finalize_dict(args: FinalizeDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let dict_size = parse_positive_dict_size(&args.dict_size, vm)?;
        let compression_level: i32 = args.compression_level.try_to_value(vm)?;
        let custom_dict = args.custom_dict_bytes.as_bytes().to_vec();
        let samples_buffer = args.samples_bytes.as_bytes().to_vec();
        let sizes = parse_sample_sizes(args.samples_sizes, vm)?;
        check_sample_sizes_match(&sizes, samples_buffer.len(), vm)?;

        let mut dict_buffer: Vec<u8> = vec![0u8; dict_size];
        let params = ZDICT_params_t {
            compressionLevel: compression_level,
            notificationLevel: 0,
            dictID: 0,
        };

        // SAFETY: All pointers point into Rust-owned, properly sized buffers
        // that outlive the FFI call. ZDICT_finalizeDictionary just reads from
        // the sample/dict buffers and writes into `dict_buffer`.
        let written = unsafe {
            ZDICT_finalizeDictionary(
                dict_buffer.as_mut_ptr() as *mut _,
                dict_buffer.len(),
                custom_dict.as_ptr() as *const _,
                custom_dict.len(),
                samples_buffer.as_ptr() as *const _,
                sizes.as_ptr(),
                sizes.len() as u32,
                params,
            )
        };
        if ZDICT_isError(written) != 0 {
            let err_ptr = ZDICT_getErrorName(written);
            let msg = if err_ptr.is_null() {
                "zstd dictionary finalization failed".to_string()
            } else {
                // SAFETY: `ZDICT_getErrorName` returns a pointer to a static,
                // NUL-terminated error string from libzstd's error table.
                unsafe { CStr::from_ptr(err_ptr) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(new_zstd_error(msg, vm));
        }
        dict_buffer.truncate(written);
        Ok(dict_buffer)
    }

    #[derive(FromArgs)]
    pub(super) struct ParamBoundsArgs {
        #[pyarg(positional)]
        parameter: i32,
        #[pyarg(named)]
        is_compress: bool,
    }

    #[pyfunction]
    fn get_param_bounds(args: ParamBoundsArgs, vm: &VirtualMachine) -> PyResult<(c_int, c_int)> {
        let unknown = || -> PyBaseExceptionRef {
            let kind = if args.is_compress {
                "compression"
            } else {
                "decompression"
            };
            vm.new_value_error(format!(
                "invalid {kind} parameter 'unknown parameter (key {})'",
                args.parameter
            ))
        };
        // Validate the id via the same safe enum-lookup helpers used in
        // `lookup_param_bounds`, then call libzstd directly so we can
        // distinguish a libzstd-reported error from our own "unknown". The
        // two `getBounds` functions return distinct (but layout-identical)
        // `ZSTD_bounds` types, so destructure into a tuple right away.
        let (error, lo, hi) = if args.is_compress {
            let p = c_param_enum(args.parameter).ok_or_else(unknown)?;
            let b = ZSTD_cParam_getBounds(p);
            (b.error, b.lowerBound, b.upperBound)
        } else {
            let p = d_param_enum(args.parameter).ok_or_else(unknown)?;
            let b = ZSTD_dParam_getBounds(p);
            (b.error, b.lowerBound, b.upperBound)
        };
        if ZSTD_isError(error) != 0 {
            return Err(catch_zstd_error(error, vm));
        }
        Ok((lo, hi))
    }

    // Register the `CompressionParameter` / `DecompressionParameter` enum
    // classes defined by the pure-Python wrapper so [`check_wrong_param_kind`]
    // can reject a key from the wrong parameter family by identity. The types
    // are stashed as private `_zstd` module attributes — the RustPython
    // equivalent of the module state CPython keeps these in. The wrapper in
    // `Lib/compression/zstd/__init__.py` calls this exactly once at import.
    #[pyfunction]
    fn set_parameter_types(
        c_parameter_type: PyTypeRef,
        d_parameter_type: PyTypeRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let module = vm.import("_zstd", 0)?;
        module.set_attr("_compression_parameter_type", c_parameter_type, vm)?;
        module.set_attr("_decompression_parameter_type", d_parameter_type, vm)?;
        Ok(())
    }
}
