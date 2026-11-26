// spell-checker:ignore cctx dctx CCTX DCTX ldm cdict ddict windowlog hashlog chainlog searchlog
// spell-checker:ignore minmatch dictid checksumflag dstream cstream pyobj zstandard btopt btultra
// spell-checker:ignore btlazy dfast nbworkers windowlogmax windowlog overlap targetcblock
// spell-checker:ignore srcsize zdict refprefix refcdict refddict pledgedsrcsize getframecontentsize
// spell-checker:ignore Zstd Zstandard pylib RFC
// spell-checker:ignore CLEVEL zstdmodule cparameter dparameter c2rust

//! The `_zstd` extension module. Backs the pure-Python `compression.zstd`
//! package by exposing the same classes, functions and constants that
//! CPython's `Modules/_zstd/` exposes. The Python wrapper at
//! `Lib/compression/zstd/__init__.py` imports from this module unconditionally,
//! so the names and call signatures here must stay in sync with CPython.
//!
//! Where a comment names a CPython function, it is the one in `Modules/_zstd`
//! that the code next to it follows. The exceptions raised here carry
//! CPython's types and messages; those raised while converting arguments are
//! worded by RustPython's own argument handling.
//!
//! Backend: the `libzstd-rs-sys` crate, Trifecta Tech Foundation's c2rust
//! translation of Facebook's libzstd (the C library CPython links against),
//! used directly through its `unsafe extern "C"` API surface. It is Rust except
//! for libzstd's x86-64 assembly Huffman decoder, which its build script
//! compiles on x86-64 Unix hosts and the library uses on x86-64 Unix targets.

pub(crate) use _zstd::module_def;

