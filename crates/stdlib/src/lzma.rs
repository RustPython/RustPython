// spell-checker:ignore ARMTHUMB memlimit

pub(crate) use _lzma::module_def;

#[pymodule]
mod _lzma {
    use crate::compression::DecompressorArgs;
    use alloc::fmt;
    use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use rustpython_common::{compression::lzma as backend, lock::PyMutex};
    use rustpython_vm::builtins::{PyBaseExceptionRef, PyBytes, PyDict, PyStr, PyType, PyTypeRef};
    use rustpython_vm::function::{ArgBytesLike, ItemDoc};
    use rustpython_vm::object::PyAtomicRef;
    use rustpython_vm::types::Constructor;
    use rustpython_vm::{Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine};

    #[pyattr]
    const CHECK_NONE: i32 = backend::CHECK_NONE;
    #[pyattr]
    const CHECK_CRC32: i32 = backend::CHECK_CRC32;
    #[pyattr]
    const CHECK_CRC64: i32 = backend::CHECK_CRC64;
    #[pyattr]
    const CHECK_SHA256: i32 = backend::CHECK_SHA256;
    #[pyattr]
    const CHECK_ID_MAX: i32 = backend::CHECK_ID_MAX;
    #[pyattr]
    const CHECK_UNKNOWN: i32 = backend::CHECK_UNKNOWN;

    #[pyattr]
    const MF_HC3: i32 = backend::MF_HC3;
    #[pyattr]
    const MF_HC4: i32 = backend::MF_HC4;
    #[pyattr]
    const MF_BT2: i32 = backend::MF_BT2;
    #[pyattr]
    const MF_BT3: i32 = backend::MF_BT3;
    #[pyattr]
    const MF_BT4: i32 = backend::MF_BT4;

    #[pyattr]
    const MODE_FAST: i32 = backend::MODE_FAST;
    #[pyattr]
    const MODE_NORMAL: i32 = backend::MODE_NORMAL;

    #[pyattr]
    const FORMAT_AUTO: i32 = backend::FORMAT_AUTO;
    #[pyattr]
    const FORMAT_XZ: i32 = backend::FORMAT_XZ;
    #[pyattr]
    const FORMAT_ALONE: i32 = backend::FORMAT_ALONE;
    #[pyattr]
    const FORMAT_RAW: i32 = backend::FORMAT_RAW;

    #[pyattr]
    const FILTER_LZMA1: u64 = backend::FILTER_LZMA1;
    #[pyattr]
    const FILTER_LZMA2: u64 = backend::FILTER_LZMA2;
    #[pyattr]
    const FILTER_DELTA: u64 = backend::FILTER_DELTA;
    #[pyattr]
    const FILTER_X86: u64 = backend::FILTER_X86;
    #[pyattr]
    const FILTER_POWERPC: u64 = backend::FILTER_POWERPC;
    #[pyattr]
    const FILTER_IA64: u64 = backend::FILTER_IA64;
    #[pyattr]
    const FILTER_ARM: u64 = backend::FILTER_ARM;
    #[pyattr]
    const FILTER_ARMTHUMB: u64 = backend::FILTER_ARMTHUMB;
    #[pyattr]
    const FILTER_SPARC: u64 = backend::FILTER_SPARC;

    #[pyattr]
    const PRESET_DEFAULT: u32 = backend::PRESET_DEFAULT;
    #[pyattr]
    const PRESET_EXTREME: u32 = backend::PRESET_EXTREME;

