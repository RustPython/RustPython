// spell-checker:ignore ARMTHUMB memlimit

pub(crate) use _lzma::module_def;

#[pymodule]
mod _lzma {
    use crate::compression::DecompressArgs;
    use alloc::fmt;
    use rustpython_common::{compression::lzma as backend, lock::PyMutex};
    use rustpython_vm::builtins::{PyBaseExceptionRef, PyBytesRef, PyDict, PyType, PyTypeRef};
    use rustpython_vm::function::ArgBytesLike;
    use rustpython_vm::types::Constructor;
    use rustpython_vm::{Py, PyObjectRef, PyPayload, PyResult, VirtualMachine};

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
        vm.ctx.new_exception_type(
            "lzma",
            "LZMAError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    fn new_lzma_error(message: impl Into<String>, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let message: String = message.into();
        vm.new_exception_msg(vm.class("lzma", "LZMAError"), message.into())
    }

    fn map_backend_error(error: backend::Error, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match error {
            backend::Error::Memory => vm.new_memory_error(""),
            backend::Error::Value(message) => vm.new_value_error(message),
            backend::Error::Lzma(message) => new_lzma_error(message, vm),
            backend::Error::Eof => vm.new_eof_error("End of stream already reached"),
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
            Some(value) => Ok(Some(value.try_into_value::<u32>(vm)?)),
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
            Some(value) => Ok(Some(value.try_into_value::<u64>(vm)?)),
            None => Ok(None),
        }
    }

    fn filter_spec_with_id(
        spec: &PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<backend::FilterSpec> {
        let id = get_dict_opt_u64(spec, "id", vm)?
            .ok_or_else(|| vm.new_value_error("Filter specifier must have an \"id\" entry"))?;
        Ok(backend::FilterSpec {
            id,
            ..backend::FilterSpec::default()
        })
    }

    fn parse_filter_chain_item(
        spec: &PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<backend::FilterSpec> {
        let mut parsed = filter_spec_with_id(spec, vm)?;
        match parsed.id {
            FILTER_LZMA1 | FILTER_LZMA2 => {
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
            FILTER_DELTA => parsed.dist = get_dict_opt_u32(spec, "dist", vm)?,
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                parsed.start_offset = get_dict_opt_u32(spec, "start_offset", vm)?;
            }
            _ => {}
        }
        Ok(parsed)
    }

    fn parse_filter_properties(
        spec: &PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<backend::FilterSpec> {
        let mut parsed = filter_spec_with_id(spec, vm)?;
        match parsed.id {
            FILTER_LZMA1 => {
                parsed.preset = get_dict_opt_u32(spec, "preset", vm)?;
                parsed.lc = get_dict_opt_u32(spec, "lc", vm)?;
                parsed.lp = get_dict_opt_u32(spec, "lp", vm)?;
                parsed.pb = get_dict_opt_u32(spec, "pb", vm)?;
                parsed.dict_size = get_dict_opt_u32(spec, "dict_size", vm)?;
            }
            FILTER_LZMA2 => {
                parsed.preset = get_dict_opt_u32(spec, "preset", vm)?;
                parsed.dict_size = get_dict_opt_u32(spec, "dict_size", vm)?;
            }
            FILTER_DELTA => parsed.dist = get_dict_opt_u32(spec, "dist", vm)?,
            FILTER_X86 | FILTER_POWERPC | FILTER_IA64 | FILTER_ARM | FILTER_ARMTHUMB
            | FILTER_SPARC => {
                parsed.start_offset = get_dict_opt_u32(spec, "start_offset", vm)?;
            }
            _ => {}
        }
        Ok(parsed)
    }

    fn parse_filter_chain(
        filter_specs: PyObjectRef,
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
            .map(|filters| parse_filter_chain(filters, vm))
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
    fn _encode_filter_properties(
        filter_spec: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let spec = parse_filter_properties(&filter_spec, vm)?;
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
        unused_data: PyBytesRef,
    }

    impl DecompressorInner {
        fn sync_visible_state(&mut self, vm: &VirtualMachine) {
            if self.unused_data.as_bytes() != self.backend.unused_data() {
                self.unused_data = vm.ctx.new_bytes(self.backend.unused_data().to_vec());
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "LZMADecompressor")]
    #[derive(PyPayload)]
    struct LZMADecompressor {
        state: PyMutex<DecompressorInner>,
    }

    impl fmt::Debug for LZMADecompressor {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "_lzma.LZMADecompressor")
        }
    }

    #[derive(FromArgs)]
    pub(super) struct LZMADecompressorConstructorArgs {
        #[pyarg(any, default = FORMAT_AUTO)]
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
            Ok(Self {
                state: PyMutex::new(DecompressorInner {
                    backend,
                    unused_data: vm.ctx.empty_bytes.clone(),
                }),
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
            let result = state.backend.decompress(data, max_length);
            state.sync_visible_state(vm);
            result.map_err(|error| map_backend_error(error, vm))
        }

        #[pygetset]
        fn check(&self) -> i32 {
            self.state.lock().backend.check()
        }

        #[pygetset]
        fn eof(&self) -> bool {
            self.state.lock().backend.eof()
        }

        #[pygetset]
        fn unused_data(&self) -> PyBytesRef {
            self.state.lock().unused_data.clone()
        }

        #[pygetset]
        fn needs_input(&self) -> bool {
            self.state.lock().backend.needs_input()
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
        #[pyarg(any, default = FORMAT_XZ)]
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
        fn compress(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            data.with_ref(|data| self.state.lock().compress(data))
                .map_err(|error| map_backend_error(error, vm))
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.state
                .lock()
                .flush()
                .map_err(|error| map_backend_error(error, vm))
        }
    }
}