// The parameter and strategy constants keep libzstd's `ZSTD_c_camelCase`
// spelling because `Lib/compression/zstd/__init__.py` reads them from this
// module under those exact names.
#[allow(non_upper_case_globals)]
#[pymodule]
mod _zstd {
    use core::ffi::{CStr, c_int};
    use core::ptr::NonNull;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use libzstd_rs_sys::lib::common::zstd_common::ZSTD_getErrorString;
    use libzstd_rs_sys::lib::compress::zstd_compress::{ZSTD_e_continue, ZSTD_e_end, ZSTD_e_flush};
    use libzstd_rs_sys::lib::zdict::ZDICT_finalizeDictionary;
    use libzstd_rs_sys::lib::zstd::ZSTD_error_parameter_unsupported;
    use libzstd_rs_sys::{
        ZDICT_isError, ZDICT_params_t, ZDICT_trainFromBuffer, ZSTD_CCtx, ZSTD_CCtx_loadDictionary,
        ZSTD_CCtx_refCDict, ZSTD_CCtx_refPrefix, ZSTD_CCtx_reset, ZSTD_CCtx_setParameter,
        ZSTD_CCtx_setPledgedSrcSize, ZSTD_CDict, ZSTD_CONTENTSIZE_ERROR, ZSTD_CONTENTSIZE_UNKNOWN,
        ZSTD_CStreamOutSize, ZSTD_DCtx, ZSTD_DCtx_loadDictionary, ZSTD_DCtx_refDDict,
        ZSTD_DCtx_refPrefix, ZSTD_DCtx_reset, ZSTD_DCtx_setParameter, ZSTD_DDict,
        ZSTD_DStreamOutSize, ZSTD_EndDirective, ZSTD_ResetDirective, ZSTD_cParam_getBounds,
        ZSTD_cParameter, ZSTD_compressBound, ZSTD_compressStream2, ZSTD_createCCtx,
        ZSTD_createCDict, ZSTD_createDCtx, ZSTD_createDDict, ZSTD_dParam_getBounds,
        ZSTD_dParameter, ZSTD_decompressStream, ZSTD_findFrameCompressedSize, ZSTD_freeCCtx,
        ZSTD_freeCDict, ZSTD_freeDCtx, ZSTD_freeDDict, ZSTD_getDictID_fromDict,
        ZSTD_getDictID_fromFrame, ZSTD_getErrorName, ZSTD_getFrameContentSize, ZSTD_inBuffer,
        ZSTD_isError, ZSTD_maxCLevel, ZSTD_minCLevel, ZSTD_outBuffer, ZSTD_versionNumber,
        ZSTD_versionString,
    };
    use rustpython_common::lock::{PyMutex, PyMutexGuard, PyRwLock};
    use rustpython_common::rc::PyRc;
    use rustpython_vm::builtins::{
        PyBaseExceptionRef, PyBytes, PyBytesRef, PyDict, PyInt, PyModule, PyTuple, PyTupleRef,
        PyType, PyTypeRef,
    };
    use rustpython_vm::function::{ArgBytesLike, ArgIntoBool, ItemDoc};
    use rustpython_vm::protocol::PySequenceMethods;
    use rustpython_vm::types::{AsSequence, Constructor, Representable};
    use rustpython_vm::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
    };
    use std::collections::HashMap;

    #[pyattr]
    const ZSTD_CLEVEL_DEFAULT: i32 = libzstd_rs_sys::ZSTD_CLEVEL_DEFAULT;

    /// Read the numeric id back out of a libzstd compression-parameter
    /// constant.
    ///
    /// The ids are needed as plain ints because the module constants below are
    /// what `Lib/compression/zstd/__init__.py` turns into the public
    /// `CompressionParameter` IntEnum. libzstd-rs-sys declares
    /// `ZSTD_cParameter` as `#[repr(transparent)] pub struct
    /// ZSTD_cParameter(pub(crate) u32)` and gives it no accessor, `From` impl
    /// or `Deref`, so short of parsing its derived `Debug` output there is no
    /// other way to ask it for its id. Reading the wrapped `u32` back keeps
    /// libzstd the single source of truth for these values instead of
    /// restating them here as literals.
    const fn c_param_id(param: ZSTD_cParameter) -> i32 {
        // SAFETY: `#[repr(transparent)]` gives `ZSTD_cParameter` the layout of
        // the single `u32` it wraps, so this reads that `u32`.
        unsafe { core::mem::transmute::<ZSTD_cParameter, u32>(param) as i32 }
    }

    /// `ZSTD_dParameter` counterpart of [`c_param_id`]; that type is likewise
    /// a `#[repr(transparent)]` newtype over a private `u32`.
    const fn d_param_id(param: ZSTD_dParameter) -> i32 {
        // SAFETY: as in `c_param_id`.
        unsafe { core::mem::transmute::<ZSTD_dParameter, u32>(param) as i32 }
    }

    // Compression parameter identifiers, taken straight from libzstd's
    // `ZSTD_cParameter` constants. These are the ids the public
    // `CompressionParameter` IntEnum in `Lib/compression/zstd/__init__.py`
    // derives its members from.
    #[pyattr]
    const ZSTD_c_compressionLevel: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_compressionLevel);
    #[pyattr]
    const ZSTD_c_windowLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_windowLog);
    #[pyattr]
    const ZSTD_c_hashLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_hashLog);
    #[pyattr]
    const ZSTD_c_chainLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_chainLog);
    #[pyattr]
    const ZSTD_c_searchLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_searchLog);
    #[pyattr]
    const ZSTD_c_minMatch: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_minMatch);
    #[pyattr]
    const ZSTD_c_targetLength: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_targetLength);
    #[pyattr]
    const ZSTD_c_strategy: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_strategy);
    #[pyattr]
    const ZSTD_c_enableLongDistanceMatching: i32 =
        c_param_id(ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching);
    #[pyattr]
    const ZSTD_c_ldmHashLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_ldmHashLog);
    #[pyattr]
    const ZSTD_c_ldmMinMatch: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_ldmMinMatch);
    #[pyattr]
    const ZSTD_c_ldmBucketSizeLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_ldmBucketSizeLog);
    #[pyattr]
    const ZSTD_c_ldmHashRateLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_ldmHashRateLog);
    #[pyattr]
    const ZSTD_c_contentSizeFlag: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_contentSizeFlag);
    #[pyattr]
    const ZSTD_c_checksumFlag: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_checksumFlag);
    #[pyattr]
    const ZSTD_c_dictIDFlag: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_dictIDFlag);
    #[pyattr]
    const ZSTD_c_nbWorkers: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_nbWorkers);
    #[pyattr]
    const ZSTD_c_jobSize: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_jobSize);
    #[pyattr]
    const ZSTD_c_overlapLog: i32 = c_param_id(ZSTD_cParameter::ZSTD_c_overlapLog);

    // Decompression parameter identifiers. libzstd only exposes one non-
    // experimental decompression parameter.
    #[pyattr]
    const ZSTD_d_windowLogMax: i32 = d_param_id(ZSTD_dParameter::ZSTD_d_windowLogMax);

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

    #[pyattr(once)]
    fn zstd_version(_vm: &VirtualMachine) -> String {
        // SAFETY: `ZSTD_versionString` returns a pointer to libzstd's static,
        // NUL-terminated version string.
        let version = unsafe { CStr::from_ptr(ZSTD_versionString()) };
        // The string is a fixed `MAJOR.MINOR.RELEASE` ASCII literal baked into
        // libzstd, so it is always valid UTF-8 and there is nothing for a
        // lossy conversion to repair.
        version
            .to_str()
            .expect("libzstd version string is ASCII")
            .to_owned()
    }

    #[pyattr(once)]
    fn zstd_version_number(_vm: &VirtualMachine) -> u32 {
        ZSTD_versionNumber()
    }

    #[pyattr(once, name = "ZSTD_DStreamOutSize")]
    fn zstd_dstream_out_size(_vm: &VirtualMachine) -> usize {
        ZSTD_DStreamOutSize()
    }

    // CPython creates this type as `compression.zstd.ZstdError`, the name it
    // is public under, and gives the three classes below the same module.
    #[pyattr(once, name = "ZstdError")]
    fn zstd_error_type(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type_with_doc(
            "compression.zstd",
            "ZstdError",
            None,
            const { ItemDoc::db("_zstd.ZstdError") },
        )
    }

    // Use the cached type directly rather than looking it up through
    // `sys.modules`, so mutating `_zstd.ZstdError` or `sys.modules["_zstd"]`
    // cannot break or panic exception construction.
    fn new_zstd_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let message: String = message.into();
        vm.new_exception_msg(zstd_error_type(vm), message.into())
    }

    /// What was being attempted when libzstd reported an error: CPython's
    /// `error_type`. It selects the sentence that libzstd's own message is
    /// appended to.
    #[derive(Clone, Copy)]
    enum ErrorKind {
        Decompress,
        Compress,
        SetPledgedInputSize,
        LoadDDict,
        LoadCDict,
        GetCBounds,
        GetDBounds,
        SetCLevel,
        TrainDict,
        FinalizeDict,
    }

    /// libzstd's message for an error code.
    fn error_name(code: usize) -> &'static str {
        // SAFETY: `ZSTD_getErrorName` maps every value to one of libzstd's
        // static, NUL-terminated messages.
        let name = unsafe { CStr::from_ptr(ZSTD_getErrorName(code)) };
        // They are ASCII literals compiled into libzstd.
        name.to_str().expect("libzstd error messages are ASCII")
    }

    /// CPython's `set_zstd_error`, for an error libzstd describes as `reason`.
    fn zstd_error_because(
        kind: ErrorKind,
        reason: &str,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let action = match kind {
            ErrorKind::Decompress => "Unable to decompress Zstandard data",
            ErrorKind::Compress => "Unable to compress Zstandard data",
            ErrorKind::SetPledgedInputSize => "Unable to set pledged uncompressed content size",
            ErrorKind::LoadDDict => {
                "Unable to load Zstandard dictionary or prefix for decompression"
            }
            ErrorKind::LoadCDict => "Unable to load Zstandard dictionary or prefix for compression",
            ErrorKind::GetCBounds => "Unable to get zstd compression parameter bounds",
            ErrorKind::GetDBounds => "Unable to get zstd decompression parameter bounds",
            ErrorKind::SetCLevel => "Unable to set zstd compression level",
            ErrorKind::TrainDict => "Unable to train the Zstandard dictionary",
            ErrorKind::FinalizeDict => "Unable to finalize the Zstandard dictionary",
        };
        new_zstd_error(format!("{action}: {reason}"), vm)
    }

    /// CPython's `set_zstd_error`, for the error code a libzstd call returned.
    fn zstd_error(kind: ErrorKind, code: usize, vm: &VirtualMachine) -> PyBaseExceptionRef {
        zstd_error_because(kind, error_name(code), vm)
    }

    /// Split what a libzstd function returned into its value or its error
    /// code.
    fn check(code: usize) -> Result<usize, usize> {
        if ZSTD_isError(code) != 0 {
            Err(code)
        } else {
            Ok(code)
        }
    }

    /// Which half of the module a parameter id or an options dict belongs to.
    /// The two id spaces overlap (`ZSTD_c_compressionLevel` and
    /// `ZSTD_d_windowLogMax` are both 100), so an id says nothing without it.
    #[derive(Clone, Copy)]
    enum Direction {
        Compression,
        Decompression,
    }

    impl Direction {
        const fn noun(self) -> &'static str {
            match self {
                Self::Compression => "compression",
                Self::Decompression => "decompression",
            }
        }
    }

    // CPython's `cp_list` and `dp_list`: the documented parameters exposed by
    // `CompressionParameter` and `DecompressionParameter`, together with the
    // names error messages refer to them by.
    //
    // CPython passes any int key straight on to libzstd, so there an options
    // dict can also pass undocumented ids such as `ZSTD_c_targetCBlockSize` or
    // libzstd's experimental parameters. Here we map only the 19 compression
    // and 1 decompression parameters from `cp_list` and `dp_list` to
    // `libzstd-rs-sys`'s `ZSTD_cParameter` / `ZSTD_dParameter` constants, and
    // report any other key the way CPython reports a key its libzstd does not
    // know.
    const C_PARAMETERS: [(ZSTD_cParameter, &str); 19] = [
        (
            ZSTD_cParameter::ZSTD_c_compressionLevel,
            "compression_level",
        ),
        (ZSTD_cParameter::ZSTD_c_windowLog, "window_log"),
        (ZSTD_cParameter::ZSTD_c_hashLog, "hash_log"),
        (ZSTD_cParameter::ZSTD_c_chainLog, "chain_log"),
        (ZSTD_cParameter::ZSTD_c_searchLog, "search_log"),
        (ZSTD_cParameter::ZSTD_c_minMatch, "min_match"),
        (ZSTD_cParameter::ZSTD_c_targetLength, "target_length"),
        (ZSTD_cParameter::ZSTD_c_strategy, "strategy"),
        (
            ZSTD_cParameter::ZSTD_c_enableLongDistanceMatching,
            "enable_long_distance_matching",
        ),
        (ZSTD_cParameter::ZSTD_c_ldmHashLog, "ldm_hash_log"),
        (ZSTD_cParameter::ZSTD_c_ldmMinMatch, "ldm_min_match"),
        (
            ZSTD_cParameter::ZSTD_c_ldmBucketSizeLog,
            "ldm_bucket_size_log",
        ),
        (ZSTD_cParameter::ZSTD_c_ldmHashRateLog, "ldm_hash_rate_log"),
        (ZSTD_cParameter::ZSTD_c_contentSizeFlag, "content_size_flag"),
        (ZSTD_cParameter::ZSTD_c_checksumFlag, "checksum_flag"),
        (ZSTD_cParameter::ZSTD_c_dictIDFlag, "dict_id_flag"),
        (ZSTD_cParameter::ZSTD_c_nbWorkers, "nb_workers"),
        (ZSTD_cParameter::ZSTD_c_jobSize, "job_size"),
        (ZSTD_cParameter::ZSTD_c_overlapLog, "overlap_log"),
    ];

    const D_PARAMETERS: [(ZSTD_dParameter, &str); 1] =
        [(ZSTD_dParameter::ZSTD_d_windowLogMax, "window_log_max")];

    fn c_parameter(id: i32) -> Option<(ZSTD_cParameter, &'static str)> {
        C_PARAMETERS
            .iter()
            .copied()
            .find(|&(parameter, _)| c_param_id(parameter) == id)
    }

    fn d_parameter(id: i32) -> Option<(ZSTD_dParameter, &'static str)> {
        D_PARAMETERS
            .iter()
            .copied()
            .find(|&(parameter, _)| d_param_id(parameter) == id)
    }

    /// The inclusive range libzstd accepts for a parameter, or the error code
    /// it reports instead.
    type Bounds = Result<(c_int, c_int), usize>;

    fn c_bounds(parameter: ZSTD_cParameter) -> Bounds {
        let bounds = ZSTD_cParam_getBounds(parameter);
        check(bounds.error).map(|_| (bounds.lowerBound, bounds.upperBound))
    }

    // The two `getBounds` functions return distinct (but identically laid out)
    // `ZSTD_bounds` types, hence two functions.
    fn d_bounds(parameter: ZSTD_dParameter) -> Bounds {
        let bounds = ZSTD_dParam_getBounds(parameter);
        check(bounds.error).map(|_| (bounds.lowerBound, bounds.upperBound))
    }

    /// The name and bounds of the parameter `id` stands for, or `None` if it
    /// is not in the table for `direction`.
    fn parameter_info(direction: Direction, id: i32) -> Option<(&'static str, Bounds)> {
        match direction {
            Direction::Compression => {
                c_parameter(id).map(|(parameter, name)| (name, c_bounds(parameter)))
            }
            Direction::Decompression => {
                d_parameter(id).map(|(parameter, name)| (name, d_bounds(parameter)))
            }
        }
    }

    /// CPython's `set_parameter_error`: the `ValueError` for an options-dict
    /// entry that could not be applied.
    fn parameter_error(
        direction: Direction,
        key: i32,
        value: i32,
        vm: &VirtualMachine,
    ) -> PyBaseExceptionRef {
        let kind = direction.noun();
        let message = match parameter_info(direction, key) {
            Some((name, Ok((lower, upper)))) => format!(
                "{kind} parameter '{name}' received an illegal value {value}; \
                 the valid range is [{lower}, {upper}]"
            ),
            Some((name, Err(_))) => format!("invalid {kind} parameter '{name}'"),
            None => format!("invalid {kind} parameter 'unknown parameter (key {key})'"),
        };
        vm.new_value_error(message)
    }

    // Per-module state stored in `PyModule::native_state()`, keeping the enum
    // classes registered by `set_parameter_types` out of `_zstd.__dict__` and
    // isolated per interpreter (the equivalent of CPython's `_zstd_state`).
    #[pyclass(no_attr, module = "_zstd", name = "_zstd_state", traverse)]
    #[derive(Debug, Default, PyPayload)]
    struct ZstdState {
        c_parameter_type: PyRwLock<Option<PyTypeRef>>,
        d_parameter_type: PyRwLock<Option<PyTypeRef>>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl ZstdState {}

    #[expect(clippy::unnecessary_wraps, reason = "Needs to comply with a signature")]
    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        __module_exec(vm, module);
        module.set_native_state(ZstdState::default().into_ref(&vm.ctx).into());
        Ok(())
    }

    fn module_state(
        vm: &VirtualMachine,
        import_if_missing: bool,
    ) -> PyResult<Option<PyRef<ZstdState>>> {
        let modules = vm.sys_module.get_attr("modules", vm)?;
        let module = match modules.get_item("_zstd", vm) {
            Ok(module) => module,
            Err(_) if import_if_missing => vm.import("_zstd", 0)?,
            Err(_) => return Ok(None),
        };
        Ok(module
            .downcast_ref::<PyModule>()
            .and_then(|module| module.native_state())
            .and_then(|state| state.downcast::<ZstdState>().ok()))
    }

    /// Walk an `options` dict the way CPython's `_zstd_set_c_parameters` and
    /// `_zstd_set_d_parameters` do, handing every key and value to `set` as a
    /// C int.
    fn for_each_option(
        options: PyObjectRef,
        direction: Direction,
        vm: &VirtualMachine,
        mut set: impl FnMut(i32, i32) -> PyResult<()>,
    ) -> PyResult<()> {
        let (class_name, other_enum) = match direction {
            Direction::Compression => ("ZstdCompressor", "DecompressionParameter"),
            Direction::Decompression => ("ZstdDecompressor", "CompressionParameter"),
        };
        let options = match options.downcast::<PyDict>() {
            Ok(options) => options,
            Err(options) => {
                let type_name = options.class().fully_qualified_name(vm)?;
                return Err(vm.new_type_error(format!(
                    "{class_name}() argument 'options' must be dict, not {type_name}"
                )));
            }
        };
        // The enum of the other direction, once `compression.zstd` has
        // registered it. Both enums are IntEnums, so a member of the wrong one
        // would otherwise pass for whichever parameter of this direction shares
        // its value. Until they are registered the check is skipped, as it is
        // while CPython's module state still holds NULL.
        let other_type = module_state(vm, false)?.and_then(|state| match direction {
            Direction::Compression => state.d_parameter_type.read().clone(),
            Direction::Decompression => state.c_parameter_type.read().clone(),
        });
        for (key, value) in options {
            if other_type
                .as_ref()
                .is_some_and(|other_type| key.class().is(other_type))
            {
                return Err(vm.new_type_error(format!(
                    "{} options dictionary key must not be a {other_enum} attribute",
                    direction.noun()
                )));
            }
            // Both go through `PyLong_AsInt` in CPython, which takes an int or
            // anything with `__index__`; so does `try_to_value`.
            let key: i32 = key.try_to_value(vm)?;
            let value: i32 = value.try_to_value(vm)?;
            set(key, value)?;
        }
        Ok(())
    }

    /// Which of the three load modes a `zstd_dict=` argument asks for. The
    /// `ZstdDict.as_*` properties pair the dictionary with the matching
    /// discriminant in a `(zdict, marker)` tuple, and the compressor and
    /// decompressor constructors turn that marker back into this enum.
    ///
    /// The discriminants are pinned because they cross the Python boundary as
    /// plain ints: they are the values of CPython's `dictionary_type` enum in
    /// `Modules/_zstd/_zstdmodule.h`. `test_zstd` pins the accepted range from
    /// the outside, hand-building `(zd, -1)` and `(zd, 3)` tuples and
    /// requiring both to raise.
    #[derive(Copy, Clone)]
    enum DictType {
        Digested = 0,
        Undigested = 1,
        Prefix = 2,
    }

    impl DictType {
        /// Decode the int half of a `(zdict, marker)` tuple, which may come
        /// straight from user code. `None` for anything outside the three
        /// documented markers.
        fn from_marker(marker: i32) -> Option<Self> {
            [Self::Digested, Self::Undigested, Self::Prefix]
                .into_iter()
                .find(|&dict_type| dict_type as i32 == marker)
        }
    }

    /// CPython's `_Py_parse_zstd_dict`: decode the `zstd_dict=` constructor
    /// argument. A `ZstdDict` takes the caller's `default` load mode; the
    /// `(ZstdDict, marker)` tuples that the `ZstdDict.as_*` properties return
    /// name theirs.
    fn parse_zstd_dict_arg(
        obj: PyObjectRef,
        default: DictType,
        vm: &VirtualMachine,
    ) -> PyResult<(PyRef<ZstdDict>, DictType)> {
        let obj = match obj.downcast::<ZstdDict>() {
            Ok(zdict) => return Ok((zdict, default)),
            Err(obj) => obj,
        };
        // CPython takes exactly a 2-tuple here (`PyTuple_CheckExact`) whose
        // second item is an int (`PyLong_Check`): no tuple subclass, and no
        // `__index__` for the marker.
        if let Some(tuple) = obj.downcast_ref_if_exact::<PyTuple>(vm)
            && let [zdict, marker] = tuple.as_slice()
            && let Some(zdict) = zdict.downcast_ref::<ZstdDict>()
            && let Some(marker) = marker.downcast_ref::<PyInt>()
        {
            // An int too large for a C int is reported as such.
            let marker: i32 = marker.try_to_primitive(vm)?;
            if let Some(dict_type) = DictType::from_marker(marker) {
                return Ok((zdict.to_owned(), dict_type));
            }
        }
        Err(vm.new_type_error("zstd_dict argument should be a ZstdDict object."))
    }

    /// Take a lock that another thread may be holding across `allow_threads`.
    ///
    /// Compression and decompression detach from the VM while they hold their
    /// state lock, and a digested dictionary is built detached under its cache
    /// lock. A thread that waited for one of these locks while attached would
    /// reach no safepoint for as long as it waited, so a stop-the-world
    /// request would have to wait with it. It detaches for the wait instead.
    /// That is safe for these locks because no stop-the-world section takes
    /// them: none of the payloads holding one is traversed by the collector.
    /// `RawDetachingRwLock` in `rustpython_common::lock` has the full argument.
    fn lock_detached<'a, T>(lock: &'a PyMutex<T>, vm: &VirtualMachine) -> PyMutexGuard<'a, T> {
        vm.allow_threads(|| lock.lock())
    }

    /// Owning wrapper around a digested compression dictionary. Frees the
    /// dictionary with `ZSTD_freeCDict` on drop.
    struct CDict(NonNull<ZSTD_CDict>);

    // SAFETY: a `ZSTD_CDict` is not tied to the thread that created it, and
    // once `ZSTD_createCDict` has returned libzstd only reads it, so contexts
    // on several threads may use the same one at once. zstd.h says as much:
    // "ZSTD_CDict can be created once and shared by multiple threads
    // concurrently, since its usage is read-only."
    unsafe impl Send for CDict {}
    unsafe impl Sync for CDict {}

    impl CDict {
        /// Digest `bytes` for compression at `level`; libzstd copies the bytes
        /// into the dictionary. `None` when libzstd cannot build one: it is
        /// out of memory, or the content is laid out as a standard-format
        /// dictionary but is corrupted.
        fn create(bytes: &[u8], level: c_int) -> Option<Self> {
            // SAFETY: the pointer and length are those of a live slice.
            let cdict = unsafe { ZSTD_createCDict(bytes.as_ptr().cast(), bytes.len(), level) };
            NonNull::new(cdict).map(Self)
        }
    }

    impl Drop for CDict {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createCDict` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeCDict(self.0.as_ptr()) };
        }
    }

    /// Owning wrapper around a digested decompression dictionary. Frees the
    /// dictionary with `ZSTD_freeDDict` on drop.
    struct DDict(NonNull<ZSTD_DDict>);

    // SAFETY: as for `CDict`. libzstd takes a `ZSTD_DDict` by `*const`
    // everywhere except in `ZSTD_freeDDict`, and CPython shares one between
    // all the decompressors of a `ZstdDict` in the same way.
    unsafe impl Send for DDict {}
    unsafe impl Sync for DDict {}

    impl DDict {
        /// See [`CDict::create`].
        fn create(bytes: &[u8]) -> Option<Self> {
            // SAFETY: the pointer and length are those of a live slice.
            let ddict = unsafe { ZSTD_createDDict(bytes.as_ptr().cast(), bytes.len()) };
            NonNull::new(ddict).map(Self)
        }
    }

    impl Drop for DDict {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createDDict` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeDDict(self.0.as_ptr()) };
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdDictArgs {
        #[pyarg(positional)]
        dict_content: ArgBytesLike,
        #[pyarg(named, default = ArgIntoBool::FALSE, py_default = "False")]
        is_raw: ArgIntoBool,
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdDict")]
    #[derive(PyPayload)]
    struct ZstdDict {
        dict_content: PyBytesRef,
        #[pymember]
        dict_id: u32,
        // The digested forms of this dictionary, built the first time a
        // compressor or decompressor asks for one and shared by all of them
        // from then on: CPython's `c_dicts`, one per compression level, and
        // `d_dict`. Digesting is the expensive part of loading a dictionary,
        // and sparing every later user that cost is what `as_digested_dict`
        // is for.
        c_dicts: PyMutex<HashMap<c_int, PyRc<CDict>>>,
        d_dict: PyMutex<Option<PyRc<DDict>>>,
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

    impl ZstdDict {
        /// CPython's `_get_CDict`: this dictionary digested for compression at
        /// `level`.
        fn c_dict(&self, level: c_int, vm: &VirtualMachine) -> PyResult<PyRc<CDict>> {
            let created = {
                let mut c_dicts = lock_detached(&self.c_dicts, vm);
                if let Some(c_dict) = c_dicts.get(&level) {
                    return Ok(c_dict.clone());
                }
                let bytes = self.dict_content.as_bytes();
                let created = vm
                    .allow_threads(|| CDict::create(bytes, level))
                    .map(PyRc::new);
                if let Some(c_dict) = &created {
                    c_dicts.insert(level, c_dict.clone());
                }
                created
            };
            created.ok_or_else(|| {
                new_zstd_error(
                    "Failed to create a ZSTD_CDict instance from Zstandard dictionary content.",
                    vm,
                )
            })
        }

        /// CPython's `_get_DDict`: this dictionary digested for decompression.
        fn d_dict(&self, vm: &VirtualMachine) -> PyResult<PyRc<DDict>> {
            let created = {
                let mut d_dict = lock_detached(&self.d_dict, vm);
                if let Some(d_dict) = &*d_dict {
                    return Ok(d_dict.clone());
                }
                let bytes = self.dict_content.as_bytes();
                let created = vm.allow_threads(|| DDict::create(bytes)).map(PyRc::new);
                d_dict.clone_from(&created);
                created
            };
            created.ok_or_else(|| {
                new_zstd_error(
                    "Failed to create a ZSTD_DDict instance from Zstandard dictionary content.",
                    vm,
                )
            })
        }

        fn with_marker(zelf: PyRef<Self>, dict_type: DictType, vm: &VirtualMachine) -> PyTupleRef {
            vm.ctx
                .new_tuple(vec![zelf.into(), vm.ctx.new_int(dict_type as i32).into()])
        }
    }

    impl Constructor for ZstdDict {
        type Args = ZstdDictArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.dict_content.len() < 8 {
                return Err(vm.new_value_error(
                    "Zstandard dictionary content too short \
                     (must have at least eight bytes)",
                ));
            }
            let dict_content = if let Some(bytes) = args
                .dict_content
                .as_object()
                .downcast_ref_if_exact::<PyBytes>(vm)
            {
                bytes.to_owned()
            } else {
                let copy = args.dict_content.with_ref(|content| {
                    let mut buf = Vec::new();
                    buf.try_reserve_exact(content.len())
                        .map_err(|_| vm.no_memory_error())?;
                    buf.extend_from_slice(content);
                    Ok(buf)
                })?;
                vm.ctx.new_bytes(copy)
            };
            // `ZSTD_getDictID_fromDict` returns 0 unless the content starts
            // with the dictionary magic number followed by a non-zero id, that
            // is, unless it is a dictionary in the standard format. Only a
            // "raw content" dictionary may be anything else.
            let bytes = dict_content.as_bytes();
            // SAFETY: the pointer and length are those of a live slice.
            let dict_id = unsafe { ZSTD_getDictID_fromDict(bytes.as_ptr().cast(), bytes.len()) };
            if dict_id == 0 && !args.is_raw.into_bool() {
                return Err(vm.new_value_error("invalid Zstandard dictionary"));
            }
            Ok(Self {
                dict_content,
                dict_id,
                c_dicts: PyMutex::default(),
                d_dict: PyMutex::default(),
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

    impl AsSequence for ZstdDict {
        fn as_sequence() -> &'static PySequenceMethods {
            static AS_SEQUENCE: PySequenceMethods = PySequenceMethods {
                length: atomic_func!(|seq, _vm| {
                    Ok(ZstdDict::sequence_downcast(seq).dict_content.len())
                }),
                ..PySequenceMethods::NOT_IMPLEMENTED
            };
            &AS_SEQUENCE
        }
    }

    // CPython creates all three classes without `Py_TPFLAGS_BASETYPE` and with
    // (or, for the compressor, frozen to) `Py_TPFLAGS_IMMUTABLETYPE`.
    #[pyclass(with(Constructor, Representable, AsSequence), flags(IMMUTABLETYPE))]
    impl ZstdDict {
        #[pygetset]
        fn dict_content(zelf: &Py<Self>) -> PyBytesRef {
            zelf.dict_content.clone()
        }

        #[pygetset]
        fn as_digested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::with_marker(zelf, DictType::Digested, vm)
        }

        #[pygetset]
        fn as_undigested_dict(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::with_marker(zelf, DictType::Undigested, vm)
        }

        #[pygetset]
        fn as_prefix(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
            Self::with_marker(zelf, DictType::Prefix, vm)
        }
    }

    /// The flush modes `ZstdCompressor.compress()` and
    /// `ZstdCompressor.flush()` accept, mirrored as the class attributes
    /// `CONTINUE`, `FLUSH_BLOCK` and `FLUSH_FRAME` by `extend_class` below.
    /// They are libzstd's end directives under the names CPython gives them.
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum CompressMode {
        Continue = ZSTD_e_continue as isize,
        FlushBlock = ZSTD_e_flush as isize,
        FlushFrame = ZSTD_e_end as isize,
    }

    impl CompressMode {
        /// Decode a user-supplied `mode=` argument. `None` for anything
        /// outside the three documented modes.
        fn from_int(mode: i32) -> Option<Self> {
            [Self::Continue, Self::FlushBlock, Self::FlushFrame]
                .into_iter()
                .find(|&known| known as i32 == mode)
        }
    }

    /// Why a streaming call stopped short. Plain data rather than an
    /// exception, because the calls run detached under `allow_threads` where
    /// VM operations cannot happen, and exceptions are built after the lock is
    /// released.
    enum StreamError {
        /// libzstd returned this error code.
        Zstd(usize),
        /// The output buffer could not be grown.
        NoMemory,
    }

    impl StreamError {
        fn into_exception(self, kind: ErrorKind, vm: &VirtualMachine) -> PyBaseExceptionRef {
            match self {
                Self::Zstd(code) => zstd_error(kind, code, vm),
                Self::NoMemory => vm.no_memory_error(),
            }
        }
    }

    /// Owning wrapper around a libzstd compression context. Frees the context
    /// with `ZSTD_freeCCtx` on drop, once its worker threads are idle.
    struct CCtx {
        ptr: NonNull<ZSTD_CCtx>,
        /// Whether `nb_workers` has been set to a non-zero value: CPython's
        /// `use_multithread`.
        multithreaded: bool,
        /// Whether worker threads may still be compressing data they were
        /// handed: CPython's `mt_maybe_busy`.
        workers_maybe_busy: bool,
    }

    // SAFETY: a libzstd context is not tied to the thread that created it: it
    // keeps no thread-local state, and may be used and freed from any thread
    // as long as only one uses it at a time (zstd.h: "Use one context per
    // thread for parallel execution"). `CCtx` is not `Sync`, so moving it to
    // another thread is all the sharing there can be.
    unsafe impl Send for CCtx {}

    impl CCtx {
        fn new(vm: &VirtualMachine) -> PyResult<Self> {
            // SAFETY: `ZSTD_createCCtx` has no preconditions; it returns NULL
            // when it cannot allocate the context.
            let ptr = NonNull::new(unsafe { ZSTD_createCCtx() })
                .ok_or_else(|| new_zstd_error("Unable to create ZSTD_CCtx instance.", vm))?;
            Ok(Self {
                ptr,
                multithreaded: false,
                workers_maybe_busy: false,
            })
        }

        /// Hand a parameter to libzstd, which checks or clamps the value
        /// itself. `Err` carries its error code.
        fn set_parameter(&mut self, parameter: ZSTD_cParameter, value: c_int) -> Result<(), usize> {
            // SAFETY: `self.ptr` is a live context owned by this wrapper.
            check(unsafe { ZSTD_CCtx_setParameter(self.ptr.as_ptr(), parameter, value) })?;
            if parameter == ZSTD_cParameter::ZSTD_c_nbWorkers {
                // libzstd clamps negative `nbWorkers` to 0 (single-threaded),
                // so worker threads are only used when `value > 0`.
                self.multithreaded = value > 0;
            }
            Ok(())
        }

        /// CPython's `_zstd_set_c_level`.
        fn set_level(&mut self, level: c_int, vm: &VirtualMachine) -> PyResult<()> {
            let (min, max) = (ZSTD_minCLevel(), ZSTD_maxCLevel());
            // libzstd clamps out-of-range compression levels (as well as
            // `nbWorkers`, `jobSize`, and `overlapLog`); like CPython, this
            // module explicitly enforces the range for `compression_level`.
            if !(min..=max).contains(&level) {
                return Err(vm.new_value_error(format!(
                    "illegal compression level {level}; the valid range is [{min}, {max}]"
                )));
            }
            self.set_parameter(ZSTD_cParameter::ZSTD_c_compressionLevel, level)
                .map_err(|code| zstd_error(ErrorKind::SetCLevel, code, vm))
        }

        /// Return the context to a frame boundary, discarding a partially
        /// written frame but keeping parameters and dictionary. For a
        /// session-only reset libzstd reports no errors.
        fn reset_session(&mut self) {
            // SAFETY: `self.ptr` is a live context owned by this wrapper.
            unsafe {
                ZSTD_CCtx_reset(
                    self.ptr.as_ptr(),
                    ZSTD_ResetDirective::ZSTD_reset_session_only,
                )
            };
        }

        /// Announce the size of the next frame's content, or
        /// `ZSTD_CONTENTSIZE_UNKNOWN`. `Err` carries libzstd's error code.
        fn set_pledged_src_size(&mut self, pledged: u64) -> Result<(), usize> {
            // SAFETY: `self.ptr` is a live context owned by this wrapper.
            check(unsafe { ZSTD_CCtx_setPledgedSrcSize(self.ptr.as_ptr(), pledged) }).map(drop)
        }

        /// CPython's `compress_lock_held` and `compress_mt_continue_lock_held`:
        /// feed `data` to libzstd under `mode` and collect what it produces.
        fn compress(&mut self, data: &[u8], mode: CompressMode) -> Result<Vec<u8>, StreamError> {
            // With worker threads and no flush requested, libzstd only queues
            // the input and returns whatever output happens to be ready, so
            // the call is over as soon as all of the input has been taken. In
            // every other case libzstd works synchronously and says, by
            // returning 0, when it has nothing more to hand over.
            let asynchronous = self.multithreaded && mode == CompressMode::Continue;
            let block = ZSTD_CStreamOutSize();
            // Room enough for libzstd to finish in one call when it works
            // synchronously, as CPython provides. An input too large to have a
            // bound makes this an error code, which no allocation can satisfy.
            let mut room = if asynchronous {
                block
            } else {
                ZSTD_compressBound(data.len())
            };
            let mut output: Vec<u8> = Vec::new();
            let mut input = ZSTD_inBuffer {
                src: data.as_ptr().cast(),
                size: data.len(),
                pos: 0,
            };
            loop {
                output
                    .try_reserve(room)
                    .map_err(|_| StreamError::NoMemory)?;
                let spare = output.spare_capacity_mut();
                let mut out = ZSTD_outBuffer {
                    dst: spare.as_mut_ptr().cast(),
                    size: spare.len(),
                    pos: 0,
                };
                // SAFETY: `self.ptr` is a live context; `out` describes the
                // spare capacity of `output` and `input` describes `data`,
                // both of which outlive the call.
                let remaining = check(unsafe {
                    ZSTD_compressStream2(
                        self.ptr.as_ptr(),
                        &mut out,
                        &mut input,
                        mode as ZSTD_EndDirective,
                    )
                })
                .map_err(StreamError::Zstd)?;
                debug_assert!(out.pos <= out.size);
                // SAFETY: libzstd filled the first `out.pos` bytes of the
                // spare capacity.
                unsafe { output.set_len(output.len() + out.pos) };
                let finished = if asynchronous {
                    out.pos < out.size && input.pos == input.size
                } else {
                    remaining == 0
                };
                if finished {
                    output.shrink_to_fit();
                    return Ok(output);
                }
                room = block;
            }
        }
    }

    impl Drop for CCtx {
        fn drop(&mut self) {
            if self.workers_maybe_busy {
                // `ZSTD_freeCCtx` frees the context's own copy of a dictionary
                // loaded with `ZSTD_CCtx_loadDictionary` before it stops the
                // worker threads, which may still be reading it. Starting a
                // new frame makes libzstd wait for the jobs of the previous
                // one, and with no input it starts none of its own. This is
                // CPython's workaround in `ZstdCompressor_dealloc` (gh-160549).
                let mut input = ZSTD_inBuffer {
                    src: NonNull::<u8>::dangling().as_ptr().cast(),
                    size: 0,
                    pos: 0,
                };
                let mut out = ZSTD_outBuffer {
                    dst: NonNull::<u8>::dangling().as_ptr().cast(),
                    size: 0,
                    pos: 0,
                };
                self.reset_session();
                // SAFETY: `self.ptr` is a live context, and both buffers are
                // empty, so libzstd reads and writes nothing through them.
                // Whether the call succeeds does not matter: its only purpose
                // is the wait it starts with.
                unsafe {
                    ZSTD_compressStream2(self.ptr.as_ptr(), &mut out, &mut input, ZSTD_e_continue)
                };
            }
            // SAFETY: `self.ptr` was created by `ZSTD_createCCtx` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeCCtx(self.ptr.as_ptr()) };
        }
    }

    /// Owning wrapper around a libzstd decompression context. Frees the
    /// context with `ZSTD_freeDCtx` on drop.
    struct DCtx(NonNull<ZSTD_DCtx>);

    // SAFETY: as for `CCtx`.
    unsafe impl Send for DCtx {}

    impl DCtx {
        fn new(vm: &VirtualMachine) -> PyResult<Self> {
            // SAFETY: `ZSTD_createDCtx` has no preconditions; it returns NULL
            // when it cannot allocate the context.
            NonNull::new(unsafe { ZSTD_createDCtx() })
                .map(Self)
                .ok_or_else(|| new_zstd_error("Unable to create ZSTD_DCtx instance.", vm))
        }

        /// See [`CCtx::set_parameter`].
        fn set_parameter(&mut self, parameter: ZSTD_dParameter, value: c_int) -> Result<(), usize> {
            // SAFETY: `self.0` is a live context owned by this wrapper.
            check(unsafe { ZSTD_DCtx_setParameter(self.0.as_ptr(), parameter, value) }).map(drop)
        }

        /// See [`CCtx::reset_session`].
        fn reset_session(&mut self) {
            // SAFETY: `self.0` is a live context owned by this wrapper.
            unsafe {
                ZSTD_DCtx_reset(
                    self.0.as_ptr(),
                    ZSTD_ResetDirective::ZSTD_reset_session_only,
                )
            };
        }

        /// CPython's `decompress_lock_held`: decompress from `input` until the
        /// frame ends, the output holds `max_length` bytes, or libzstd has
        /// used up the input and has nothing left to hand over.
        ///
        /// Returns the output, how much of `input` was consumed, and whether
        /// the frame ended.
        fn decompress(
            &mut self,
            input: &[u8],
            max_length: Option<usize>,
        ) -> Result<(Vec<u8>, usize, bool), StreamError> {
            let block = ZSTD_DStreamOutSize();
            let mut output: Vec<u8> = Vec::new();
            let mut in_buf = ZSTD_inBuffer {
                src: input.as_ptr().cast(),
                size: input.len(),
                pos: 0,
            };
            loop {
                // What libzstd may write in this call: a block's worth, cut
                // down to what `max_length` still allows. For `max_length ==
                // 0` that is nothing at all, which is still worth a call:
                // libzstd consumes input without output to show for it, and
                // that is all a frame without content needs in order to end.
                let room = max_length.map_or(block, |max| (max - output.len()).min(block));
                output
                    .try_reserve(room)
                    .map_err(|_| StreamError::NoMemory)?;
                let mut out = ZSTD_outBuffer {
                    dst: output.spare_capacity_mut().as_mut_ptr().cast(),
                    size: room,
                    pos: 0,
                };
                // SAFETY: `self.0` is a live context; `out` describes `room`
                // bytes of the spare capacity of `output`, which holds at
                // least that many, and `in_buf` describes `input`; both
                // outlive the call.
                let remaining =
                    check(unsafe { ZSTD_decompressStream(self.0.as_ptr(), &mut out, &mut in_buf) })
                        .map_err(StreamError::Zstd)?;
                debug_assert!(out.pos <= out.size);
                // SAFETY: libzstd filled the first `out.pos` bytes of the
                // spare capacity.
                unsafe { output.set_len(output.len() + out.pos) };
                let frame_ended = remaining == 0;
                // A full buffer means libzstd may be holding more output back,
                // so that is looked at before the input: stop only if the
                // caller wants no more, otherwise offer more room.
                let stop = if out.pos == out.size {
                    max_length == Some(output.len())
                } else {
                    in_buf.pos == in_buf.size
                };
                if frame_ended || stop {
                    output.shrink_to_fit();
                    return Ok((output, in_buf.pos, frame_ended));
                }
            }
        }
    }

    impl Drop for DCtx {
        fn drop(&mut self) {
            // SAFETY: `self.0` was created by `ZSTD_createDCtx` and is freed
            // exactly once, here.
            unsafe { ZSTD_freeDCtx(self.0.as_ptr()) };
        }
    }

    /// A compression context together with what it holds raw pointers into.
    struct CompressorState {
        // Declared before the two fields below so that it is dropped before
        // them (fields drop in declaration order): `cctx` may point at
        // `c_dict` or into the bytes of `dict`, and its worker threads may be
        // reading either until the context has been freed.
        cctx: CCtx,
        /// The digested dictionary given to `ZSTD_CCtx_refCDict`, if any.
        c_dict: Option<PyRc<CDict>>,
        /// The dictionary whose bytes were given to `ZSTD_CCtx_refPrefix`, if
        /// any. It is kept for the other two load modes as well, as CPython
        /// keeps `self->dict`.
        dict: Option<PyRef<ZstdDict>>,
        last_mode: CompressMode,
    }

    impl CompressorState {
        /// CPython's `_zstd_load_c_dict`: attach `zstd_dict`, if there is one,
        /// to a context whose parameters are already set, `level` being the
        /// compression level among them.
        fn new(
            cctx: CCtx,
            zstd_dict: Option<PyObjectRef>,
            level: c_int,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            let mut state = Self {
                cctx,
                c_dict: None,
                dict: None,
                last_mode: CompressMode::FlushFrame,
            };
            let Some(zstd_dict) = zstd_dict else {
                return Ok(state);
            };
            // A compressor loads a plain `ZstdDict` undigested, which, unlike
            // the digested form, leaves the compressor's parameters alone.
            let (zdict, dict_type) = parse_zstd_dict_arg(zstd_dict, DictType::Undigested, vm)?;
            // Whatever libzstd is about to be given a pointer to goes into
            // `state` first, so that from the moment it holds the pointer the
            // target can only be dropped together with, and after, `cctx`.
            let zdict = state.dict.insert(zdict);
            let bytes = zdict.dict_content.as_bytes();
            let cctx = state.cctx.ptr.as_ptr();
            let code = match dict_type {
                DictType::Digested => {
                    let c_dict = state.c_dict.insert(zdict.c_dict(level, vm)?);
                    // SAFETY: both are live. `cctx` keeps the pointer, and
                    // `state.c_dict` keeps what it points to alive for it.
                    unsafe { ZSTD_CCtx_refCDict(cctx, c_dict.0.as_ptr()) }
                }
                // SAFETY: `cctx` is live and `bytes` is a live slice that
                // libzstd copies.
                DictType::Undigested => unsafe {
                    ZSTD_CCtx_loadDictionary(cctx, bytes.as_ptr().cast(), bytes.len())
                },
                // SAFETY: `cctx` is live. It keeps the pointer for the next
                // frame; `state.dict` keeps the bytes alive for it, and a
                // `bytes` object never changes or moves its content.
                DictType::Prefix => unsafe {
                    ZSTD_CCtx_refPrefix(cctx, bytes.as_ptr().cast(), bytes.len())
                },
            };
            check(code).map_err(|code| zstd_error(ErrorKind::LoadCDict, code, vm))?;
            Ok(state)
        }

        /// The body of `compress()` and `flush()` once the lock is held.
        fn compress(
            &mut self,
            data: &[u8],
            mode: CompressMode,
            vm: &VirtualMachine,
        ) -> Result<Vec<u8>, StreamError> {
            // Nothing in the closure touches the VM: it works on this state,
            // which the caller has locked, on `data`, which the caller
            // borrowed in a form that stays readable while detached, and on a
            // buffer of its own. The one Python object libzstd may read from
            // is the `bytes` of a dictionary that `self.dict` keeps alive.
            let result = vm.allow_threads(|| self.cctx.compress(data, mode));
            // A failed call leaves the context mid-frame, where libzstd would
            // refuse everything that follows; CPython drops the frame.
            self.last_mode = if result.is_ok() {
                mode
            } else {
                self.cctx.reset_session();
                CompressMode::FlushFrame
            };
            // Only a call that ended a frame is known to have waited for the
            // worker threads.
            self.cctx.workers_maybe_busy =
                self.cctx.multithreaded && !(result.is_ok() && mode == CompressMode::FlushFrame);
            result
        }
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdCompressor")]
    #[derive(PyPayload)]
    struct ZstdCompressor {
        state: PyMutex<CompressorState>,
        #[pymember]
        last_mode: AtomicI32,
    }

    impl core::fmt::Debug for ZstdCompressor {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "_zstd.ZstdCompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdCompressorArgs {
        #[pyarg(any, default = None)]
        level: Option<PyObjectRef>,
        #[pyarg(any, default = None)]
        options: Option<PyObjectRef>,
        #[pyarg(any, default = None)]
        zstd_dict: Option<PyObjectRef>,
    }

    impl Constructor for ZstdCompressor {
        type Args = ZstdCompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let mut cctx = CCtx::new(vm)?;

            if args.level.is_some() && args.options.is_some() {
                return Err(vm.new_type_error("Only one of level or options should be used."));
            }

            // CPython's `self->compression_level`: the level libzstd ends up
            // compressing at, which a digested dictionary has to be built for.
            let mut level = ZSTD_CLEVEL_DEFAULT;

            if let Some(level_obj) = args.level {
                // Unlike the entries of an options dict, `level` has to be an
                // int (`PyLong_Check`): `__index__` is not consulted.
                let level_int = level_obj
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error("invalid type for level, expected int"))?;
                level = level_int.try_to_primitive(vm).map_err(|_| {
                    vm.new_value_error(format!(
                        "illegal compression level; the valid range is [{}, {}]",
                        ZSTD_minCLevel(),
                        ZSTD_maxCLevel()
                    ))
                })?;
                cctx.set_level(level, vm)?;
            }

            if let Some(options) = args.options {
                for_each_option(options, Direction::Compression, vm, |key, value| {
                    if key == ZSTD_c_compressionLevel {
                        cctx.set_level(value, vm)?;
                        level = value;
                        return Ok(());
                    }
                    let accepted = c_parameter(key)
                        .is_some_and(|(parameter, _)| cctx.set_parameter(parameter, value).is_ok());
                    if accepted {
                        Ok(())
                    } else {
                        Err(parameter_error(Direction::Compression, key, value, vm))
                    }
                })?;
            }

            let state = CompressorState::new(cctx, args.zstd_dict, level, vm)?;
            let last_mode = AtomicI32::new(state.last_mode as i32);
            Ok(Self {
                state: PyMutex::new(state),
                last_mode,
            })
        }
    }

    #[derive(FromArgs)]
    pub(super) struct CompressArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(
            any,
            default = CompressMode::Continue as i32,
            py_default = "ZstdCompressor.CONTINUE"
        )]
        mode: i32,
    }

    #[derive(FromArgs)]
    pub(super) struct FlushArgs {
        #[pyarg(
            any,
            default = CompressMode::FlushFrame as i32,
            py_default = "ZstdCompressor.FLUSH_FRAME"
        )]
        mode: i32,
    }

    #[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
    impl ZstdCompressor {
        // Shared by `compress()` and `flush()`. The lock is released before
        // building any exception.
        fn run(&self, data: &[u8], mode: CompressMode, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let result = {
                let mut state = lock_detached(&self.state, vm);
                let result = state.compress(data, mode, vm);
                self.last_mode
                    .store(state.last_mode as i32, Ordering::Release);
                result
            };
            result.map_err(|error| error.into_exception(ErrorKind::Compress, vm))
        }

        #[pymethod]
        fn compress(zelf: &Py<Self>, args: CompressArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mode = CompressMode::from_int(args.mode).ok_or_else(|| {
                vm.new_value_error(
                    "mode argument wrong value, it should be one of \
                     ZstdCompressor.CONTINUE, ZstdCompressor.FLUSH_BLOCK, \
                     ZstdCompressor.FLUSH_FRAME.",
                )
            })?;
            // Borrowed if the bytes cannot change under a detached thread,
            // copied otherwise.
            let data = args.data.borrow_buf_unlocked(vm)?;
            zelf.run(&data, mode, vm)
        }

        #[pymethod]
        fn flush(zelf: &Py<Self>, args: FlushArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mode = match CompressMode::from_int(args.mode) {
                Some(mode @ (CompressMode::FlushBlock | CompressMode::FlushFrame)) => mode,
                _ => {
                    return Err(vm.new_value_error(
                        "mode argument wrong value, it should be \
                         ZstdCompressor.FLUSH_FRAME or \
                         ZstdCompressor.FLUSH_BLOCK.",
                    ));
                }
            };
            zelf.run(&[], mode, vm)
        }

        #[pymethod]
        fn set_pledged_input_size(
            zelf: &Py<Self>,
            size: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            // CPython's `zstd_contentsize_converter`. `None` says the size is
            // unknown; anything else has to be an int (`__index__` is not
            // consulted) below the two values at the top of the range that
            // libzstd reserves for "unknown" and "error".
            let pledged = if vm.is_none(&size) {
                ZSTD_CONTENTSIZE_UNKNOWN
            } else {
                let size = size
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| vm.new_type_error("an integer is required"))?;
                match size.try_to_primitive::<u64>(vm) {
                    Ok(size) if size < ZSTD_CONTENTSIZE_ERROR => size,
                    _ => {
                        return Err(vm.new_value_error(format!(
                            "size argument should be a positive int less than \
                             {ZSTD_CONTENTSIZE_ERROR}"
                        )));
                    }
                }
            };
            let result = {
                let mut state = lock_detached(&zelf.state, vm);
                if state.last_mode != CompressMode::FlushFrame {
                    return Err(vm.new_value_error(
                        "set_pledged_input_size() method must be called \
                         when last_mode == FLUSH_FRAME",
                    ));
                }
                state.cctx.set_pledged_src_size(pledged)
            };
            result.map_err(|code| zstd_error(ErrorKind::SetPledgedInputSize, code, vm))
        }

        // Install class-level constants `CONTINUE`, `FLUSH_BLOCK`, and
        // `FLUSH_FRAME` so callers can reference them as
        // `ZstdCompressor.FLUSH_FRAME` (as the Python `ZstdFile` wrapper
        // does).
        #[extend_class]
        fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
            for (name, mode) in [
                ("CONTINUE", CompressMode::Continue),
                ("FLUSH_BLOCK", CompressMode::FlushBlock),
                ("FLUSH_FRAME", CompressMode::FlushFrame),
            ] {
                class.set_attr(ctx.intern_str(name), ctx.new_int(mode as i32).into());
            }
        }
    }

    /// A decompression context together with what it holds raw pointers into,
    /// and the bookkeeping of CPython's `ZstdDecompressor` struct.
    struct DecompressorState {
        // Declared before the two fields below so that it is dropped before
        // them, for the reason given in `CompressorState`.
        dctx: DCtx,
        /// The digested dictionary given to `ZSTD_DCtx_refDDict`, if any.
        d_dict: Option<PyRc<DDict>>,
        /// The dictionary whose bytes were given to `ZSTD_DCtx_refPrefix`, if
        /// any; kept for the other two load modes as well.
        dict: Option<PyRef<ZstdDict>>,
        /// Whether the end of the frame has been reached. This decompressor
        /// handles a single frame.
        eof: bool,
        needs_input: bool,
        /// Input that a call was given but could not consume, from
        /// `pending_start` on. Either that range is not empty or the buffer
        /// is: CPython's `input_buffer`, `in_begin` and `in_end`.
        pending: Vec<u8>,
        pending_start: usize,
        /// What `unused_data` returned once the frame had ended, so that it
        /// is only built once.
        unused_data: Option<PyBytesRef>,
    }

    impl DecompressorState {
        /// CPython's `_zstd_load_d_dict`: attach `zstd_dict`, if there is one,
        /// to a new context.
        fn new(dctx: DCtx, zstd_dict: Option<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Self> {
            let mut state = Self {
                dctx,
                d_dict: None,
                dict: None,
                eof: false,
                needs_input: true,
                pending: Vec::new(),
                pending_start: 0,
                unused_data: None,
            };
            let Some(zstd_dict) = zstd_dict else {
                return Ok(state);
            };
            // A decompressor loads a plain `ZstdDict` digested.
            let (zdict, dict_type) = parse_zstd_dict_arg(zstd_dict, DictType::Digested, vm)?;
            // Owned by `state` before libzstd gets a pointer to it, as in
            // `CompressorState::new`.
            let zdict = state.dict.insert(zdict);
            let bytes = zdict.dict_content.as_bytes();
            let dctx = state.dctx.0.as_ptr();
            let code = match dict_type {
                DictType::Digested => {
                    let d_dict = state.d_dict.insert(zdict.d_dict(vm)?);
                    // SAFETY: both are live. `dctx` keeps the pointer, and
                    // `state.d_dict` keeps what it points to alive for it.
                    unsafe { ZSTD_DCtx_refDDict(dctx, d_dict.0.as_ptr()) }
                }
                // SAFETY: `dctx` is live and `bytes` is a live slice that
                // libzstd copies.
                DictType::Undigested => unsafe {
                    ZSTD_DCtx_loadDictionary(dctx, bytes.as_ptr().cast(), bytes.len())
                },
                // SAFETY: `dctx` is live. It keeps the pointer for the frame
                // it decompresses; `state.dict` keeps the bytes alive for it.
                DictType::Prefix => unsafe {
                    ZSTD_DCtx_refPrefix(dctx, bytes.as_ptr().cast(), bytes.len())
                },
            };
            check(code).map_err(|code| zstd_error(ErrorKind::LoadDDict, code, vm))?;
            Ok(state)
        }

        /// CPython's `stream_decompress_lock_held`, minus its end-of-frame
        /// check, which the caller has made.
        fn decompress(
            &mut self,
            data: &[u8],
            max_length: Option<usize>,
            vm: &VirtualMachine,
        ) -> Result<Vec<u8>, StreamError> {
            let result = self.decompress_step(data, max_length, vm);
            if result.is_err() {
                // CPython's `decompressor_reset_session_lock_held`: after any
                // error the decompressor starts over from scratch.
                self.pending.clear();
                self.pending_start = 0;
                self.unused_data = None;
                self.needs_input = true;
                self.eof = false;
                self.dctx.reset_session();
            }
            result
        }

        fn decompress_step(
            &mut self,
            data: &[u8],
            max_length: Option<usize>,
            vm: &VirtualMachine,
        ) -> Result<Vec<u8>, StreamError> {
            // Input left over from earlier calls comes first. New data is
            // appended to it; with no new data, as when a caller drains the
            // output in `max_length` steps, nothing is copied at all.
            let from_pending = !self.pending.is_empty();
            if from_pending && !data.is_empty() {
                self.pending.drain(..self.pending_start);
                self.pending_start = 0;
                self.pending
                    .try_reserve(data.len())
                    .map_err(|_| StreamError::NoMemory)?;
                self.pending.extend_from_slice(data);
            }
            let input = if from_pending {
                &self.pending[self.pending_start..]
            } else {
                data
            };

            // See `CompressorState::compress` for what may run detached.
            let dctx = &mut self.dctx;
            let (output, consumed, frame_ended) =
                vm.allow_threads(|| dctx.decompress(input, max_length))?;
            self.eof = frame_ended;

            if consumed == input.len() {
                // Out of input. More is needed unless the frame is over or the
                // call stopped because the output is as long as was asked for,
                // in which case libzstd may be holding more of it back.
                self.needs_input = !(frame_ended || max_length == Some(output.len()));
                self.pending.clear();
                self.pending_start = 0;
            } else {
                self.needs_input = false;
                if from_pending {
                    self.pending_start += consumed;
                } else {
                    let rest = &data[consumed..];
                    self.pending
                        .try_reserve(rest.len())
                        .map_err(|_| StreamError::NoMemory)?;
                    self.pending.extend_from_slice(rest);
                }
            }
            Ok(output)
        }
    }

    #[pyattr]
    #[pyclass(module = "compression.zstd", name = "ZstdDecompressor")]
    #[derive(PyPayload)]
    struct ZstdDecompressor {
        state: PyMutex<DecompressorState>,
        #[pymember]
        eof: AtomicBool,
        #[pymember]
        needs_input: AtomicBool,
    }

    impl core::fmt::Debug for ZstdDecompressor {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "_zstd.ZstdDecompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ZstdDecompressorArgs {
        #[pyarg(any, default = None)]
        zstd_dict: Option<PyObjectRef>,
        #[pyarg(any, default = None)]
        options: Option<PyObjectRef>,
    }

    impl Constructor for ZstdDecompressor {
        type Args = ZstdDecompressorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // The dictionary is loaded before the options are applied, as in
            // CPython, which decides the error when both are wrong.
            let mut state = DecompressorState::new(DCtx::new(vm)?, args.zstd_dict, vm)?;

            if let Some(options) = args.options {
                for_each_option(options, Direction::Decompression, vm, |key, value| {
                    let accepted = d_parameter(key).is_some_and(|(parameter, _)| {
                        state.dctx.set_parameter(parameter, value).is_ok()
                    });
                    if accepted {
                        Ok(())
                    } else {
                        Err(parameter_error(Direction::Decompression, key, value, vm))
                    }
                })?;
            }

            Ok(Self {
                state: PyMutex::new(state),
                eof: AtomicBool::new(false),
                needs_input: AtomicBool::new(true),
            })
        }
    }

    #[derive(FromArgs)]
    pub(super) struct DecompressArgs {
        #[pyarg(any)]
        data: ArgBytesLike,
        #[pyarg(any, default = -1)]
        max_length: isize,
    }

    #[pyclass(with(Constructor), flags(IMMUTABLETYPE))]
    impl ZstdDecompressor {
        #[pymethod]
        fn decompress(
            zelf: &Py<Self>,
            args: DecompressArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let eof_error = || {
                vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "Already at the end of a Zstandard frame.".into(),
                )
            };
            // Avoid copying a mutable buffer when the frame has already ended.
            if zelf.eof.load(Ordering::Acquire) {
                return Err(eof_error());
            }
            // Borrowed if the bytes cannot change under a detached thread,
            // copied otherwise.
            let data = args.data.borrow_buf_unlocked(vm)?;
            // A negative `max_length` means no limit.
            let max_length = usize::try_from(args.max_length).ok();
            let result = {
                let mut state = lock_detached(&zelf.state, vm);
                if state.eof {
                    return Err(eof_error());
                }
                let result = state.decompress(&data, max_length, vm);
                zelf.eof.store(state.eof, Ordering::Release);
                zelf.needs_input.store(state.needs_input, Ordering::Release);
                result
            };
            result.map_err(|error| error.into_exception(ErrorKind::Decompress, vm))
        }

        #[pygetset]
        fn unused_data(zelf: &Py<Self>, vm: &VirtualMachine) -> PyBytesRef {
            let mut state = lock_detached(&zelf.state, vm);
            if !state.eof {
                return vm.ctx.empty_bytes.clone();
            }
            // Whatever input the frame did not need. Once `eof` is true,
            // `pending` is never read by `decompress()` again, so transfer its
            // buffer into the cached `bytes` object instead of keeping two
            // copies alive.
            let state = &mut *state;
            state
                .unused_data
                .get_or_insert_with(|| {
                    let mut pending = core::mem::take(&mut state.pending);
                    let start = core::mem::replace(&mut state.pending_start, 0);
                    if start >= pending.len() {
                        vm.ctx.empty_bytes.clone()
                    } else {
                        if start > 0 {
                            pending.drain(..start);
                        }
                        pending.shrink_to_fit();
                        vm.ctx.new_bytes(pending)
                    }
                })
                .clone()
        }
    }

    #[derive(FromArgs)]
    pub(super) struct FrameBufferArgs {
        #[pyarg(any)]
        frame_buffer: ArgBytesLike,
    }

    #[pyfunction]
    fn get_frame_size(args: FrameBufferArgs, vm: &VirtualMachine) -> PyResult<usize> {
        let frame_size = args.frame_buffer.with_ref(|frame| {
            // SAFETY: the pointer and length are those of a live slice.
            unsafe { ZSTD_findFrameCompressedSize(frame.as_ptr().cast(), frame.len()) }
        });
        check(frame_size).map_err(|code| {
            new_zstd_error(
                format!(
                    "Error when finding the compressed size of a Zstandard frame. \
                     Ensure the frame_buffer argument starts from the \
                     beginning of a frame, and its length is not less than this \
                     complete frame. Zstd error message: {}.",
                    error_name(code)
                ),
                vm,
            )
        })
    }

    #[pyfunction]
    fn get_frame_info(args: FrameBufferArgs, vm: &VirtualMachine) -> PyResult<(Option<u64>, u32)> {
        let (content_size, dict_id) = args.frame_buffer.with_ref(|frame| {
            let (ptr, len) = (frame.as_ptr().cast(), frame.len());
            // SAFETY: the pointer and length are those of a live slice.
            unsafe {
                (
                    ZSTD_getFrameContentSize(ptr, len),
                    ZSTD_getDictID_fromFrame(ptr, len),
                )
            }
        });
        // The content size is one of two sentinels when there is none to
        // report: a header that could not be read, or one that does not say.
        if content_size == ZSTD_CONTENTSIZE_ERROR {
            return Err(new_zstd_error(
                "Error when getting information from the header of \
                 a Zstandard frame. Ensure the frame_buffer argument \
                 starts from the beginning of a frame, and its length \
                 is not less than the frame header (6~18 bytes).",
                vm,
            ));
        }
        let content_size = (content_size != ZSTD_CONTENTSIZE_UNKNOWN).then_some(content_size);
        Ok((content_size, dict_id))
    }

    /// CPython's `calculate_samples_stats`: the sizes in `samples_sizes` in
    /// the form libzstd takes them, with their count.
    ///
    /// libzstd reads as many bytes of the samples as the sizes add up to, so
    /// they are checked here to add up to exactly `samples_len`. Subtracting
    /// from that length as the sizes go by cannot wrap the way summing them
    /// could.
    fn sample_sizes(
        samples_len: usize,
        samples_sizes: &PyTuple,
        vm: &VirtualMachine,
    ) -> PyResult<(Vec<usize>, u32)> {
        let samples_sizes = samples_sizes.as_slice();
        // libzstd counts the samples in a 32-bit unsigned.
        let count = u32::try_from(samples_sizes.len()).map_err(|_| {
            vm.new_value_error(format!("The number of samples should be <= {}.", u32::MAX))
        })?;
        let mismatch =
            || vm.new_value_error("The samples size tuple doesn't match the concatenation's size.");
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(samples_sizes.len())
            .map_err(|_| vm.no_memory_error())?;
        let mut remaining = samples_len;
        for size in samples_sizes {
            // As `PyLong_AsSize_t` has it: an int, without `__index__`.
            let size = size
                .downcast_ref::<PyInt>()
                .ok_or_else(|| vm.new_type_error("an integer is required"))?;
            // A size that is negative or too large for a `size_t` cannot be
            // part of a match either.
            let size: usize = size.try_to_primitive(vm).map_err(|_| mismatch())?;
            remaining = remaining.checked_sub(size).ok_or_else(mismatch)?;
            sizes.push(size);
        }
        if remaining != 0 {
            return Err(mismatch());
        }
        Ok((sizes, count))
    }

    /// The `dict_size` argument of `train_dict` and `finalize_dict` as the
    /// capacity of the buffer to build the dictionary in.
    fn dict_capacity(dict_size: isize, vm: &VirtualMachine) -> PyResult<usize> {
        usize::try_from(dict_size)
            .ok()
            .filter(|&capacity| capacity > 0)
            .ok_or_else(|| vm.new_value_error("dict_size argument should be positive number."))
    }

    /// A buffer for libzstd to build a dictionary of at most `capacity` bytes
    /// in. The size comes straight from the caller, so failing to allocate it
    /// raises `MemoryError`, as it does in CPython, instead of aborting.
    /// (libzstd-rs-sys aborts all the same if one of its own allocations for
    /// a dictionary of that size fails.)
    fn dict_buffer(capacity: usize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(capacity)
            .map_err(|_| vm.no_memory_error())?;
        Ok(buffer)
    }

    /// Turn what `ZDICT_trainFromBuffer` or `ZDICT_finalizeDictionary`
    /// returned into the dictionary it left in `buffer`, or into the error.
    fn finish_dict(
        mut buffer: Vec<u8>,
        written: usize,
        kind: ErrorKind,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        if ZDICT_isError(written) != 0 {
            return Err(zstd_error(kind, written, vm));
        }
        debug_assert!(written <= buffer.capacity());
        // SAFETY: both functions return the size of the dictionary they wrote
        // to the start of the buffer, which is at most the capacity they were
        // told it has.
        unsafe { buffer.set_len(written) };
        Ok(buffer)
    }

    #[derive(FromArgs)]
    pub(super) struct TrainDictArgs {
        #[pyarg(positional)]
        samples_bytes: PyBytesRef,
        #[pyarg(positional)]
        samples_sizes: PyTupleRef,
        #[pyarg(positional)]
        dict_size: isize,
    }

    #[pyfunction]
    fn train_dict(args: TrainDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let capacity = dict_capacity(args.dict_size, vm)?;
        let samples = args.samples_bytes.as_bytes();
        let (sizes, count) = sample_sizes(samples.len(), &args.samples_sizes, vm)?;
        let mut dict = dict_buffer(capacity, vm)?;
        // Training can take seconds, so it runs detached, as CPython releases
        // the GIL for it. `samples` belongs to a `bytes` object, which never
        // changes.
        let written = vm.allow_threads(|| {
            // SAFETY: `dict` has room for `dict.capacity()` bytes, `sizes`
            // holds `count` sizes, and `sample_sizes` checked that they add up
            // to the length of `samples`.
            unsafe {
                ZDICT_trainFromBuffer(
                    dict.as_mut_ptr().cast(),
                    dict.capacity(),
                    samples.as_ptr().cast(),
                    sizes.as_ptr(),
                    count,
                )
            }
        });
        finish_dict(dict, written, ErrorKind::TrainDict, vm)
    }

    #[derive(FromArgs)]
    pub(super) struct FinalizeDictArgs {
        #[pyarg(positional)]
        custom_dict_bytes: PyBytesRef,
        #[pyarg(positional)]
        samples_bytes: PyBytesRef,
        #[pyarg(positional)]
        samples_sizes: PyTupleRef,
        #[pyarg(positional)]
        dict_size: isize,
        #[pyarg(positional)]
        compression_level: i32,
    }

    #[pyfunction]
    fn finalize_dict(args: FinalizeDictArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let capacity = dict_capacity(args.dict_size, vm)?;
        let custom_dict = args.custom_dict_bytes.as_bytes();
        let samples = args.samples_bytes.as_bytes();
        let (sizes, count) = sample_sizes(samples.len(), &args.samples_sizes, vm)?;
        let mut dict = dict_buffer(capacity, vm)?;
        let params = ZDICT_params_t {
            compressionLevel: args.compression_level,
            // Nothing is to be logged to stderr.
            notificationLevel: 0,
            // libzstd is to derive the dictionary id from the content.
            dictID: 0,
        };
        // Detached for the same reason as `train_dict`.
        let written = vm.allow_threads(|| {
            // SAFETY: as in `train_dict`, and `custom_dict` is a live slice.
            unsafe {
                ZDICT_finalizeDictionary(
                    dict.as_mut_ptr().cast(),
                    dict.capacity(),
                    custom_dict.as_ptr().cast(),
                    custom_dict.len(),
                    samples.as_ptr().cast(),
                    sizes.as_ptr(),
                    count,
                    params,
                )
            }
        });
        finish_dict(dict, written, ErrorKind::FinalizeDict, vm)
    }

    #[derive(FromArgs)]
    pub(super) struct ParamBoundsArgs {
        #[pyarg(any)]
        parameter: i32,
        #[pyarg(any)]
        is_compress: ArgIntoBool,
    }

    #[pyfunction]
    fn get_param_bounds(args: ParamBoundsArgs, vm: &VirtualMachine) -> PyResult<(c_int, c_int)> {
        let (direction, kind) = if args.is_compress.into_bool() {
            (Direction::Compression, ErrorKind::GetCBounds)
        } else {
            (Direction::Decompression, ErrorKind::GetDBounds)
        };
        match parameter_info(direction, args.parameter) {
            Some((_, Ok(bounds))) => Ok(bounds),
            Some((_, Err(code))) => Err(zstd_error(kind, code, vm)),
            None => {
                // What libzstd says about a parameter it does not know.
                // SAFETY: `ZSTD_getErrorString` returns one of libzstd's
                // static, NUL-terminated messages.
                let reason = unsafe {
                    CStr::from_ptr(ZSTD_getErrorString(ZSTD_error_parameter_unsupported))
                };
                let reason = reason.to_str().expect("libzstd error messages are ASCII");
                Err(zstd_error_because(kind, reason, vm))
            }
        }
    }

    #[derive(FromArgs)]
    pub(super) struct ParameterTypesArgs {
        #[pyarg(any)]
        c_parameter_type: PyTypeRef,
        #[pyarg(any)]
        d_parameter_type: PyTypeRef,
    }

    // Register the `CompressionParameter` / `DecompressionParameter` enum
    // classes defined by the pure-Python wrapper so `for_each_option` can
    // reject a key from the wrong parameter family by identity. The types
    // live in `PyModule::native_state()`, keeping them out of `_zstd.__dict__`
    // and isolated per interpreter. The wrapper in
    // `Lib/compression/zstd/__init__.py` calls this once at import.
    #[pyfunction]
    fn set_parameter_types(args: ParameterTypesArgs, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(state) = module_state(vm, true)? {
            *state.c_parameter_type.write() = Some(args.c_parameter_type);
            *state.d_parameter_type.write() = Some(args.d_parameter_type);
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        // Free a compressor while a worker thread is still compressing with a
        // dictionary that the context owns (see `Drop for CCtx`). Without the
        // wait there the worker reads freed memory. Whether that is noticed
        // depends on the allocator; with the system allocator, which this test
        // binary uses, it crashes.
        #[test]
        fn free_busy_multithreaded_compressor() {
            let builder = rustpython_vm::Interpreter::builder(Default::default());
            let module = module_def(&builder.ctx);
            let interpreter = builder.add_native_module(module).build();
            interpreter.enter(|vm| {
                // Incompressible, so that the worker has work to do.
                let mut random = 0x9E37_79B9_u32;
                let block: Vec<u8> = (0..1 << 10)
                    .map(|_| {
                        random ^= random << 13;
                        random ^= random >> 17;
                        random ^= random << 5;
                        random as u8
                    })
                    .collect();
                let scope = vm.new_scope_with_builtins();
                scope
                    .globals
                    .set_item("block", vm.ctx.new_bytes(block).into(), vm)
                    .unwrap();
                vm.run_string(
                    scope,
                    r#"
import _zstd

job_size = 1 << 20
options = {
    _zstd.ZSTD_c_compressionLevel: 10,
    _zstd.ZSTD_c_nbWorkers: 1,
    _zstd.ZSTD_c_jobSize: job_size,
}
for _ in range(4):
    zstd_dict = _zstd.ZstdDict(block * (1 << 10), is_raw=True)
    compressor = _zstd.ZstdCompressor(options=options, zstd_dict=zstd_dict)
    # Returns while the worker thread is compressing the first job.
    compressor.compress(block * ((job_size >> 10) + 64))
    del compressor
"#,
                    "<busy compressor>",
                )
                .unwrap();
            });
        }
    }
}
