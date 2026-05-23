// spell-checker:ignore cctx dctx CCTX DCTX ldm cdict ddict windowlog hashlog chainlog searchlog
// spell-checker:ignore minmatch dictid checksumflag dstream cstream pyobj zstandard btopt btultra
// spell-checker:ignore btlazy dfast nbworkers windowlogmax windowlog overlap targetcblock
// spell-checker:ignore srcsize zdict refprefix refcdict refddict pledgedsrcsize getframecontentsize
// spell-checker:ignore Zstd Zstandard pylib RFC
// spell-checker:ignore CLEVEL zstdmodule cparameter dparameter maxl
// spell-checker:ignore cctx dctx CCTX DCTX ldm cdict ddict windowlog hashlog chainlog searchlog CLEVEL

//! The `_zstd` extension module. Backs the pure-Python `compression.zstd`
//! package by exposing the same classes, functions and constants that
//! CPython's `Modules/_zstd/` exposes. The Python wrapper at
//! `Lib/compression/zstd/__init__.py` imports from this module unconditionally,
//! so the names and call signatures here must stay in sync with CPython.
//!
//! Backend: the `zstd_safe` Rust crate (a thin safe wrapper over Facebook's
//! libzstd C library, which is what CPython links against). A handful of
//! routines that `zstd_safe` does not expose at a safe level (parameter
//! bounds, dictionary finalization) drop down to raw `zstd_sys` FFI calls.

pub(crate) use _zstd::module_def;