    #[pyattr(once, name = "LZMAError")]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type_with_doc(
            "_lzma",
            "LZMAError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
            const { ItemDoc::db("_lzma.LZMAError") },
        )
    }

    fn new_lzma_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let message: String = message.into();
        vm.new_exception_msg(vm.class("lzma", "LZMAError"), message.into())
    }

    fn map_backend_error(error: backend::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match error {
            backend::Error::Memory => vm.no_memory_error(),
            backend::Error::Value(message) => vm.new_value_error(message),
            backend::Error::Lzma(message) => new_lzma_error(message, vm),
            backend::Error::Eof => vm.new_eof_error("End of stream already reached"),
        }
    }

    fn get_dict_opt_u32(spec: &PyObject, key: &str, vm: &VirtualMachine) -> PyResult<Option<u32>> {
        let dict = spec.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error("Filter specifier must be a dict or dict-like object")
        })?;
        match dict.get_item_opt(key, vm)? {
            Some(value) => Ok(Some(value.try_into_value::<u32>(vm)?)),
            None => Ok(None),
        }
    }

    fn get_dict_opt_u64(spec: &PyObject, key: &str, vm: &VirtualMachine) -> PyResult<Option<u64>> {
        let dict = spec.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error("Filter specifier must be a dict or dict-like object")
        })?;
        match dict.get_item_opt(key, vm)? {
            Some(value) => Ok(Some(value.try_into_value::<u64>(vm)?)),
            None => Ok(None),
        }
    }

    fn filter_spec_with_id(spec: &PyObject, vm: &VirtualMachine) -> PyResult<backend::FilterSpec> {
        let id = get_dict_opt_u64(spec, "id", vm)?
            .ok_or_else(|| vm.new_value_error("Filter specifier must have an \"id\" entry"))?;
        Ok(backend::FilterSpec {
            id,
            ..backend::FilterSpec::default()
        })
    }

    /// A key the filter does not take makes the whole specifier invalid, as
    /// the spec dict is read as keyword arguments.
    fn check_filter_spec_keys(
        spec: &PyObject,
        allowed: &[&str],
        error: &str,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dict = spec.downcast_ref::<PyDict>().ok_or_else(|| {
            vm.new_type_error("Filter specifier must be a dict or dict-like object")
        })?;
        for key in dict.keys_vec() {
            let ok = key.downcast_ref::<PyStr>().is_some_and(|k| {
                allowed
                    .iter()
                    .any(|a| k.as_wtf8().as_bytes() == a.as_bytes())
            });
            if !ok {
                return Err(vm.new_value_error(error.to_owned()));
            }
        }
        Ok(())
    }

    fn parse_filter_chain_item(
        spec: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<backend::FilterSpec> {
        let mut parsed = filter_spec_with_id(spec, vm)?;
        match parsed.id {
            FILTER_LZMA1 | FILTER_LZMA2 => {
                check_filter_spec_keys(
                    spec,
                    &[
                        "id",
                        "preset",
                        "dict_size",
                        "lc",
                        "lp",
                        "pb",
                        "mode",
                        "nice_len",
                        "mf",
                        "depth",
                    ],
                    "Invalid filter specifier for LZMA filter",
                    vm,
                )?;
                parsed.preset = get_dict_opt_u32(spec, "preset", vm)?;
                parsed.dict_size = get_dict_opt_u32(spec, "dict_size", vm)?;
                parsed.lc = get_dict_opt_u32(spec, "lc", vm)?;
                parsed.lp = get_dict_opt_u32(spec, "lp", vm)?;
                parsed.pb = get_dict_opt_u32(spec, "pb", vm)?;
                parsed.mode = get_dict_opt_u32(spec, "mode", vm)?;
                parsed.nice_len = get_dict_opt_u32(spec, "nice_len", vm)?;
                parsed.mf = get_dict_opt_u32(spec, "mf", vm)?;
                parsed.depth = get_dict_opt_u32(spec, "depth", vm)?;
            }
            FILTER_DELTA => {
                check_filter_spec_keys(
                    spec,
                    &["id", "dist"],
                    "Invalid filter specifier for delta filter",
                    vm,
                )?;
                parsed.dist = get_dict_opt_u32(spec, "dist", vm)?;
            }
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                check_filter_spec_keys(
                    spec,
                    &["id", "start_offset"],
                    "Invalid filter specifier for BCJ filter",
                    vm,
                )?;
                parsed.start_offset = get_dict_opt_u32(spec, "start_offset", vm)?;
            }
            _ => {}
        }
        Ok(parsed)
    }

    fn parse_filter_properties(
        spec: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<backend::FilterSpec> {
        fn take_keyword(
            dict: &PyDict,
            key: &str,
            remaining: &mut usize,
            error: &str,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            if *remaining == 0 {
                return Ok(None);
            }
            let value = dict
                .get_item_opt(key, vm)
                .map_err(|_| vm.new_value_error(error))?;
            if value.is_some() {
                *remaining -= 1;
            }
            Ok(value)
        }

        fn field(
            dict: &PyDict,
            key: &str,
            remaining: &mut usize,
            error: &str,
            vm: &VirtualMachine,
        ) -> PyResult<Option<u32>> {
            take_keyword(dict, key, remaining, error, vm)?
                .map(|value| {
                    value
                        .try_into_value::<u32>(vm)
                        .map_err(|_| vm.new_value_error(error))
                })
                .transpose()
        }

        let mut parsed = filter_spec_with_id(spec, vm)?;
        let dict = spec
            .downcast_ref::<PyDict>()
            .expect("filter ID was read from a dict");
        let (max, error) = match parsed.id {
            FILTER_LZMA1 | FILTER_LZMA2 => {
                parsed.preset = get_dict_opt_u32(spec, "preset", vm)?;
                if let Some(preset) = parsed.preset
                    && preset & !PRESET_EXTREME > 9
                {
                    return Err(new_lzma_error(
                        format!("Invalid compression preset: {preset}"),
                        vm,
                    ));
                }
                (10, "Invalid filter specifier for LZMA filter")
            }
            FILTER_DELTA => (2, "Invalid filter specifier for delta filter"),
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => (2, "Invalid filter specifier for BCJ filter"),
            _ => return Ok(parsed),
        };
        // PyArg_ParseTupleAndKeywords snapshots the keyword count and stops
        // looking up optional fields when all of those keywords are consumed.
        let mut remaining = dict.__len__();
        if remaining > max {
            return Err(vm.new_value_error(error));
        }
        let _ = take_keyword(dict, "id", &mut remaining, error, vm)?;
        if matches!(parsed.id, FILTER_LZMA1 | FILTER_LZMA2) {
            let _ = take_keyword(dict, "preset", &mut remaining, error, vm)?;
            parsed.dict_size = field(dict, "dict_size", &mut remaining, error, vm)?;
            parsed.lc = field(dict, "lc", &mut remaining, error, vm)?;
            parsed.lp = field(dict, "lp", &mut remaining, error, vm)?;
            parsed.pb = field(dict, "pb", &mut remaining, error, vm)?;
            parsed.mode = field(dict, "mode", &mut remaining, error, vm)?;
            parsed.nice_len = field(dict, "nice_len", &mut remaining, error, vm)?;
            parsed.mf = field(dict, "mf", &mut remaining, error, vm)?;
            parsed.depth = field(dict, "depth", &mut remaining, error, vm)?;
            if remaining > 0 {
                check_filter_spec_keys(
                    spec,
                    &[
                        "id",
                        "preset",
                        "dict_size",
                        "lc",
                        "lp",
                        "pb",
                        "mode",
                        "nice_len",
                        "mf",
                        "depth",
                    ],
                    error,
                    vm,
                )?;
            }
        } else if parsed.id == FILTER_DELTA {
            parsed.dist = field(dict, "dist", &mut remaining, error, vm)?;
            if remaining > 0 {
                check_filter_spec_keys(spec, &["id", "dist"], error, vm)?;
            }
        } else {
            parsed.start_offset = field(dict, "start_offset", &mut remaining, error, vm)?;
            if remaining > 0 {
                check_filter_spec_keys(spec, &["id", "start_offset"], error, vm)?;
            }
        }
        Ok(parsed)
    }

    fn parse_filter_chain(
        filter_specs: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<backend::FilterSpec>> {
        const LZMA_FILTERS_MAX: usize = 4;
        let length = filter_specs.length(vm)?;
        if length > LZMA_FILTERS_MAX {
            return Err(new_lzma_error(
                format!("Too many filters - liblzma supports a maximum of {LZMA_FILTERS_MAX}"),
                vm,
            ));
        }
        let sequence = filter_specs.try_sequence(vm)?;
        (0..length)
            .map(|index| {
                let spec = sequence.get_item(index as isize, vm)?;
                parse_filter_chain_item(&spec, vm)
            })
            .collect()
    }

    fn filters_to_backend(
        filters: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<Vec<backend::FilterSpec>>> {
        filters
            .map(|filters| parse_filter_chain(&filters, vm))
            .transpose()
    }

    fn filter_spec_to_dict(
        spec: backend::FilterSpec,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let dict = vm.ctx.new_dict();
        dict.set_item("id", vm.new_pyobj(spec.id), vm)?;
        match spec.id {
            FILTER_LZMA1 => {
                dict.set_item("lc", vm.new_pyobj(spec.lc.unwrap()), vm)?;
                dict.set_item("lp", vm.new_pyobj(spec.lp.unwrap()), vm)?;
                dict.set_item("pb", vm.new_pyobj(spec.pb.unwrap()), vm)?;
                dict.set_item("dict_size", vm.new_pyobj(spec.dict_size.unwrap()), vm)?;
            }
            FILTER_LZMA2 => {
                dict.set_item("dict_size", vm.new_pyobj(spec.dict_size.unwrap()), vm)?;
            }
            FILTER_DELTA => {
                dict.set_item("dist", vm.new_pyobj(spec.dist.unwrap()), vm)?;
            }
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                if let Some(start_offset) = spec.start_offset {
                    dict.set_item("start_offset", vm.new_pyobj(start_offset), vm)?;
                }
            }
            _ => unreachable!("common backend validated the filter ID"),
        }
        Ok(dict.into())
    }

    #[pyfunction]
    fn is_check_supported(check_id: i32) -> bool {
        backend::is_check_supported(check_id)
    }

    #[pyfunction]
    fn _encode_filter_properties(filter: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let spec = parse_filter_properties(&filter, vm)?;
        backend::encode_filter_properties(&spec).map_err(|error| map_backend_error(error, vm))
    }

    #[pyfunction]
    fn _decode_filter_properties(
        filter_id: u64,
        encoded_props: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let spec = encoded_props
            .with_ref(|properties| backend::decode_filter_properties(filter_id, properties));
        filter_spec_to_dict(spec.map_err(|error| map_backend_error(error, vm))?, vm)
    }

    struct DecompressorInner {
        backend: backend::Decompressor,
    }

    #[pyattr]
    #[pyclass(name = "LZMADecompressor", traverse)]
    #[derive(PyPayload)]
    struct LZMADecompressor {
        #[pytraverse(skip)]
        state: PyMutex<DecompressorInner>,
        #[pymember]
        #[pytraverse(skip)]
        check: AtomicI32,
        #[pymember]
        #[pytraverse(skip)]
        eof: AtomicBool,
        #[pymember]
        #[pytraverse(skip)]
        needs_input: AtomicBool,
        #[pymember(type = "object_ex")]
        unused_data: PyAtomicRef<Option<PyObject>>,
    }

    impl fmt::Debug for LZMADecompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_lzma.LZMADecompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct LZMADecompressorConstructorArgs {
        #[pyarg(any, default = ::FORMAT_AUTO)]
        format: i32,
        #[pyarg(any, optional)]
        memlimit: Option<u64>,
        #[pyarg(any, optional)]
        filters: Option<PyObjectRef>,
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
            let filters = filters_to_backend(args.filters, vm)?;
            let backend = backend::Decompressor::new(args.format, args.memlimit, filters)
                .map_err(|error| map_backend_error(error, vm))?;
            let check = backend.check();
            Ok(Self {
                state: PyMutex::new(DecompressorInner { backend }),
                check: AtomicI32::new(check),
                eof: AtomicBool::new(false),
                needs_input: AtomicBool::new(true),
                unused_data: PyAtomicRef::from(Some(vm.ctx.empty_bytes.clone().into())),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl LZMADecompressor {
        #[pymethod]
        fn decompress(
            zelf: &Py<Self>,
            args: DecompressorArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let max_length = args.max_length();
            let data = &*args.data();
            let mut state = zelf.state.lock();
            let result = state.backend.decompress(data, max_length);
            zelf.check.store(state.backend.check(), Ordering::Relaxed);
            zelf.eof.store(state.backend.eof(), Ordering::Relaxed);
            zelf.needs_input
                .store(state.backend.needs_input(), Ordering::Relaxed);
            let stale = zelf.unused_data.deref().is_none_or(|obj| {
                obj.downcast_ref::<PyBytes>()
                    .is_none_or(|bytes| bytes.as_bytes() != state.backend.unused_data())
            });
            if stale {
                let bytes = vm.ctx.new_bytes(state.backend.unused_data().to_vec());
                let _previous = unsafe { zelf.unused_data.swap(Some(bytes.into())) };
            }
            result.map_err(|error| map_backend_error(error, vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "LZMACompressor")]
    #[derive(PyPayload)]
    struct LZMACompressor {
        state: PyMutex<backend::Compressor>,
    }

    impl fmt::Debug for LZMACompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_lzma.LZMACompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct LZMACompressorConstructorArgs {
        #[pyarg(any, default = ::FORMAT_XZ)]
        format: i32,
        #[pyarg(any, default = -1)]
        check: i32,
        #[pyarg(any, optional)]
        preset: Option<PyObjectRef>,
        #[pyarg(any, optional)]
        filters: Option<PyObjectRef>,
    }

    impl Constructor for LZMACompressor {
        type Args = LZMACompressorConstructorArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            if args.format != FORMAT_XZ && args.check != -1 && args.check != CHECK_NONE {
                return Err(vm.new_value_error("Integrity checks are only supported by FORMAT_XZ"));
            }
            if args.preset.is_some() && args.filters.is_some() {
                return Err(vm.new_value_error("Cannot specify both preset and filter chain"));
            }
            let preset = match args.preset {
                Some(preset) => preset.try_into_value::<u32>(vm)?,
                None => PRESET_DEFAULT,
            };
            let filters = match args.format {
                FORMAT_XZ | FORMAT_ALONE | FORMAT_RAW => filters_to_backend(args.filters, vm)?,
                _ => None,
            };
            let backend = backend::Compressor::new(args.format, args.check, preset, filters)
                .map_err(|error| map_backend_error(error, vm))?;
            Ok(Self {
                state: PyMutex::new(backend),
            })
        }
    }

    #[pyclass(with(Constructor))]
    impl LZMACompressor {
        #[pymethod]
        fn compress(zelf: &Py<Self>, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            data.with_ref(|data| zelf.state.lock().compress(data))
                .map_err(|error| map_backend_error(error, vm))
        }

        #[pymethod]
        fn flush(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            zelf.state
                .lock()
                .flush()
                .map_err(|error| map_backend_error(error, vm))
        }
    }
}