// The compression/decompression parameter and strategy constants below use
// CPython's `ZSTD_c_camelCase` / `ZSTD_d_camelCase` naming convention so the
// pure-Python `compression.zstd` package, which references them by those exact
// names, keeps working unchanged.
#[allow(non_upper_case_globals)]
#[pymodule]
mod _zstd {
    use core::ffi::c_int;
    use rustpython_common::lock::PyMutex;
    use rustpython_vm::builtins::{
        PyBaseExceptionRef, PyBytesRef, PyDict, PyTupleRef, PyType, PyTypeRef,
    };
    use rustpython_vm::function::{ArgBytesLike, OptionalArg};
    use rustpython_vm::types::{AsMapping, Constructor, Representable};
    use rustpython_vm::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    };
    use zstd_safe::zstd_sys;
    use zstd_safe::{CCtx, CParameter, DCtx, DParameter, InBuffer, OutBuffer};

    /// Class names of the `CompressionParameter` and `DecompressionParameter`
    /// `IntEnum`s defined by the pure-Python wrapper at
    /// `Lib/compression/zstd/__init__.py`. We identify these by name rather
    /// than identity because storing `PyTypeRef` in a Rust `static` requires
    /// `Sync`, which `PyMutex` does not provide on single-threaded targets
    /// (Android, iOS, wasm32). Name comparison is sufficient since both names
    /// originate from a module we ship and control.
    const C_PARAMETER_TYPE_NAME: &str = "CompressionParameter";
    const D_PARAMETER_TYPE_NAME: &str = "DecompressionParameter";

    // =========================================================================
    // Module-level constants
    // =========================================================================

    /// Default compression level used when none is requested. Matches
    /// libzstd's `ZSTD_CLEVEL_DEFAULT`.
    #[pyattr]
    const ZSTD_CLEVEL_DEFAULT: i32 = zstd_sys::ZSTD_CLEVEL_DEFAULT as i32;

    // Compression parameter identifiers. Values match the `ZSTD_cParameter`
    // enum in libzstd, which is what the public `CompressionParameter` IntEnum
    // in `Lib/compression/zstd/__init__.py` derives its members from.
    #[pyattr]
    const ZSTD_c_compressionLevel: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_compressionLevel as i32;
    #[pyattr]
    const ZSTD_c_windowLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_windowLog as i32;
    #[pyattr]
    const ZSTD_c_hashLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_hashLog as i32;
    #[pyattr]
    const ZSTD_c_chainLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_chainLog as i32;
    #[pyattr]
    const ZSTD_c_searchLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_searchLog as i32;
    #[pyattr]
    const ZSTD_c_minMatch: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_minMatch as i32;
    #[pyattr]
    const ZSTD_c_targetLength: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_targetLength as i32;
    #[pyattr]
    const ZSTD_c_strategy: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_strategy as i32;
    #[pyattr]
    const ZSTD_c_enableLongDistanceMatching: i32 =
        zstd_sys::ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching as i32;
    #[pyattr]
    const ZSTD_c_ldmHashLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_ldmHashLog as i32;
    #[pyattr]
    const ZSTD_c_ldmMinMatch: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_ldmMinMatch as i32;
    #[pyattr]
    const ZSTD_c_ldmBucketSizeLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_ldmBucketSizeLog as i32;
    #[pyattr]
    const ZSTD_c_ldmHashRateLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_ldmHashRateLog as i32;
    #[pyattr]
    const ZSTD_c_contentSizeFlag: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_contentSizeFlag as i32;
    #[pyattr]
    const ZSTD_c_checksumFlag: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_checksumFlag as i32;
    #[pyattr]
    const ZSTD_c_dictIDFlag: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_dictIDFlag as i32;
    #[pyattr]
    const ZSTD_c_nbWorkers: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_nbWorkers as i32;
    #[pyattr]
    const ZSTD_c_jobSize: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_jobSize as i32;
    #[pyattr]
    const ZSTD_c_overlapLog: i32 = zstd_sys::ZSTD_cParameter::ZSTD_c_overlapLog as i32;

    // Decompression parameter identifiers. libzstd only exposes one non-
    // experimental decompression parameter.
    #[pyattr]
    const ZSTD_d_windowLogMax: i32 = zstd_sys::ZSTD_dParameter::ZSTD_d_windowLogMax as i32;

    // Strategy enum members ordered from fastest to strongest. These power
    // the `Strategy` IntEnum in `Lib/compression/zstd/__init__.py`.
    #[pyattr]
    const ZSTD_fast: i32 = zstd_sys::ZSTD_strategy::ZSTD_fast as i32;
    #[pyattr]
    const ZSTD_dfast: i32 = zstd_sys::ZSTD_strategy::ZSTD_dfast as i32;
    #[pyattr]
    const ZSTD_greedy: i32 = zstd_sys::ZSTD_strategy::ZSTD_greedy as i32;
    #[pyattr]
    const ZSTD_lazy: i32 = zstd_sys::ZSTD_strategy::ZSTD_lazy as i32;
    #[pyattr]
    const ZSTD_lazy2: i32 = zstd_sys::ZSTD_strategy::ZSTD_lazy2 as i32;
    #[pyattr]
    const ZSTD_btlazy2: i32 = zstd_sys::ZSTD_strategy::ZSTD_btlazy2 as i32;
    #[pyattr]
    const ZSTD_btopt: i32 = zstd_sys::ZSTD_strategy::ZSTD_btopt as i32;
    #[pyattr]
    const ZSTD_btultra: i32 = zstd_sys::ZSTD_strategy::ZSTD_btultra as i32;
    #[pyattr]
    const ZSTD_btultra2: i32 = zstd_sys::ZSTD_strategy::ZSTD_btultra2 as i32;

    /// Runtime version string of the linked libzstd, like `"1.5.7"`.
    #[pyattr(once, name = "zstd_version")]
    fn zstd_version(_vm: &VirtualMachine) -> String {
        zstd_safe::version_string().to_string()
    }

    /// Packed integer version: `MAJOR * 10000 + MINOR * 100 + RELEASE`.
    /// The Python wrapper unpacks this into a `zstd_version_info` tuple.
    #[pyattr(once, name = "zstd_version_number")]
    fn zstd_version_number(_vm: &VirtualMachine) -> u32 {
        zstd_safe::version_number()
    }

    /// Recommended output buffer size for decompression. The `ZstdFile.read1`
    /// wrapper uses this as its default read size.
    #[pyattr(once, name = "ZSTD_DStreamOutSize")]
    fn zstd_dstream_out_size(_vm: &VirtualMachine) -> usize {
        DCtx::out_size()
    }

    // Dictionary load type markers. The `ZstdDict.as_*` properties wrap the
    // dictionary in a `(zdict, marker)` tuple so the compressor or decompressor
    // constructor knows which load mode to apply. Numbering matches CPython's
    // `Modules/_zstd/_zstdmodule.h::DictType`.
    const DICT_TYPE_DIGESTED: i32 = 0;
    const DICT_TYPE_UNDIGESTED: i32 = 1;
    const DICT_TYPE_PREFIX: i32 = 2;

    // =========================================================================
    // ZstdError exception
    // =========================================================================

    /// `_zstd.ZstdError`: the exception type raised for libzstd errors and for
    /// invalid usage (calling `decompress` on a closed decompressor, etc.).
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
        new_zstd_error(zstd_safe::get_error_name(code).to_string(), vm)
    }

    // =========================================================================
    // Parameter helpers
    // =========================================================================

    /// Convert any int-like Python value to an `i32`, rejecting floats. CPython
    /// uses `PyLong_AsInt` here which only accepts integers; `try_index` is
    /// the equivalent of CPython's `__index__` slot lookup (rejects floats).
    fn pyobj_to_i32(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<i32> {
        obj.try_index(vm)?.try_to_primitive(vm)
    }

    /// Normalize an `OptionalArg<PyObjectRef>` constructor parameter to an
    /// `Option<PyObjectRef>`. Treats both "argument missing" and "argument
    /// explicitly None" the same way (consistent with CPython, which uses
    /// `None` defaults throughout the `_zstd` API).
    fn arg_or_none(arg: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> Option<PyObjectRef> {
        arg.into_option().filter(|o| !vm.is_none(o))
    }

    /// If `key`'s class is the wrong kind of parameter enum for the caller's
    /// context (a `CompressionParameter` passed to a decompressor, or vice
    /// versa), return a `TypeError`. Used by both (de)compressor option-dict
    /// loops so the resulting message names the actual type rather than the
    /// generic "out of range" wording.
    ///
    /// We identify the enums by class name (see [`C_PARAMETER_TYPE_NAME`]),
    /// which avoids storing a `PyTypeRef` in a Rust `static` (impossible on
    /// single-threaded builds whose `PyMutex` is not `Sync`).
    fn check_wrong_param_kind(
        key: &PyObjectRef,
        is_compress: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let class_name = key.class().name();
        let (forbidden, kind) = if is_compress {
            (D_PARAMETER_TYPE_NAME, "compression")
        } else {
            (C_PARAMETER_TYPE_NAME, "decompression")
        };
        if &*class_name == forbidden {
            return Err(vm.new_type_error(format!(
                "{kind} options dictionary key must not be a {forbidden}"
            )));
        }
        Ok(())
    }

    /// Map a compression parameter id to its raw libzstd `ZSTD_cParameter`
    /// enum value. Returns `None` for unknown ids so callers can surface a
    /// targeted `ValueError`. Done with an explicit match rather than
    /// `mem::transmute` so passing junk like `ZSTD_cParameter(42)` cannot
    /// be triggered from Python.
    fn c_param_enum(param: i32) -> Option<zstd_sys::ZSTD_cParameter> {
        use zstd_sys::ZSTD_cParameter as P;
        Some(match param {
            ZSTD_c_compressionLevel => P::ZSTD_c_compressionLevel,
            ZSTD_c_windowLog => P::ZSTD_c_windowLog,
            ZSTD_c_hashLog => P::ZSTD_c_hashLog,
            ZSTD_c_chainLog => P::ZSTD_c_chainLog,
            ZSTD_c_searchLog => P::ZSTD_c_searchLog,
            ZSTD_c_minMatch => P::ZSTD_c_minMatch,
            ZSTD_c_targetLength => P::ZSTD_c_targetLength,
            ZSTD_c_strategy => P::ZSTD_c_strategy,
            ZSTD_c_enableLongDistanceMatching => P::ZSTD_c_enableLongDistanceMatching,
            ZSTD_c_ldmHashLog => P::ZSTD_c_ldmHashLog,
            ZSTD_c_ldmMinMatch => P::ZSTD_c_ldmMinMatch,
            ZSTD_c_ldmBucketSizeLog => P::ZSTD_c_ldmBucketSizeLog,
            ZSTD_c_ldmHashRateLog => P::ZSTD_c_ldmHashRateLog,
            ZSTD_c_contentSizeFlag => P::ZSTD_c_contentSizeFlag,
            ZSTD_c_checksumFlag => P::ZSTD_c_checksumFlag,
            ZSTD_c_dictIDFlag => P::ZSTD_c_dictIDFlag,
            ZSTD_c_nbWorkers => P::ZSTD_c_nbWorkers,
            ZSTD_c_jobSize => P::ZSTD_c_jobSize,
            ZSTD_c_overlapLog => P::ZSTD_c_overlapLog,
            _ => return None,
        })
    }

    /// Map a decompression parameter id to its raw libzstd `ZSTD_dParameter`
    /// enum value. See [`c_param_enum`] for rationale.
    fn d_param_enum(param: i32) -> Option<zstd_sys::ZSTD_dParameter> {
        use zstd_sys::ZSTD_dParameter as P;
        match param {
            ZSTD_d_windowLogMax => Some(P::ZSTD_d_windowLogMax),
            _ => None,
        }
    }

    /// Map a compression-parameter id and integer value to a `CParameter`
    /// variant. Used by the compressor's `options=` constructor argument.
    fn cparameter_from_int(param: i32, value: i32, vm: &VirtualMachine) -> PyResult<CParameter> {
        let p = match param {
            ZSTD_c_compressionLevel => CParameter::CompressionLevel(value),
            ZSTD_c_windowLog => CParameter::WindowLog(value as u32),
            ZSTD_c_hashLog => CParameter::HashLog(value as u32),
            ZSTD_c_chainLog => CParameter::ChainLog(value as u32),
            ZSTD_c_searchLog => CParameter::SearchLog(value as u32),
            ZSTD_c_minMatch => CParameter::MinMatch(value as u32),
            ZSTD_c_targetLength => CParameter::TargetLength(value as u32),
            ZSTD_c_strategy => {
                CParameter::Strategy(strategy_from_int(value).ok_or_else(|| {
                    new_zstd_error(format!("invalid strategy value: {value}"), vm)
                })?)
            }
            ZSTD_c_enableLongDistanceMatching => CParameter::EnableLongDistanceMatching(value != 0),
            ZSTD_c_ldmHashLog => CParameter::LdmHashLog(value as u32),
            ZSTD_c_ldmMinMatch => CParameter::LdmMinMatch(value as u32),
            ZSTD_c_ldmBucketSizeLog => CParameter::LdmBucketSizeLog(value as u32),
            ZSTD_c_ldmHashRateLog => CParameter::LdmHashRateLog(value as u32),
            ZSTD_c_contentSizeFlag => CParameter::ContentSizeFlag(value != 0),
            ZSTD_c_checksumFlag => CParameter::ChecksumFlag(value != 0),
            ZSTD_c_dictIDFlag => CParameter::DictIdFlag(value != 0),
            ZSTD_c_nbWorkers => CParameter::NbWorkers(value as u32),
            ZSTD_c_jobSize => CParameter::JobSize(value as u32),
            ZSTD_c_overlapLog => CParameter::OverlapSizeLog(value as u32),
            _ => {
                return Err(vm.new_value_error(format!(
                    "invalid compression parameter 'unknown parameter (key {param})'"
                )));
            }
        };
        Ok(p)
    }

    /// Map a decompression-parameter id and integer value to a `DParameter`.
    /// Used by the decompressor's `options=` constructor argument.
    fn dparameter_from_int(param: i32, value: i32, vm: &VirtualMachine) -> PyResult<DParameter> {
        match param {
            ZSTD_d_windowLogMax => Ok(DParameter::WindowLogMax(value as u32)),
            _ => Err(vm.new_value_error(format!(
                "invalid decompression parameter 'unknown parameter (key {param})'"
            ))),
        }
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
        // SAFETY: `ZSTD_*Param_getBounds` reads no memory beyond the enum
        // discriminant; the helpers above validated that `param` maps to a
        // real enum variant.
        let bounds = if is_compress {
            let p = c_param_enum(param)?;
            unsafe { zstd_sys::ZSTD_cParam_getBounds(p) }
        } else {
            let p = d_param_enum(param)?;
            unsafe { zstd_sys::ZSTD_dParam_getBounds(p) }
        };
        // SAFETY: ZSTD_isError just inspects the bounds.error integer.
        if unsafe { zstd_sys::ZSTD_isError(bounds.error) } != 0 {
            return None;
        }
        Some((bounds.lowerBound, bounds.upperBound))
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
    /// to the underlying `ZSTD_strategy` C enum value. Done via an explicit
    /// match for the same reason as [`c_param_enum`]: an untrusted int
    /// might not correspond to any real enum variant.
    fn strategy_from_int(v: i32) -> Option<zstd_sys::ZSTD_strategy> {
        use zstd_sys::ZSTD_strategy as S;
        Some(match v {
            ZSTD_fast => S::ZSTD_fast,
            ZSTD_dfast => S::ZSTD_dfast,
            ZSTD_greedy => S::ZSTD_greedy,
            ZSTD_lazy => S::ZSTD_lazy,
            ZSTD_lazy2 => S::ZSTD_lazy2,
            ZSTD_btlazy2 => S::ZSTD_btlazy2,
            ZSTD_btopt => S::ZSTD_btopt,
            ZSTD_btultra => S::ZSTD_btultra,
            ZSTD_btultra2 => S::ZSTD_btultra2,
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
            let marker = pyobj_to_i32(marker_obj, vm).map_err(|e| {
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

    // =========================================================================
    // ZstdDict
    // =========================================================================

    #[derive(FromArgs)]
    pub(super) struct ZstdDictArgs {
        #[pyarg(positional)]
        dict_content: ArgBytesLike,
        #[pyarg(named, default = false)]
        is_raw: bool,
    }

    /// ZstdDict(dict_content, /, *, is_raw=False)
    ///
    /// Represents a Zstandard dictionary. Hold the raw dictionary bytes and
    /// the parsed `dict_id`. The same instance may be shared by many
    /// compressors and decompressors.
    ///
    /// *dict_content* is a bytes-like object containing the dictionary
    /// content. If *is_raw* is False (the default), the content is expected
    /// to start with the Zstandard dictionary magic number and have a valid
    /// header. If *is_raw* is True, arbitrary bytes are accepted and the
    /// dictionary is treated as raw back-reference content.
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
            // libzstd's `get_dict_id_from_dict` returns `None` either when
            // the content is too small to contain a valid header or when it
            // does not carry the dictionary magic. Both are runtime errors
            // when `is_raw=False`, matching CPython's behavior of raising
            // `ValueError` on a non-conformant dictionary.
            let parsed_id = zstd_safe::get_dict_id_from_dict(&dict_content).map_or(0, |n| n.get());
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
        /// The content of a Zstandard dictionary, as a bytes object.
        #[pygetset]
        fn dict_content(&self) -> PyBytesRef {
            self.dict_content.clone()
        }

        /// The Zstandard dictionary ID.
        ///
        /// A non-zero value identifies a standard-format dictionary. A value
        /// of zero either means the dictionary is in raw-content form or
        /// that the dictionary ID was deliberately omitted from the header.
        /// Dictionary IDs fall in the range 0 to 2**32 - 1.
        #[pygetset]
        fn dict_id(&self) -> u32 {
            self.dict_id
        }

        /// Load this ZstdDict as a digested dictionary.
        ///
        /// Returns a `(self, DICT_TYPE_DIGESTED)` tuple that can be passed
        /// as the `zstd_dict=` argument of `ZstdCompressor` or
        /// `ZstdDecompressor`. Digested form lets libzstd cache the parsed
        /// dictionary for reuse across compression contexts.
        #[pygetset]
        fn as_digested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(DICT_TYPE_DIGESTED).into()])
        }

        /// Load this ZstdDict as an undigested dictionary.
        ///
        /// Returns a `(self, DICT_TYPE_UNDIGESTED)` tuple. In undigested
        /// form the raw dictionary bytes are copied into each
        /// (de)compressor, which is slower than digested but preserves the
        /// original dictionary parameters across loads.
        #[pygetset]
        fn as_undigested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx.new_tuple(vec![
                zelf.into(),
                vm.ctx.new_int(DICT_TYPE_UNDIGESTED).into(),
            ])
        }

        /// Load this ZstdDict as a prefix dictionary.
        ///
        /// Returns a `(self, DICT_TYPE_PREFIX)` tuple. Prefix loading treats
        /// the dictionary bytes as a one-shot back-reference window for the
        /// next frame only; subsequent frames will not see the prefix.
        #[pygetset]
        fn as_prefix(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(DICT_TYPE_PREFIX).into()])
        }
    }

    // =========================================================================
    // ZstdCompressor
    // =========================================================================

    // The three flush modes for `ZstdCompressor.compress()`, mirrored as
    // class attributes via `extend_class` below. Values are positional and
    // chosen to match what CPython exposes.
    const COMP_MODE_CONTINUE: i32 = 0;
    const COMP_MODE_FLUSH_BLOCK: i32 = 1;
    const COMP_MODE_FLUSH_FRAME: i32 = 2;

    /// Internal state of a `ZstdCompressor`. Holds the libzstd context, the
    /// last mode used (for `last_mode` and `set_pledged_input_size`
    /// validation), and the dictionary handles that the context may reference
    /// internally. Field order matters here: Rust drops in declaration order,
    /// so `cctx` is freed first; the held `CDict` (if any) and the source
    /// `PyRef<ZstdDict>` go away afterwards, which is the safe order for
    /// teardown.
    struct CompressorState {
        cctx: CCtx<'static>,
        /// Cached digested dictionary. The CCtx references this via
        /// `ref_cdict`, so it must outlive the CCtx (handled by Rust's
        /// field drop order: `cctx` drops first).
        _cdict: Option<zstd_safe::CDict<'static>>,
        /// Keeps the ZstdDict's bytes alive for `ref_prefix` mode.
        _dict: Option<PyRef<ZstdDict>>,
        last_mode: i32,
    }

    /// ZstdCompressor(level=None, options=None, zstd_dict=None)
    ///
    /// Create a compressor object for incremental compression of data into
    /// the Zstandard format.
    ///
    /// *level* is an int specifying the compression level. Valid range is
    /// roughly [-(1<<17), 22]; 0 means "use the default level". Cannot be
    /// combined with *options*.
    ///
    /// *options* is a dict mapping `CompressionParameter` enum members to
    /// integer values for fine-grained control. Cannot be combined with
    /// *level*.
    ///
    /// *zstd_dict* is an optional `ZstdDict` (or one of its `as_*`
    /// variants) for dictionary-assisted compression.
    ///
    /// Use `compress()` to feed data into the compressor and `flush()` to
    /// finalize the current frame.
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
        level: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        options: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        zstd_dict: OptionalArg<PyObjectRef>,
    }

    /// Translate the public `mode` int to the libzstd `ZSTD_EndDirective`
    /// the streaming API takes.
    fn end_directive_from_mode(
        mode: i32,
        vm: &VirtualMachine,
    ) -> PyResult<zstd_sys::ZSTD_EndDirective> {
        match mode {
            COMP_MODE_CONTINUE => Ok(zstd_sys::ZSTD_EndDirective::ZSTD_e_continue),
            COMP_MODE_FLUSH_BLOCK => Ok(zstd_sys::ZSTD_EndDirective::ZSTD_e_flush),
            COMP_MODE_FLUSH_FRAME => Ok(zstd_sys::ZSTD_EndDirective::ZSTD_e_end),
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
            let level_opt = arg_or_none(args.level, vm);
            let options_opt = arg_or_none(args.options, vm);
            let dict_opt = arg_or_none(args.zstd_dict, vm);

            if level_opt.is_some() && options_opt.is_some() {
                return Err(vm.new_runtime_error("Only one of level or options should be used"));
            }

            let mut cctx = CCtx::<'static>::create();

            if let Some(level_obj) = level_opt {
                let level = parse_compression_level(&level_obj, vm)?;
                cctx.set_parameter(CParameter::CompressionLevel(level))
                    .map_err(|_| param_value_error_for(ZSTD_c_compressionLevel, level, true, vm))?;
            }

            if let Some(options_obj) = options_opt {
                apply_options(&mut cctx, options_obj, true, vm)?;
            }

            let (held_cdict, held_dict) = load_compressor_dict(&mut cctx, dict_opt, vm)?;

            Ok(Self {
                state: PyMutex::new(CompressorState {
                    cctx,
                    _cdict: held_cdict,
                    _dict: held_dict,
                    last_mode: COMP_MODE_FLUSH_FRAME,
                }),
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
        let level = pyobj_to_i32(obj, vm).map_err(|e| {
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
        for (k, v) in dict {
            // Reject the wrong enum kind ("CompressionParameter" vs
            // "DecompressionParameter") before doing any numeric coercion,
            // so the error carries the documented type name rather than a
            // generic out-of-range message.
            check_wrong_param_kind(&k, is_compress, vm)?;
            let key_int = pyobj_to_i32(&k, vm)?;
            let val_int = pyobj_to_i32(&v, vm)?;
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
    /// `set_parameter` translates the (id, value) pair to the appropriate
    /// `CParameter` / `DParameter` enum variant, then forwards to libzstd.
    trait ParamSetter {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()>;
    }

    impl ParamSetter for CCtx<'static> {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            let p = cparameter_from_int(param, value, vm)?;
            self.set_parameter(p)
                .map_err(|_| param_value_error_for(param, value, true, vm))?;
            Ok(())
        }
    }

    impl ParamSetter for DCtx<'static> {
        fn apply(&mut self, param: i32, value: i32, vm: &VirtualMachine) -> PyResult<()> {
            let p = dparameter_from_int(param, value, vm)?;
            self.set_parameter(p)
                .map_err(|_| param_value_error_for(param, value, false, vm))?;
            Ok(())
        }
    }

    /// Trait that captures the only differences between how the compressor
    /// and decompressor consume a dictionary: the name of the type that
    /// appears in error messages, the eager-validation constructor for the
    /// digested variant, and the three ways of attaching it to the context.
    trait DictLoader<'a> {
        type Digested;
        const KIND_NAME: &'static str;
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested>;
        fn ref_digested(&mut self, dict: &Self::Digested) -> zstd_safe::SafeResult;
        fn load_undigested(&mut self, bytes: &[u8]) -> zstd_safe::SafeResult;
        fn ref_prefix_static(&mut self, bytes: &'static [u8]) -> zstd_safe::SafeResult;
    }

    impl DictLoader<'static> for CCtx<'static> {
        type Digested = zstd_safe::CDict<'static>;
        const KIND_NAME: &'static str = "ZSTD_CDict";
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested> {
            zstd_safe::CDict::try_create(bytes, ZSTD_CLEVEL_DEFAULT)
        }
        fn ref_digested(&mut self, dict: &Self::Digested) -> zstd_safe::SafeResult {
            self.ref_cdict(dict)
        }
        fn load_undigested(&mut self, bytes: &[u8]) -> zstd_safe::SafeResult {
            self.load_dictionary(bytes)
        }
        fn ref_prefix_static(&mut self, bytes: &'static [u8]) -> zstd_safe::SafeResult {
            self.ref_prefix(bytes)
        }
    }

    impl DictLoader<'static> for DCtx<'static> {
        type Digested = zstd_safe::DDict<'static>;
        const KIND_NAME: &'static str = "ZSTD_DDict";
        fn try_create_digested(bytes: &[u8]) -> Option<Self::Digested> {
            zstd_safe::DDict::try_create(bytes)
        }
        fn ref_digested(&mut self, dict: &Self::Digested) -> zstd_safe::SafeResult {
            self.ref_ddict(dict)
        }
        fn load_undigested(&mut self, bytes: &[u8]) -> zstd_safe::SafeResult {
            self.load_dictionary(bytes)
        }
        fn ref_prefix_static(&mut self, bytes: &'static [u8]) -> zstd_safe::SafeResult {
            self.ref_prefix(bytes)
        }
    }

    /// Return value of `load_dict`: the digested `CDict`/`DDict` (if any)
    /// and the `PyRef<ZstdDict>` we hold to keep the dictionary bytes alive
    /// while `ref_prefix` may point into them.
    type DictLoadResult<D> = PyResult<(Option<D>, Option<PyRef<ZstdDict>>)>;

    /// Common path for attaching a dictionary to either context type. Returns
    /// the digested `CDict`/`DDict` (if the caller used digested mode) plus
    /// the `PyRef<ZstdDict>` we hold to keep the bytes alive for `ref_prefix`.
    fn load_dict<L: DictLoader<'static>>(
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
                // SAFETY: the returned `PyRef<ZstdDict>` keeps `zdict` alive
                // (and therefore `dict_bytes`) for the lifetime of the
                // context. libzstd `ref_prefix` stores a raw pointer into
                // `dict_bytes`; as long as the (de)compressor outlives no
                // longer than the held PyRef, the pointer stays valid.
                let static_bytes: &'static [u8] =
                    unsafe { core::slice::from_raw_parts(dict_bytes.as_ptr(), dict_bytes.len()) };
                ctx.ref_prefix_static(static_bytes)
                    .map_err(|_| bad_dict_err())?;
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

    fn load_compressor_dict(
        cctx: &mut CCtx<'static>,
        dict_obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> DictLoadResult<zstd_safe::CDict<'static>> {
        load_dict::<CCtx<'static>>(cctx, dict_obj, vm)
    }

    fn load_decompressor_dict(
        dctx: &mut DCtx<'static>,
        dict_obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> DictLoadResult<zstd_safe::DDict<'static>> {
        load_dict::<DCtx<'static>>(dctx, dict_obj, vm)
    }

    /// Drive `compress_stream2` until the input is fully consumed and, for
    /// flush/end directives, the internal buffers report zero remaining bytes.
    /// Grows the output `Vec` by `CStreamOutSize` chunks as needed.
    fn do_compress(
        state: &mut CompressorState,
        data: &[u8],
        end_op: zstd_sys::ZSTD_EndDirective,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let mut output = Vec::new();
        let mut input = InBuffer::around(data);
        let chunk_size = CCtx::out_size().max(1);
        let is_end = end_op != zstd_sys::ZSTD_EndDirective::ZSTD_e_continue;

        loop {
            let prev_len = output.len();
            output.reserve(chunk_size);
            let remaining = {
                let mut out_buf = OutBuffer::around_pos(&mut output, prev_len);
                state
                    .cctx
                    .compress_stream2(&mut out_buf, &mut input, end_op)
            }
            .map_err(|c| catch_zstd_error(c, vm))?;
            let consumed_all = input.pos == input.src.len();
            // Stop when input is fully consumed and, for flush/end directives,
            // libzstd reports that all internal buffers have been drained
            // (remaining == 0). Otherwise loop; the next `reserve` will grow
            // the output buffer if we hit the previous capacity.
            if consumed_all && (!is_end || remaining == 0) {
                break;
            }
        }
        Ok(output)
    }

    #[pyclass(with(Constructor))]
    impl ZstdCompressor {
        /// compress(data, mode=ZstdCompressor.CONTINUE)
        ///
        /// Provide data to the compressor and return a chunk of compressed
        /// output bytes (which may be empty if not enough data has been
        /// buffered yet).
        ///
        /// *mode* controls when to flush the output:
        ///   * `CONTINUE` (default) keeps buffering, producing the densest
        ///     compression. The compressor may return an empty byte string.
        ///   * `FLUSH_BLOCK` closes the current zstd block so all data
        ///     provided so far can be immediately decompressed. Frequent
        ///     use reduces the compression ratio.
        ///   * `FLUSH_FRAME` closes the current frame entirely. The next
        ///     `compress()` call starts a new frame.
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

        /// flush(mode=ZstdCompressor.FLUSH_FRAME)
        ///
        /// Finish the compression process and return all the remaining
        /// compressed bytes. *mode* must be one of `FLUSH_FRAME` (close the
        /// current frame; new frames may be started afterward) or
        /// `FLUSH_BLOCK` (close the current block but stay inside the
        /// frame). `CONTINUE` is not a valid value for `flush()`.
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

        /// set_pledged_input_size(size, /)
        ///
        /// Set the uncompressed content size to be written into the frame
        /// header of the next frame. Useful when the size is known up front:
        /// libzstd writes it into the header (unless the content-size flag
        /// is disabled) and verifies that the actual input matches at frame
        /// end. Pass `None` to mean "unknown".
        ///
        /// May only be called when `last_mode == FLUSH_FRAME` (i.e. between
        /// frames; not in the middle of one).
        #[pymethod]
        fn set_pledged_input_size(&self, size: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let mut state = self.state.lock();
            if state.last_mode != COMP_MODE_FLUSH_FRAME {
                return Err(vm.new_value_error(
                    "set_pledged_input_size() method must be called when last_mode == FLUSH_FRAME",
                ));
            }
            // Python passes `None` to mean "unknown"; libzstd represents that
            // internally as `ZSTD_CONTENTSIZE_UNKNOWN` (`u64::MAX`), and
            // `zstd_safe` translates `None` accordingly. Reject any concrete
            // int outside the valid range up front so callers see the
            // documented `ValueError`, not a libzstd-level error. The valid
            // upper bound is `ZSTD_CONTENTSIZE_ERROR - 1`, i.e. `u64::MAX - 2`.
            let pledged: Option<u64> = if vm.is_none(&size) {
                None
            } else {
                const UPPER_BOUND: u64 = u64::MAX - 2;
                let int_ref = size.try_index(vm)?;
                // Try fitting into u64; OverflowError covers negatives and
                // values above `u64::MAX`.
                let v: u64 = int_ref.try_to_primitive(vm).map_err(|_| {
                    vm.new_value_error(format!(
                        "size argument should be a positive int less than {UPPER_BOUND}"
                    ))
                })?;
                if v > UPPER_BOUND {
                    return Err(vm.new_value_error(format!(
                        "size argument should be a positive int less than {UPPER_BOUND}"
                    )));
                }
                Some(v)
            };
            state
                .cctx
                .set_pledged_src_size(pledged)
                .map_err(|c| catch_zstd_error(c, vm))?;
            Ok(())
        }

        /// The mode argument passed to the most recent `compress()` or
        /// `flush()` call.
        ///
        /// The value is one of `CONTINUE`, `FLUSH_BLOCK`, or `FLUSH_FRAME`.
        /// The initial value is `FLUSH_FRAME`, which signals that no frame
        /// is currently in progress (it is safe to call
        /// `set_pledged_input_size()` in this state).
        #[pygetset]
        fn last_mode(&self) -> i32 {
            self.state.lock().last_mode
        }

        /// Install class-level constants `CONTINUE`, `FLUSH_BLOCK`, and
        /// `FLUSH_FRAME` so callers can reference them as
        /// `ZstdCompressor.FLUSH_FRAME` (as the Python `ZstdFile` wrapper
        /// does).
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

    // =========================================================================
    // ZstdDecompressor
    // =========================================================================

    /// Internal state of a `ZstdDecompressor`. The CPython decompressor is
    /// single-frame: once we hit end-of-frame, additional bytes go into
    /// `unused_data` and further `decompress` calls raise `EOFError`. Field
    /// drop order matters here for the same reason as in `CompressorState`:
    /// the `dctx` is freed first and must give up its internal pointers
    /// before any referenced `DDict`/`PyRef<ZstdDict>` is dropped.
    struct DecompressorState {
        dctx: DCtx<'static>,
        /// Cached decompression dictionary referenced by the DCtx.
        _ddict: Option<zstd_safe::DDict<'static>>,
        _dict: Option<PyRef<ZstdDict>>,
        eof: bool,
        needs_input: bool,
        /// Bytes that arrived after the end of the first frame.
        unused_data: PyBytesRef,
        /// Input bytes buffered because the previous `decompress` call ran
        /// into its `max_length` cap before consuming them all.
        input_buffer: Vec<u8>,
    }

    /// ZstdDecompressor(zstd_dict=None, options=None)
    ///
    /// Create a decompressor object for incremental decompression of a
    /// single Zstandard frame. Once the frame is fully decompressed the
    /// decompressor is at EOF; trailing bytes are exposed via
    /// `unused_data`. For multi-frame input use the module-level
    /// `compression.zstd.decompress()` or `ZstdFile`.
    ///
    /// *zstd_dict* is an optional `ZstdDict` (or one of its `as_*`
    /// variants) matching the dictionary used during compression.
    ///
    /// *options* is a dict mapping `DecompressionParameter` enum members
    /// to integer values for fine-grained control.
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
        zstd_dict: OptionalArg<PyObjectRef>,
        #[pyarg(any, optional)]
        options: OptionalArg<PyObjectRef>,
    }

    impl Constructor for ZstdDecompressor {
        type Args = ZstdDecompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let dict_opt = arg_or_none(args.zstd_dict, vm);
            let options_opt = arg_or_none(args.options, vm);

            let mut dctx = DCtx::<'static>::create();

            if let Some(options_obj) = options_opt {
                apply_options(&mut dctx, options_obj, false, vm)?;
            }

            let (held_ddict, held_dict) = load_decompressor_dict(&mut dctx, dict_opt, vm)?;

            Ok(Self {
                state: PyMutex::new(DecompressorState {
                    dctx,
                    _ddict: held_ddict,
                    _dict: held_dict,
                    eof: false,
                    needs_input: true,
                    unused_data: vm.ctx.empty_bytes.clone(),
                    input_buffer: Vec::new(),
                }),
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

        let mut input = InBuffer::around(&work_data);
        let mut output: Vec<u8> = Vec::new();
        let chunk_size = DCtx::out_size().max(1);
        // Reusable scratch buffer for each decompress_stream call. We need an
        // exact-size output buffer because `Vec::reserve` may over-allocate;
        // OutBuffer reports the full Vec capacity to libzstd, which would
        // then happily write past `max_length`.
        let mut scratch: Vec<u8> = vec![0u8; chunk_size];
        let mut hit_max = false;
        let mut iteration = 0usize;

        loop {
            iteration += 1;
            // Honor `max_length`: stop growing the output buffer once we have
            // produced enough. Special-case the first iteration when the cap
            // is zero so a zero-output frame (skippable frame, empty content
            // frame) can still complete; we hand libzstd a 1-byte scratch
            // and discard the byte if it ends up writing one.
            let grow = match max_length {
                Some(maxl) if output.len() >= maxl && iteration > 1 => {
                    hit_max = true;
                    break;
                }
                Some(maxl) if output.len() >= maxl => 1,
                Some(maxl) => (maxl - output.len()).min(chunk_size),
                None => chunk_size,
            };
            let result;
            let written;
            {
                let slot = &mut scratch[..grow];
                let mut out_buf = OutBuffer::around(slot as &mut [u8]);
                result = state.dctx.decompress_stream(&mut out_buf, &mut input);
                written = out_buf.pos();
            }
            output.extend_from_slice(&scratch[..written]);
            match result {
                Ok(0) => {
                    // Frame fully decompressed; the decompressor is at EOF.
                    state.eof = true;
                    break;
                }
                Ok(_) => {
                    let output_was_full = written == grow;
                    let input_consumed = input.pos == input.src.len();

                    if let Some(maxl) = max_length
                        && output.len() >= maxl
                        && iteration > 1
                    {
                        hit_max = true;
                        break;
                    }

                    // Input is gone and libzstd had room to write but did not,
                    // which means the frame is incomplete and the caller has
                    // to supply more input.
                    if input_consumed && !output_was_full {
                        break;
                    }
                }
                Err(code) => return Err(catch_zstd_error(code, vm)),
            }
        }

        // If `max_length == 0` opened a courtesy iteration that produced more
        // bytes than the caller asked for, truncate. Should not happen with
        // the scratch slicing above, but keep the safety net.
        if let Some(maxl) = max_length
            && output.len() > maxl
        {
            output.truncate(maxl);
            hit_max = true;
        }

        let consumed = input.pos;
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
        /// decompress(data, max_length=-1)
        ///
        /// Decompress *data* and return at most *max_length* bytes of
        /// uncompressed output. A *max_length* of -1 (the default) means
        /// "no limit". After the current frame is fully decompressed, `eof`
        /// becomes `True` and any further `decompress()` calls raise
        /// `EOFError`; bytes that arrived after the frame go into
        /// `unused_data` for the caller to forward.
        ///
        /// If the output cap is reached with input still pending, the
        /// remaining input is buffered internally and `needs_input` is set
        /// to `False`, signalling that the next call can be made with an
        /// empty *data* argument to keep draining.
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

        /// True if the end-of-frame marker has been reached.
        #[pygetset]
        fn eof(&self) -> bool {
            self.state.lock().eof
        }

        /// True when the decompressor expects more compressed input.
        ///
        /// Becomes False after a `decompress()` call that hits its
        /// `max_length` cap with input still buffered internally; in that
        /// state the caller should call `decompress(b'', max_length=...)`
        /// to drain the buffered output before providing more input.
        #[pygetset]
        fn needs_input(&self) -> bool {
            self.state.lock().needs_input
        }

        /// Bytes left over after the end-of-frame marker.
        ///
        /// Empty until the decompressor reaches EOF. Read-only.
        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.state.lock().unused_data.clone()
        }
    }

    // =========================================================================
    // Module-level functions
    // =========================================================================

    /// get_frame_size(frame_buffer, /)
    ///
    /// Return the size in bytes of the first complete Zstandard frame in
    /// *frame_buffer* (which must contain the entire frame, header through
    /// trailer). Raises `ZstdError` if *frame_buffer* is shorter than the
    /// complete frame.
    #[pyfunction]
    fn get_frame_size(frame_buffer: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
        let buf = frame_buffer.with_ref(|b| b.to_vec());
        zstd_safe::find_frame_compressed_size(&buf).map_err(|_| {
            new_zstd_error(
                "Error when finding the compressed size of a Zstandard frame. \
                 Ensure the frame_buffer argument starts from the beginning of a frame, \
                 and its length not less than this complete frame.",
                vm,
            )
        })
    }

    /// get_frame_info(frame_buffer, /)
    ///
    /// Return `(decompressed_size, dictionary_id)` for the first Zstandard
    /// frame in *frame_buffer*. `decompressed_size` is `None` when the
    /// frame header does not record it (small or streaming frames).
    /// `dictionary_id` is 0 if no dictionary was used. *frame_buffer* must
    /// contain at least the 6-18 byte frame header. The pure-Python
    /// wrapper boxes this tuple into a `FrameInfo` named tuple.
    #[pyfunction]
    fn get_frame_info(
        frame_buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, u32)> {
        let buf = frame_buffer.with_ref(|b| b.to_vec());
        let content_size = zstd_safe::get_frame_content_size(&buf).map_err(|_| {
            new_zstd_error(
                "Error when getting information from the header of a Zstandard frame. \
                 Ensure the frame_buffer argument starts from the beginning of a frame, \
                 and its length not less than the frame header (6~18 bytes).",
                vm,
            )
        })?;
        let content_size_obj: PyObjectRef = match content_size {
            Some(n) => vm.ctx.new_int(n).into(),
            None => vm.ctx.none(),
        };
        let dict_id = zstd_safe::get_dict_id_from_frame(&buf).map_or(0, |n| n.get());
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
            let v: usize = idx.try_to_primitive(vm).map_err(|_| {
                vm.new_value_error("sample size out of range for size_t".to_owned())
            })?;
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

    /// train_dict(samples_bytes, samples_sizes, dict_size, /)
    ///
    /// Train a Zstandard dictionary from sample data and return the raw
    /// dictionary bytes. The pure-Python `train_dict` wrapper then wraps
    /// the result in a `ZstdDict`.
    ///
    /// `samples_bytes` is the concatenation of all sample contents.
    /// `samples_sizes` is a tuple giving the length of each sample, in the
    /// same order they appear in `samples_bytes`. `dict_size` is the
    /// maximum size of the dictionary in bytes.
    #[pyfunction]
    fn train_dict(args: TrainDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let dict_size = parse_positive_dict_size(&args.dict_size, vm)?;
        let samples_buffer = args.samples_bytes.as_bytes().to_vec();
        let sizes = parse_sample_sizes(args.samples_sizes, vm)?;
        let total: usize = sizes.iter().sum();
        if total != samples_buffer.len() {
            return Err(
                vm.new_value_error("The samples size tuple doesn't match the concatenation's size")
            );
        }
        let mut dict_buffer: Vec<u8> = Vec::with_capacity(dict_size);
        zstd_safe::train_from_buffer(&mut dict_buffer, &samples_buffer, &sizes)
            .map_err(|c| catch_zstd_error(c, vm))?;
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

    /// finalize_dict(custom_dict_bytes, samples_bytes, samples_sizes, dict_size, compression_level, /)
    ///
    /// Take a hand-crafted custom dictionary plus a set of samples and
    /// produce a standard-format Zstandard dictionary by appending the
    /// header and statistics tables. `zstd_safe` does not wrap this
    /// function, so we drop down to raw `zstd_sys` FFI for it.
    #[pyfunction]
    fn finalize_dict(args: FinalizeDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let dict_size = parse_positive_dict_size(&args.dict_size, vm)?;
        let compression_level = pyobj_to_i32(&args.compression_level, vm)?;
        let custom_dict = args.custom_dict_bytes.as_bytes().to_vec();
        let samples_buffer = args.samples_bytes.as_bytes().to_vec();
        let sizes = parse_sample_sizes(args.samples_sizes, vm)?;
        let total: usize = sizes.iter().sum();
        if total != samples_buffer.len() {
            return Err(
                vm.new_value_error("The samples size tuple doesn't match the concatenation's size")
            );
        }

        let mut dict_buffer: Vec<u8> = vec![0u8; dict_size];
        let params = zstd_sys::ZDICT_params_t {
            compressionLevel: compression_level,
            notificationLevel: 0,
            dictID: 0,
        };

        // SAFETY: All pointers point into Rust-owned, properly sized buffers
        // that outlive the FFI call. ZDICT_finalizeDictionary just reads from
        // the sample/dict buffers and writes into `dict_buffer`.
        let written = unsafe {
            zstd_sys::ZDICT_finalizeDictionary(
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
        // SAFETY: ZDICT_isError just inspects the integer return code.
        if unsafe { zstd_sys::ZDICT_isError(written) } != 0 {
            // SAFETY: ZDICT_getErrorName returns a static NUL-terminated
            // C string from libzstd's internal error table.
            let err_ptr = unsafe { zstd_sys::ZDICT_getErrorName(written) };
            let msg = if err_ptr.is_null() {
                "zstd dictionary finalization failed".to_string()
            } else {
                unsafe { core::ffi::CStr::from_ptr(err_ptr) }
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

    /// get_param_bounds(parameter, /, *, is_compress)
    ///
    /// Return the inclusive `(lower, upper)` integer bounds for a
    /// compression parameter (when *is_compress* is True) or a
    /// decompression parameter (when False). Used by
    /// `CompressionParameter.bounds()` and `DecompressionParameter.bounds()`
    /// in the pure-Python wrapper.
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
        // distinguish a libzstd-reported error from our own "unknown".
        let bounds = if args.is_compress {
            let p = c_param_enum(args.parameter).ok_or_else(unknown)?;
            // SAFETY: `c_param_enum` returned `Some`, so `p` is a real
            // `ZSTD_cParameter` discriminant.
            unsafe { zstd_sys::ZSTD_cParam_getBounds(p) }
        } else {
            let p = d_param_enum(args.parameter).ok_or_else(unknown)?;
            // SAFETY: same as above.
            unsafe { zstd_sys::ZSTD_dParam_getBounds(p) }
        };
        // SAFETY: ZSTD_isError just inspects the integer error code.
        if unsafe { zstd_sys::ZSTD_isError(bounds.error) } != 0 {
            return Err(catch_zstd_error(bounds.error, vm));
        }
        Ok((bounds.lowerBound, bounds.upperBound))
    }

    /// set_parameter_types(c_parameter_type, d_parameter_type, /)
    ///
    /// Validate that the `CompressionParameter` and `DecompressionParameter`
    /// types passed in by the pure-Python wrapper are actual type objects.
    /// The (de)compressor constructors use the *names* of these classes
    /// (see [`check_wrong_param_kind`]) rather than identity, so this
    /// function does not need to retain the type objects beyond the call.
    ///
    /// CPython retains them and uses them for nicer error messages naming
    /// the specific enum member; matching that behavior would require a
    /// `Sync` cell-of-`PyTypeRef`, which is not available on single-threaded
    /// targets. The pure-Python wrapper at
    /// `Lib/compression/zstd/__init__.py:242` calls this exactly once at
    /// import time.
    #[pyfunction]
    fn set_parameter_types(
        c_parameter_type: PyObjectRef,
        d_parameter_type: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let _ = c_parameter_type
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("c_parameter_type must be a type object"))?;
        let _ = d_parameter_type
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("d_parameter_type must be a type object"))?;
        Ok(())
    }
}
