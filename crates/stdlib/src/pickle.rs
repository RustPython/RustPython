// spell-checker:ignore picklebuffer unframer unpickler pickler picklers
pub(crate) use _pickle::module_def;

#[pymodule]
mod _pickle {
    use crate::common::{
        lock::{PyMutex, PyRwLock},
        wtf8::Wtf8Buf,
    };
    use crate::vm::{
        AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyByteArray, PyBytes, PyDict, PyDictRef, PyFloat, PyFrozenSet,
            PyInt, PyList, PySet, PyStr, PyTuple, PyTupleRef, PyType, PyTypeRef,
        },
        function::{FuncArgs, OptionalArg, PySetterValue},
        protocol::{PyBuffer, PyIter, PyIterReturn},
        types::{AsBuffer, Constructor, Initializer, Representable},
    };
    use malachite_bigint::BigInt;
    use num_traits::{ToPrimitive, Zero};
    use std::collections::HashMap;

    /// The highest protocol number `_pickle` knows how to read.
    #[pyattr]
    const HIGHEST_PROTOCOL: u8 = 5;
    /// The protocol `dumps`/`dump` use when no protocol is given.
    #[pyattr]
    const DEFAULT_PROTOCOL: u8 = 5;

    #[pyattr(name = "PickleError", once)]
    fn pickle_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx.new_exception_type(
            "_pickle",
            "PickleError",
            Some(vec![vm.ctx.exceptions.exception_type.to_owned()]),
        )
    }

    #[pyattr(name = "PicklingError", once)]
    fn pickling_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("_pickle", "PicklingError", Some(vec![pickle_error(vm)]))
    }

    #[pyattr(name = "UnpicklingError", once)]
    fn unpickling_error(vm: &VirtualMachine) -> PyTypeRef {
        vm.ctx
            .new_exception_type("_pickle", "UnpicklingError", Some(vec![pickle_error(vm)]))
    }

    pub(super) fn new_pickling_error(
        vm: &VirtualMachine,
        msg: impl Into<rustpython_common::wtf8::Wtf8Buf>,
    ) -> PyBaseExceptionRef {
        vm.new_exception_msg(pickling_error(vm), msg.into())
    }

    pub(super) fn new_unpickling_error(
        vm: &VirtualMachine,
        msg: impl Into<rustpython_common::wtf8::Wtf8Buf>,
    ) -> PyBaseExceptionRef {
        vm.new_exception_msg(unpickling_error(vm), msg.into())
    }

    // ---------------------------------------------------------------- PickleBuffer

    #[pyattr]
    #[pyclass(module = "_pickle", name = "PickleBuffer")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyPickleBuffer {
        buffer: PyRwLock<Option<PyBuffer>>,
    }

    impl Constructor for PyPickleBuffer {
        type Args = PyObjectRef;

        fn py_new(_cls: &Py<PyType>, arg: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let buffer =
                PyBuffer::from_object(vm, &arg, crate::vm::protocol::BufferFlags::FULL_RO)?;
            Ok(Self {
                buffer: PyRwLock::new(Some(buffer)),
            })
        }
    }

    #[pyclass(
        with(Constructor, AsBuffer, Representable),
        flags(BASETYPE, HAS_WEAKREF)
    )]
    impl PyPickleBuffer {
        fn get(&self, vm: &VirtualMachine) -> PyResult<PyBuffer> {
            self.buffer.read().clone().ok_or_else(|| {
                vm.new_value_error("operation forbidden on released PickleBuffer object")
            })
        }

        /// Return a memoryview of the raw memory underlying this buffer.
        /// Will raise BufferError if the buffer isn't contiguous.
        #[pymethod]
        fn raw(&self, vm: &VirtualMachine) -> PyResult<PyRef<crate::vm::builtins::PyMemoryView>> {
            let buffer = self.get(vm)?;
            if !buffer.desc.is_contiguous() {
                return Err(vm.new_buffer_error(
                    "cannot pickle a non-contiguous buffer: PickleBuffer is not contiguous",
                ));
            }
            Ok(crate::vm::builtins::PyMemoryView::from_buffer(buffer, vm)?.into_ref(&vm.ctx))
        }

        /// Release the underlying buffer exposed by the PickleBuffer object.
        #[pymethod]
        fn release(&self) {
            *self.buffer.write() = None;
        }
    }

    impl AsBuffer for PyPickleBuffer {
        fn as_buffer(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyBuffer> {
            zelf.get(vm)
        }
    }

    impl Representable for PyPickleBuffer {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(format!("<PickleBuffer object at {:#x}>", zelf.get_id()))
        }
    }

    // ---------------------------------------------------------------- opcodes

    const MARK: u8 = b'(';
    const STOP: u8 = b'.';
    const POP: u8 = b'0';
    const POP_MARK: u8 = b'1';
    const DUP: u8 = b'2';
    const FLOAT: u8 = b'F';
    const INT: u8 = b'I';
    const BININT: u8 = b'J';
    const BININT1: u8 = b'K';
    const LONG: u8 = b'L';
    const BININT2: u8 = b'M';
    const NONE: u8 = b'N';
    const PERSID: u8 = b'P';
    const BINPERSID: u8 = b'Q';
    const REDUCE: u8 = b'R';
    const STRING: u8 = b'S';
    const BINSTRING: u8 = b'T';
    const SHORT_BINSTRING: u8 = b'U';
    const UNICODE: u8 = b'V';
    const BINUNICODE: u8 = b'X';
    const APPEND: u8 = b'a';
    const BUILD: u8 = b'b';
    const GLOBAL: u8 = b'c';
    const DICT: u8 = b'd';
    const EMPTY_DICT: u8 = b'}';
    const APPENDS: u8 = b'e';
    const GET: u8 = b'g';
    const BINGET: u8 = b'h';
    const INST: u8 = b'i';
    const LONG_BINGET: u8 = b'j';
    const LIST: u8 = b'l';
    const EMPTY_LIST: u8 = b']';
    const OBJ: u8 = b'o';
    const PUT: u8 = b'p';
    const BINPUT: u8 = b'q';
    const LONG_BINPUT: u8 = b'r';
    const SETITEM: u8 = b's';
    const TUPLE: u8 = b't';
    const EMPTY_TUPLE: u8 = b')';
    const SETITEMS: u8 = b'u';
    const BINFLOAT: u8 = b'G';

    const PROTO: u8 = 0x80;
    const NEWOBJ: u8 = 0x81;
    const EXT1: u8 = 0x82;
    const EXT2: u8 = 0x83;
    const EXT4: u8 = 0x84;
    const TUPLE1: u8 = 0x85;
    const TUPLE2: u8 = 0x86;
    const TUPLE3: u8 = 0x87;
    const NEWTRUE: u8 = 0x88;
    const NEWFALSE: u8 = 0x89;
    const LONG1: u8 = 0x8a;
    const LONG4: u8 = 0x8b;

    const BINBYTES: u8 = b'B';
    const SHORT_BINBYTES: u8 = b'C';

    const SHORT_BINUNICODE: u8 = 0x8c;
    const BINUNICODE8: u8 = 0x8d;
    const BINBYTES8: u8 = 0x8e;
    const EMPTY_SET: u8 = 0x8f;
    const ADDITEMS: u8 = 0x90;
    const FROZENSET: u8 = 0x91;
    const NEWOBJ_EX: u8 = 0x92;
    const STACK_GLOBAL: u8 = 0x93;
    const MEMOIZE: u8 = 0x94;
    const FRAME: u8 = 0x95;

    const BYTEARRAY8: u8 = 0x96;
    const NEXT_BUFFER: u8 = 0x97;
    const READONLY_BUFFER: u8 = 0x98;

    /// Amount of data `peek()` grabs at a time from a file-like input.
    const PREFETCH: usize = 8192;

    // ---------------------------------------------------------------- input handling

    #[derive(Debug, Default)]
    pub(super) struct ReadState {
        read: Option<PyObjectRef>,
        readline: Option<PyObjectRef>,
        peek: Option<PyObjectRef>,
        /// Bytes currently available to the opcode loop.
        buf: Vec<u8>,
        /// Index of the next unread byte in `buf`.
        pos: usize,
        /// Index up to which the underlying file position has been advanced.
        /// Anything past it was obtained with `peek()` and is not consumed yet.
        prefetched: usize,
        /// End of the current FRAME, if any.
        frame_end: Option<usize>,
    }

    fn to_byte_vec(obj: PyObjectRef, what: &str, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        obj.downcast_ref::<crate::vm::builtins::PyBytes>()
            .map(|b| b.as_bytes().to_vec())
            .ok_or_else(|| {
                vm.new_type_error(format!(
                    "{what}() from the underlying stream did not return bytes"
                ))
            })
    }

    fn truncated(vm: &VirtualMachine) -> PyBaseExceptionRef {
        new_unpickling_error(vm, "pickle data was truncated")
    }

    impl ReadState {
        fn set_input(&mut self, data: Vec<u8>, consumed: bool) {
            self.prefetched = if consumed { data.len() } else { 0 };
            self.buf = data;
            self.pos = 0;
        }

        /// Advance the real file position past bytes we only peeked at.
        fn skip_consumed(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            if self.read.is_none() || self.pos <= self.prefetched {
                return Ok(());
            }
            let consumed = self.pos - self.prefetched;
            let read = self.read.clone().expect("file input");
            read.call((consumed,), vm)?;
            self.prefetched = self.pos;
            Ok(())
        }

        /// Refill `buf` with at least `n` bytes from the file if possible.
        /// Returns the number of bytes made available starting at index 0.
        fn fill_from_file(&mut self, n: usize, vm: &VirtualMachine) -> PyResult<usize> {
            self.skip_consumed(vm)?;
            self.frame_end = None;
            if n < PREFETCH
                && let Some(peek) = self.peek.clone()
            {
                match peek.call((PREFETCH,), vm) {
                    Ok(data) => {
                        let data = to_byte_vec(data, "peek", vm)?;
                        let len = data.len();
                        self.set_input(data, false);
                        if n <= len {
                            return Ok(len);
                        }
                    }
                    Err(_) => {
                        // The stream does not really support peek(); stop trying.
                        self.peek = None;
                    }
                }
            }
            let read = self.read.clone().expect("file input");
            let data = to_byte_vec(read.call((n,), vm)?, "read", vm)?;
            let len = data.len();
            self.set_input(data, true);
            Ok(len)
        }

        /// Drop an exhausted frame, or reject a read that would cross its end.
        fn check_frame(&mut self, n: usize, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(end) = self.frame_end {
                if self.pos >= end {
                    self.frame_end = None;
                } else if self.pos + n > end {
                    return Err(new_unpickling_error(
                        vm,
                        "pickle exhausted before end of frame",
                    ));
                }
            }
            Ok(())
        }

        fn read_n(&mut self, n: usize, vm: &VirtualMachine) -> PyResult<&[u8]> {
            self.check_frame(n, vm)?;
            if n <= self.buf.len() - self.pos {
                let start = self.pos;
                self.pos += n;
                return Ok(&self.buf[start..start + n]);
            }
            if self.read.is_none() {
                return Err(truncated(vm));
            }
            if self.fill_from_file(n, vm)? < n {
                return Err(truncated(vm));
            }
            self.pos = n;
            Ok(&self.buf[..n])
        }

        /// Read up to and including the next newline.
        fn read_line(&mut self, vm: &VirtualMachine) -> PyResult<&[u8]> {
            if let Some(end) = self.frame_end
                && self.pos >= end
            {
                self.frame_end = None;
            }
            let limit = self.frame_end.unwrap_or(self.buf.len());
            if let Some(idx) = memchr::memchr(b'\n', &self.buf[self.pos..limit]) {
                let start = self.pos;
                self.pos = start + idx + 1;
                return Ok(&self.buf[start..self.pos]);
            }
            if self.frame_end.is_some() {
                return Err(new_unpickling_error(
                    vm,
                    "pickle exhausted before end of frame",
                ));
            }
            if self.read.is_none() {
                return Err(truncated(vm));
            }
            self.skip_consumed(vm)?;
            let readline = self.readline.clone().expect("file input");
            let data = to_byte_vec(readline.call((), vm)?, "readline", vm)?;
            let len = data.len();
            self.set_input(data, true);
            if len == 0 || self.buf[len - 1] != b'\n' {
                return Err(truncated(vm));
            }
            self.pos = len;
            Ok(&self.buf[..len])
        }

        fn load_frame(&mut self, size: usize, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(end) = self.frame_end {
                if self.pos < end {
                    return Err(new_unpickling_error(
                        vm,
                        "beginning of a new frame before end of current frame",
                    ));
                }
                self.frame_end = None;
            }
            self.read_n(size, vm)?;
            self.pos -= size;
            self.frame_end = Some(self.pos + size);
            Ok(())
        }
    }

    // ---------------------------------------------------------------- memo proxies

    #[pyattr]
    #[pyclass(module = "_pickle", name = "UnpicklerMemoProxy")]
    #[derive(Debug, PyPayload)]
    struct UnpicklerMemoProxy {
        unpickler: PyRef<PyUnpickler>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl UnpicklerMemoProxy {
        #[pymethod]
        fn clear(&self) {
            self.unpickler.memo.write().clear();
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let dict = vm.ctx.new_dict();
            for (i, entry) in self.unpickler.memo.read().iter().enumerate() {
                if let Some(obj) = entry {
                    let key: PyObjectRef = vm.ctx.new_int(i).into();
                    dict.set_item(&*key, obj.clone(), vm)?;
                }
            }
            Ok(dict.into())
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let contents = self.copy(vm)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.types.dict_type.to_owned().into(),
                    vm.ctx.new_tuple(vec![]).into(),
                    vm.ctx.none(),
                    vm.ctx.none(),
                    vm.call_method(&contents, "items", ())?,
                ])
                .into())
        }
    }

    // ---------------------------------------------------------------- Unpickler

    #[derive(Debug)]
    pub(super) struct UnpicklerConfig {
        initialized: bool,
        proto: u8,
        fix_imports: bool,
        encoding: String,
        errors: String,
        buffers: Option<PyObjectRef>,
    }

    impl Default for UnpicklerConfig {
        fn default() -> Self {
            Self {
                initialized: false,
                proto: 0,
                fix_imports: true,
                encoding: "ASCII".to_owned(),
                errors: "strict".to_owned(),
                buffers: None,
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_pickle", name = "Unpickler")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyUnpickler {
        read_state: PyMutex<ReadState>,
        memo: PyRwLock<Vec<Option<PyObjectRef>>>,
        config: PyRwLock<UnpicklerConfig>,
    }

    #[derive(FromArgs)]
    pub(super) struct UnpicklerNewArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        encoding: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        errors: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        buffers: OptionalArg<PyObjectRef>,
    }

    impl Constructor for PyUnpickler {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                read_state: PyMutex::new(ReadState::default()),
                memo: PyRwLock::new(Vec::new()),
                config: PyRwLock::new(UnpicklerConfig::default()),
            })
        }
    }

    impl Initializer for PyUnpickler {
        type Args = UnpicklerNewArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let Some(mut state) = zelf.read_state.try_lock() else {
                return Err(vm.new_runtime_error("Unpickler.__init__() called recursively"));
            };
            let encoding = match args.encoding {
                OptionalArg::Present(o) if !vm.is_none(&o) => o
                    .downcast_ref::<PyStr>()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| vm.new_type_error("encoding must be a string"))?
                    .to_owned(),
                _ => "ASCII".to_owned(),
            };
            let errors = match args.errors {
                OptionalArg::Present(o) if !vm.is_none(&o) => o
                    .downcast_ref::<PyStr>()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| vm.new_type_error("errors must be a string"))?
                    .to_owned(),
                _ => "strict".to_owned(),
            };
            let buffers = match args.buffers {
                OptionalArg::Present(o) if !vm.is_none(&o) => Some(o.get_iter(vm)?.into()),
                _ => None,
            };

            let file = args.file;
            let peek = vm.get_attribute_opt(file.clone(), "peek")?;
            let _readinto = vm.get_attribute_opt(file.clone(), "readinto")?;
            let read = vm.get_attribute_opt(file.clone(), "read")?;
            let readline = vm.get_attribute_opt(file, "readline")?;
            if read.is_none() || readline.is_none() {
                return Err(vm.new_type_error("file must have 'read' and 'readline' attributes"));
            }

            *state = ReadState {
                read,
                readline,
                peek,
                ..ReadState::default()
            };
            zelf.memo.write().clear();
            *zelf.config.write() = UnpicklerConfig {
                initialized: true,
                proto: 0,
                fix_imports: args.fix_imports.unwrap_or(true),
                encoding,
                errors,
                buffers,
            };
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_DICT))]
    impl PyUnpickler {
        #[pymethod]
        fn load(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            load_impl(zelf, vm)
        }

        /// The default hook: no persistent ids are supported.
        #[pymethod]
        fn persistent_load(&self, _pid: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            Err(new_unpickling_error(
                vm,
                "A load persistent id instruction was encountered, but no persistent_load function was specified.",
            ))
        }

        #[pymethod]
        fn find_class(
            zelf: &Py<Self>,
            module: PyObjectRef,
            name: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<PyObjectRef> {
            find_class_impl(zelf, module, name, vm)
        }

        #[pygetset]
        fn memo(zelf: PyRef<Self>, _vm: &VirtualMachine) -> UnpicklerMemoProxy {
            UnpicklerMemoProxy { unpickler: zelf }
        }

        #[pygetset(setter)]
        fn set_memo(zelf: &Py<Self>, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = match value {
                PySetterValue::Assign(v) => v,
                PySetterValue::Delete => {
                    return Err(vm.new_type_error("attribute deletion is not supported"));
                }
            };
            let new_memo = if let Some(proxy) = value.downcast_ref::<UnpicklerMemoProxy>() {
                proxy.unpickler.memo.read().clone()
            } else if let Some(dict) = value.downcast_ref::<PyDict>() {
                let mut memo: Vec<Option<PyObjectRef>> = Vec::new();
                for (key, val) in dict {
                    let idx = key
                        .downcast_ref::<PyInt>()
                        .ok_or_else(|| vm.new_type_error("memo key must be integers"))?;
                    let idx = idx
                        .as_bigint()
                        .to_usize()
                        .ok_or_else(|| vm.new_value_error("memo key must be positive integers."))?;
                    if memo.len() <= idx {
                        memo.resize(idx + 1, None);
                    }
                    memo[idx] = Some(val);
                }
                memo
            } else {
                return Err(vm.new_type_error(format!(
                    "'memo' attribute must be an UnpicklerMemoProxy object or dict, not {}",
                    value.class().name()
                )));
            };
            *zelf.memo.write() = new_memo;
            Ok(())
        }
    }

    fn mapping_get(
        obj: &PyObject,
        key: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(dict) = obj.downcast_ref::<PyDict>() {
            return dict.get_item_opt(key, vm);
        }
        match obj.get_item(key, vm) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.fast_isinstance(vm.ctx.exceptions.key_error) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn getattribute_path(
        obj: PyObjectRef,
        path: &str,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut obj = obj;
        for part in path.split('.') {
            obj = obj.get_attr(&vm.ctx.new_str(part), vm)?;
        }
        Ok(obj)
    }

    fn find_class_impl(
        zelf: &Py<PyUnpickler>,
        module: PyObjectRef,
        name: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let (proto, fix_imports) = {
            let cfg = zelf.config.read();
            (cfg.proto, cfg.fix_imports)
        };
        if let Ok(audit) = vm.sys_module.get_attr("audit", vm) {
            audit.call(
                (
                    vm.ctx.new_str("pickle.find_class"),
                    module.clone(),
                    name.clone(),
                ),
                vm,
            )?;
        }
        let mut module = module
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("module name must be a string"))?;
        let mut name = name
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("global name must be a string"))?;

        if proto < 3 && fix_imports {
            let compat = vm.import("_compat_pickle", 0)?;
            let name_mapping = compat.get_attr("NAME_MAPPING", vm)?;
            let key = vm
                .ctx
                .new_tuple(vec![module.clone().into(), name.clone().into()]);
            if let Some(mapped) = mapping_get(&name_mapping, key.as_object(), vm)? {
                let mapped: PyTupleRef = mapped.downcast().map_err(|_| {
                    vm.new_runtime_error("_compat_pickle.NAME_MAPPING values must be 2-tuples")
                })?;
                if mapped.len() != 2 {
                    return Err(
                        vm.new_runtime_error("_compat_pickle.NAME_MAPPING values must be 2-tuples")
                    );
                }
                module = mapped.as_slice()[0].clone().downcast().map_err(|_| {
                    vm.new_runtime_error(
                        "_compat_pickle.NAME_MAPPING values must be pairs of strings",
                    )
                })?;
                name = mapped.as_slice()[1].clone().downcast().map_err(|_| {
                    vm.new_runtime_error(
                        "_compat_pickle.NAME_MAPPING values must be pairs of strings",
                    )
                })?;
            } else {
                let import_mapping = compat.get_attr("IMPORT_MAPPING", vm)?;
                if let Some(mapped) = mapping_get(&import_mapping, module.as_object(), vm)? {
                    module = mapped.downcast().map_err(|_| {
                        vm.new_runtime_error("_compat_pickle.IMPORT_MAPPING values must be strings")
                    })?;
                }
            }
        }

        vm.import(&module, 0)?;
        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        let module_obj = sys_modules.get_item(&*module, vm)?;

        if proto >= 4
            && name.as_bytes().contains(&b'.')
            && let Some(name_str) = name.to_str()
        {
            getattribute_path(module_obj, name_str, vm).map_err(|e| {
                let name_repr = name
                    .as_object()
                    .repr(vm)
                    .map_or_else(|_| "?".to_owned(), |s| s.to_string_lossy().into_owned());
                let module_repr = module
                    .as_object()
                    .repr(vm)
                    .map_or_else(|_| "?".to_owned(), |s| s.to_string_lossy().into_owned());
                let err = vm.new_attribute_error(format!(
                    "Can't resolve path {name_repr} on module {module_repr}"
                ));
                err.set___context__(Some(e));
                err
            })
        } else {
            module_obj.get_attr(&name, vm)
        }
    }

    fn has_override(zelf: &Py<PyUnpickler>, name: &str, vm: &VirtualMachine) -> PyResult<bool> {
        if !zelf.class().is(PyUnpickler::class(&vm.ctx)) {
            return Ok(true);
        }
        match zelf.as_object().dict() {
            Some(dict) => Ok(dict.get_item_opt(name, vm)?.is_some()),
            None => Ok(false),
        }
    }

    fn decode_string(
        zelf: &Py<PyUnpickler>,
        data: &[u8],
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let (encoding, errors) = {
            let cfg = zelf.config.read();
            (cfg.encoding.clone(), cfg.errors.clone())
        };
        if encoding == "bytes" {
            return Ok(vm.ctx.new_bytes(data.to_vec()).into());
        }
        let errors = vm.ctx.new_utf8_str(errors);
        Ok(vm
            .state
            .codec_registry
            .decode_text(
                vm.ctx.new_bytes(data.to_vec()).into(),
                &encoding,
                Some(errors),
                vm,
            )?
            .into())
    }

    /// `str(data, 'utf-8', 'surrogatepass')`: RustPython strings are WTF-8, which is
    /// exactly the encoding `surrogatepass` produces.
    fn utf8_surrogatepass(data: &[u8], vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        match Wtf8Buf::from_bytes(data.to_vec()) {
            Ok(s) => Ok(vm.ctx.new_str(s).into()),
            Err(data) => {
                let errors = vm.ctx.new_utf8_str("surrogatepass");
                Ok(vm
                    .state
                    .codec_registry
                    .decode_text(vm.ctx.new_bytes(data).into(), "utf-8", Some(errors), vm)?
                    .into())
            }
        }
    }

    fn read_size(data: &[u8], vm: &VirtualMachine, what: &str) -> PyResult<usize> {
        let mut size: u64 = 0;
        for (i, b) in data.iter().enumerate() {
            size |= (*b as u64) << (8 * i);
        }
        usize::try_from(size)
            .ok()
            .filter(|n| isize::try_from(*n).is_ok())
            .ok_or_else(|| {
                new_unpickling_error(
                    vm,
                    format!(
                        "{what} exceeds system's maximum size of {} bytes",
                        isize::MAX
                    ),
                )
            })
    }

    fn load_impl(zelf: &Py<PyUnpickler>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let Some(mut st) = zelf.read_state.try_lock() else {
            return Err(vm.new_runtime_error("Unpickler.load() called recursively"));
        };
        if !zelf.config.read().initialized {
            return Err(new_unpickling_error(
                vm,
                format!(
                    "Unpickler.__init__() was not called by {}.__init__()",
                    zelf.class().name()
                ),
            ));
        }
        zelf.config.write().proto = 0;

        let pers_override = has_override(zelf, "persistent_load", vm)?;
        let find_class_override = has_override(zelf, "find_class", vm)?;

        let mut stack: Vec<PyObjectRef> = Vec::with_capacity(32);
        let mut marks: Vec<usize> = Vec::new();

        macro_rules! fence {
            () => {
                marks.last().copied().unwrap_or(0)
            };
        }
        macro_rules! pop {
            () => {{
                if stack.len() <= fence!() {
                    return Err(new_unpickling_error(vm, "unpickling stack underflow"));
                }
                stack.pop().unwrap()
            }};
        }
        macro_rules! top {
            () => {{
                if stack.len() <= fence!() {
                    return Err(new_unpickling_error(vm, "unpickling stack underflow"));
                }
                stack.last().unwrap().clone()
            }};
        }
        macro_rules! pop_mark {
            () => {{
                let Some(m) = marks.pop() else {
                    return Err(new_unpickling_error(vm, "could not find MARK"));
                };
                stack.split_off(m)
            }};
        }
        macro_rules! find_class {
            ($module:expr, $name:expr) => {{
                let module: PyObjectRef = $module;
                let name: PyObjectRef = $name;
                if find_class_override {
                    zelf.as_object()
                        .get_attr("find_class", vm)?
                        .call((module, name), vm)?
                } else {
                    find_class_impl(zelf, module, name, vm)?
                }
            }};
        }
        macro_rules! persistent_load {
            ($pid:expr) => {{
                let pid: PyObjectRef = $pid;
                if pers_override {
                    zelf.as_object()
                        .get_attr("persistent_load", vm)?
                        .call((pid,), vm)?
                } else {
                    return Err(new_unpickling_error(
                        vm,
                        "A load persistent id instruction was encountered, but no persistent_load function was specified.",
                    ));
                }
            }};
        }

        loop {
            let key = match st.read_n(1, vm) {
                Ok(s) => s[0],
                Err(e) => {
                    return Err(if e.fast_isinstance(&unpickling_error(vm)) {
                        vm.new_eof_error("Ran out of input")
                    } else {
                        e
                    });
                }
            };
            match key {
                STOP => {
                    let value = pop!();
                    // Put the file position right after the pickle so that the
                    // stream can be read from, or unpickled from, again.
                    st.skip_consumed(vm)?;
                    return Ok(value);
                }
                MARK => {
                    marks.push(stack.len());
                }
                NONE => stack.push(vm.ctx.none()),
                NEWTRUE => stack.push(vm.ctx.new_bool(true).into()),
                NEWFALSE => stack.push(vm.ctx.new_bool(false).into()),
                EMPTY_LIST => stack.push(vm.ctx.new_list(vec![]).into()),
                EMPTY_DICT => stack.push(vm.ctx.new_dict().into()),
                EMPTY_TUPLE => stack.push(vm.ctx.new_tuple(vec![]).into()),
                EMPTY_SET => stack.push(PySet::default().into_ref(&vm.ctx).into()),
                PROTO => {
                    let proto = st.read_n(1, vm)?[0];
                    if proto > HIGHEST_PROTOCOL {
                        return Err(
                            vm.new_value_error(format!("unsupported pickle protocol: {proto}"))
                        );
                    }
                    zelf.config.write().proto = proto;
                }
                FRAME => {
                    let arr: [u8; 8] = st.read_n(8, vm)?.try_into().unwrap();
                    let size = read_size(&arr, vm, "FRAME")?;
                    st.load_frame(size, vm)?;
                }
                BININT => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    stack.push(vm.ctx.new_int(i32::from_le_bytes(arr)).into());
                }
                BININT1 => {
                    let b = st.read_n(1, vm)?[0];
                    stack.push(vm.ctx.new_int(b).into());
                }
                BININT2 => {
                    let arr: [u8; 2] = st.read_n(2, vm)?.try_into().unwrap();
                    stack.push(vm.ctx.new_int(u16::from_le_bytes(arr)).into());
                }
                INT => {
                    let line = st.read_line(vm)?;
                    if line.len() < 2 {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let obj = if body == b"01" {
                        vm.ctx.new_bool(true).into()
                    } else if body == b"00" {
                        vm.ctx.new_bool(false).into()
                    } else {
                        parse_int_literal(body, vm)?
                    };
                    stack.push(obj);
                }
                LONG => {
                    let line = st.read_line(vm)?;
                    if line.len() < 2 {
                        return Err(truncated(vm));
                    }
                    let mut body = &line[..line.len() - 1];
                    if body.last() == Some(&b'L') {
                        body = &body[..body.len() - 1];
                    }
                    stack.push(parse_int_literal(body, vm)?);
                }
                LONG1 => {
                    let n = st.read_n(1, vm)?[0] as usize;
                    let data = st.read_n(n, vm)?;
                    stack.push(
                        vm.ctx
                            .new_bigint(&BigInt::from_signed_bytes_le(data))
                            .into(),
                    );
                }
                LONG4 => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let n = i32::from_le_bytes(arr);
                    if n < 0 {
                        return Err(new_unpickling_error(
                            vm,
                            "LONG pickle has negative byte count",
                        ));
                    }
                    let data = st.read_n(n as usize, vm)?;
                    stack.push(
                        vm.ctx
                            .new_bigint(&BigInt::from_signed_bytes_le(data))
                            .into(),
                    );
                }
                FLOAT => {
                    let line = st.read_line(vm)?;
                    if line.len() < 2 {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let s = core::str::from_utf8(body).map_err(|_| {
                        new_unpickling_error(vm, "could not convert string to float")
                    })?;
                    let value: f64 = s.trim().parse().map_err(|_| {
                        new_unpickling_error(vm, "could not convert string to float")
                    })?;
                    stack.push(vm.ctx.new_float(value).into());
                }
                BINFLOAT => {
                    let arr: [u8; 8] = st.read_n(8, vm)?.try_into().unwrap();
                    stack.push(vm.ctx.new_float(f64::from_be_bytes(arr)).into());
                }
                STRING => {
                    let line = st.read_line(vm)?;
                    if line.len() < 3 {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let quote = body[0];
                    if body.len() < 2
                        || (quote != b'"' && quote != b'\'')
                        || *body.last().unwrap() != quote
                    {
                        return Err(new_unpickling_error(
                            vm,
                            "the STRING opcode argument must be quoted",
                        ));
                    }
                    let inner = vm.ctx.new_bytes(body[1..body.len() - 1].to_vec());
                    let codecs = vm.import("_codecs", 0)?;
                    let decoded = codecs
                        .get_attr("escape_decode", vm)?
                        .call((inner,), vm)?
                        .get_item(&0, vm)?;
                    let data = decoded
                        .downcast_ref::<PyBytes>()
                        .ok_or_else(|| vm.new_type_error("escape_decode() did not return bytes"))?
                        .as_bytes()
                        .to_vec();
                    stack.push(decode_string(zelf, &data, vm)?);
                }
                BINSTRING => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let n = i32::from_le_bytes(arr);
                    if n < 0 {
                        return Err(new_unpickling_error(
                            vm,
                            "BINSTRING pickle has negative byte count",
                        ));
                    }
                    let data = st.read_n(n as usize, vm)?.to_vec();
                    stack.push(decode_string(zelf, &data, vm)?);
                }
                SHORT_BINSTRING => {
                    let n = st.read_n(1, vm)?[0] as usize;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(decode_string(zelf, &data, vm)?);
                }
                BINBYTES => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let n = read_size(&arr, vm, "BINBYTES")?;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(vm.ctx.new_bytes(data).into());
                }
                SHORT_BINBYTES => {
                    let n = st.read_n(1, vm)?[0] as usize;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(vm.ctx.new_bytes(data).into());
                }
                BINBYTES8 => {
                    let arr: [u8; 8] = st.read_n(8, vm)?.try_into().unwrap();
                    let n = read_size(&arr, vm, "BINBYTES8")?;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(vm.ctx.new_bytes(data).into());
                }
                BYTEARRAY8 => {
                    let arr: [u8; 8] = st.read_n(8, vm)?.try_into().unwrap();
                    let n = read_size(&arr, vm, "BYTEARRAY8")?;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(PyByteArray::from(data).into_ref(&vm.ctx).into());
                }
                UNICODE => {
                    let line = st.read_line(vm)?;
                    let body = if line.last() == Some(&b'\n') {
                        &line[..line.len() - 1]
                    } else {
                        line
                    };
                    let raw = vm.ctx.new_bytes(body.to_vec());
                    let decoded = vm.state.codec_registry.decode_text(
                        raw.into(),
                        "raw-unicode-escape",
                        None,
                        vm,
                    )?;
                    stack.push(decoded.into());
                }
                BINUNICODE => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let n = read_size(&arr, vm, "BINUNICODE")?;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(utf8_surrogatepass(&data, vm)?);
                }
                SHORT_BINUNICODE => {
                    let n = st.read_n(1, vm)?[0] as usize;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(utf8_surrogatepass(&data, vm)?);
                }
                BINUNICODE8 => {
                    let arr: [u8; 8] = st.read_n(8, vm)?.try_into().unwrap();
                    let n = read_size(&arr, vm, "BINUNICODE8")?;
                    let data = st.read_n(n, vm)?.to_vec();
                    stack.push(utf8_surrogatepass(&data, vm)?);
                }
                TUPLE => {
                    let items = pop_mark!();
                    stack.push(vm.ctx.new_tuple(items).into());
                }
                TUPLE1 => {
                    let a = pop!();
                    stack.push(vm.ctx.new_tuple(vec![a]).into());
                }
                TUPLE2 => {
                    let b = pop!();
                    let a = pop!();
                    stack.push(vm.ctx.new_tuple(vec![a, b]).into());
                }
                TUPLE3 => {
                    let c = pop!();
                    let b = pop!();
                    let a = pop!();
                    stack.push(vm.ctx.new_tuple(vec![a, b, c]).into());
                }
                LIST => {
                    let items = pop_mark!();
                    stack.push(vm.ctx.new_list(items).into());
                }
                DICT => {
                    let items = pop_mark!();
                    if items.len() % 2 != 0 {
                        return Err(new_unpickling_error(vm, "odd number of items for DICT"));
                    }
                    let dict = vm.ctx.new_dict();
                    for pair in items.as_chunks::<2>().0 {
                        dict.set_item(&*pair[0], pair[1].clone(), vm)?;
                    }
                    stack.push(dict.into());
                }
                FROZENSET => {
                    let items = pop_mark!();
                    let set = PyFrozenSet::from_iter(vm, items)?;
                    stack.push(set.into_ref(&vm.ctx).into());
                }
                APPEND => {
                    let value = pop!();
                    let obj = top!();
                    if let Some(list) = obj.downcast_ref::<PyList>() {
                        list.borrow_vec_mut().push(value);
                    } else {
                        vm.call_method(&obj, "append", (value,))?;
                    }
                }
                APPENDS => {
                    let items = pop_mark!();
                    let obj = top!();
                    if let Some(list) = obj.downcast_ref::<PyList>() {
                        list.borrow_vec_mut().extend(items);
                    } else {
                        match vm.get_attribute_opt(obj.clone(), "extend")? {
                            Some(extend) => {
                                extend.call((vm.ctx.new_list(items),), vm)?;
                            }
                            None => {
                                let append = obj.get_attr("append", vm)?;
                                for item in items {
                                    append.call((item,), vm)?;
                                }
                            }
                        }
                    }
                }
                SETITEM => {
                    let value = pop!();
                    let key = pop!();
                    let obj = top!();
                    obj.set_item(&*key, value, vm)?;
                }
                SETITEMS => {
                    let items = pop_mark!();
                    if items.len() % 2 != 0 {
                        return Err(new_unpickling_error(vm, "odd number of items for SETITEMS"));
                    }
                    let obj = top!();
                    if let Some(dict) = obj.downcast_ref::<PyDict>() {
                        for pair in items.as_chunks::<2>().0 {
                            dict.set_item(&*pair[0], pair[1].clone(), vm)?;
                        }
                    } else {
                        for pair in items.as_chunks::<2>().0 {
                            obj.set_item(&*pair[0], pair[1].clone(), vm)?;
                        }
                    }
                }
                ADDITEMS => {
                    let items = pop_mark!();
                    let obj = top!();
                    if let Some(set) = obj.downcast_ref::<PySet>() {
                        for item in items {
                            set.add(item, vm)?;
                        }
                    } else {
                        let add = obj.get_attr("add", vm)?;
                        for item in items {
                            add.call((item,), vm)?;
                        }
                    }
                }
                POP => {
                    if stack.len() > fence!() {
                        stack.pop();
                    } else {
                        let _ = pop_mark!();
                    }
                }
                POP_MARK => {
                    let _ = pop_mark!();
                }
                DUP => {
                    stack.push(top!());
                }
                BINGET => {
                    let i = st.read_n(1, vm)?[0] as usize;
                    stack.push(memo_get(zelf, i, vm)?);
                }
                LONG_BINGET => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let i = u32::from_le_bytes(arr) as usize;
                    stack.push(memo_get(zelf, i, vm)?);
                }
                GET => {
                    let line = st.read_line(vm)?;
                    if line.len() < 2 {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let idx = parse_memo_index(body, vm)?;
                    stack.push(memo_get(zelf, idx, vm)?);
                }
                BINPUT => {
                    let i = st.read_n(1, vm)?[0] as usize;
                    let value = top!();
                    memo_put(zelf, i, value);
                }
                LONG_BINPUT => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let i = u32::from_le_bytes(arr) as usize;
                    let value = top!();
                    memo_put(zelf, i, value);
                }
                PUT => {
                    let line = st.read_line(vm)?;
                    if line.len() < 2 {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let idx = parse_memo_index(body, vm)?;
                    let value = top!();
                    memo_put(zelf, idx, value);
                }
                MEMOIZE => {
                    let value = top!();
                    let idx = zelf.memo.read().len();
                    memo_put(zelf, idx, value);
                }
                PERSID => {
                    let line = st.read_line(vm)?;
                    if line.is_empty() {
                        return Err(truncated(vm));
                    }
                    let body = &line[..line.len() - 1];
                    let pid = core::str::from_utf8(body).map_err(|_| {
                        new_unpickling_error(
                            vm,
                            "persistent IDs in protocol 0 must be ASCII strings",
                        )
                    })?;
                    if !pid.is_ascii() {
                        return Err(new_unpickling_error(
                            vm,
                            "persistent IDs in protocol 0 must be ASCII strings",
                        ));
                    }
                    let pid = vm.ctx.new_str(pid).into();
                    let value = persistent_load!(pid);
                    stack.push(value);
                }
                BINPERSID => {
                    let pid = pop!();
                    let value = persistent_load!(pid);
                    stack.push(value);
                }
                GLOBAL => {
                    let module = {
                        let line = st.read_line(vm)?;
                        if line.len() < 2 {
                            return Err(truncated(vm));
                        }
                        decode_utf8_strict(&line[..line.len() - 1], vm)?
                    };
                    let name = {
                        let line = st.read_line(vm)?;
                        if line.len() < 2 {
                            return Err(truncated(vm));
                        }
                        decode_utf8_strict(&line[..line.len() - 1], vm)?
                    };
                    let cls = find_class!(module, name);
                    stack.push(cls);
                }
                STACK_GLOBAL => {
                    let name = pop!();
                    let module = pop!();
                    if !name.class().is(vm.ctx.types.str_type)
                        || !module.class().is(vm.ctx.types.str_type)
                    {
                        return Err(new_unpickling_error(vm, "STACK_GLOBAL requires str"));
                    }
                    let cls = find_class!(module, name);
                    stack.push(cls);
                }
                INST => {
                    let module = {
                        let line = st.read_line(vm)?;
                        if line.len() < 2 {
                            return Err(truncated(vm));
                        }
                        decode_utf8_strict(&line[..line.len() - 1], vm)?
                    };
                    let name = {
                        let line = st.read_line(vm)?;
                        if line.len() < 2 {
                            return Err(truncated(vm));
                        }
                        decode_utf8_strict(&line[..line.len() - 1], vm)?
                    };
                    let cls = find_class!(module, name);
                    let args = pop_mark!();
                    stack.push(instantiate(cls, args, vm)?);
                }
                OBJ => {
                    let mut args = pop_mark!();
                    if args.is_empty() {
                        return Err(new_unpickling_error(vm, "unpickling stack underflow"));
                    }
                    let cls = args.remove(0);
                    stack.push(instantiate(cls, args, vm)?);
                }
                NEWOBJ => {
                    let args = pop!();
                    let cls = pop!();
                    let args: PyTupleRef = args
                        .downcast()
                        .map_err(|_| vm.new_type_error("NEWOBJ expected an arg tuple."))?;
                    let mut call_args = vec![cls.clone()];
                    call_args.extend(args.as_slice().iter().cloned());
                    let obj = cls.get_attr("__new__", vm)?.call(call_args, vm)?;
                    stack.push(obj);
                }
                NEWOBJ_EX => {
                    let kwargs = pop!();
                    let args = pop!();
                    let cls = pop!();
                    let args: PyTupleRef = args
                        .downcast()
                        .map_err(|_| vm.new_type_error("NEWOBJ_EX expected an arg tuple."))?;
                    let kwargs: PyDictRef = kwargs.downcast().map_err(|_| {
                        vm.new_type_error("NEWOBJ_EX expected a dict of keyword arguments.")
                    })?;
                    let mut func_args = FuncArgs::from(
                        core::iter::once(cls.clone())
                            .chain(args.as_slice().iter().cloned())
                            .collect::<Vec<_>>(),
                    );
                    for (key, value) in &kwargs {
                        let key = key
                            .downcast_ref::<PyStr>()
                            .ok_or_else(|| vm.new_type_error("keywords must be strings"))?
                            .as_wtf8()
                            .to_owned();
                        func_args.kwargs.insert(key, value);
                    }
                    let obj = cls.get_attr("__new__", vm)?.call(func_args, vm)?;
                    stack.push(obj);
                }
                REDUCE => {
                    let args = pop!();
                    let func = top!();
                    let args: PyTupleRef = args
                        .downcast()
                        .map_err(|_| vm.new_type_error("argument list must be a tuple"))?;
                    let value = func.call(args.as_slice().to_vec(), vm)?;
                    let last = stack.len() - 1;
                    stack[last] = value;
                }
                BUILD => {
                    let state = pop!();
                    let inst = top!();
                    load_build(inst, state, vm)?;
                }
                EXT1 => {
                    let code = st.read_n(1, vm)?[0] as i32;
                    stack.push(get_extension(zelf, code, find_class_override, vm)?);
                }
                EXT2 => {
                    let arr: [u8; 2] = st.read_n(2, vm)?.try_into().unwrap();
                    let code = u16::from_le_bytes(arr) as i32;
                    stack.push(get_extension(zelf, code, find_class_override, vm)?);
                }
                EXT4 => {
                    let arr: [u8; 4] = st.read_n(4, vm)?.try_into().unwrap();
                    let code = i32::from_le_bytes(arr);
                    stack.push(get_extension(zelf, code, find_class_override, vm)?);
                }
                NEXT_BUFFER => {
                    let buffers = zelf.config.read().buffers.clone();
                    let Some(buffers) = buffers else {
                        return Err(new_unpickling_error(
                            vm,
                            "pickle stream refers to out-of-band data but no *buffers* argument was given",
                        ));
                    };
                    match PyIter::new(buffers).next(vm)? {
                        PyIterReturn::Return(obj) => stack.push(obj),
                        PyIterReturn::StopIteration(_) => {
                            return Err(new_unpickling_error(vm, "not enough out-of-band buffers"));
                        }
                    }
                }
                READONLY_BUFFER => {
                    let obj = top!();
                    let mv_type: PyObjectRef = vm.ctx.types.memoryview_type.to_owned().into();
                    let view = mv_type.call((obj,), vm)?;
                    if !view.get_attr("readonly", vm)?.try_to_bool(vm)? {
                        let last = stack.len() - 1;
                        stack[last] = vm.call_method(&view, "toreadonly", ())?;
                    }
                }
                _ => {
                    return Err(new_unpickling_error(
                        vm,
                        format!("invalid load key, {}.", ascii_repr(key)),
                    ));
                }
            }
        }
    }

    fn ascii_repr(byte: u8) -> String {
        let s: String = core::iter::once(byte as char)
            .flat_map(char::escape_default)
            .collect();
        format!("'{s}'")
    }

    fn decode_utf8_strict(data: &[u8], vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        match core::str::from_utf8(data) {
            Ok(s) => Ok(vm.ctx.new_str(s).into()),
            Err(_) => Ok(vm
                .state
                .codec_registry
                .decode_text(vm.ctx.new_bytes(data.to_vec()).into(), "utf-8", None, vm)?
                .into()),
        }
    }

    fn parse_int_literal(data: &[u8], vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let s = core::str::from_utf8(data)
            .map_err(|_| vm.new_value_error("invalid literal for int()"))?;
        let trimmed = s.trim();
        match trimmed.parse::<BigInt>() {
            Ok(v) => Ok(vm.ctx.new_bigint(&v).into()),
            Err(_) => Err(vm.new_value_error(format!(
                "invalid literal for int() with base 10: {trimmed:?}"
            ))),
        }
    }

    fn parse_memo_index(data: &[u8], vm: &VirtualMachine) -> PyResult<usize> {
        let s =
            core::str::from_utf8(data).map_err(|_| new_unpickling_error(vm, "bad memo index"))?;
        let value: isize = s
            .trim()
            .parse()
            .map_err(|_| new_unpickling_error(vm, "bad memo index"))?;
        if value < 0 {
            return Err(vm.new_value_error("negative argument"));
        }
        Ok(value as usize)
    }

    fn memo_get(zelf: &Py<PyUnpickler>, idx: usize, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        zelf.memo
            .read()
            .get(idx)
            .and_then(|o| o.clone())
            .ok_or_else(|| new_unpickling_error(vm, format!("Memo value not found at index {idx}")))
    }

    fn memo_put(zelf: &Py<PyUnpickler>, idx: usize, value: PyObjectRef) {
        let mut memo = zelf.memo.write();
        if memo.len() <= idx {
            memo.resize(idx + 1, None);
        }
        memo[idx] = Some(value);
    }

    fn instantiate(
        cls: PyObjectRef,
        args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        if args.is_empty()
            && cls.class().is(vm.ctx.types.type_type)
            && vm
                .get_attribute_opt(cls.clone(), "__getinitargs__")?
                .is_none()
        {
            return vm.call_method(&cls, "__new__", (cls.clone(),));
        }
        cls.call(args, vm)
    }

    fn load_build(inst: PyObjectRef, state: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(setstate) = vm.get_attribute_opt(inst.clone(), "__setstate__")? {
            setstate.call((state,), vm)?;
            return Ok(());
        }
        let (state, slotstate) = match state.downcast_ref::<PyTuple>() {
            Some(t) if t.len() == 2 => (t.as_slice()[0].clone(), Some(t.as_slice()[1].clone())),
            _ => (state, None),
        };
        if !vm.is_none(&state) && state.try_to_bool(vm)? {
            let Some(dict) = state.downcast_ref::<PyDict>() else {
                return Err(new_unpickling_error(vm, "state is not a dictionary"));
            };
            let inst_dict = inst.get_attr("__dict__", vm)?;
            for (key, value) in dict {
                let key = match key.downcast_exact::<PyStr>(vm) {
                    Ok(exact) => vm.ctx.intern_str(exact).to_owned().into(),
                    Err(key) => key,
                };
                inst_dict.set_item(&*key, value, vm)?;
            }
        }
        if let Some(slotstate) = slotstate
            && !vm.is_none(&slotstate)
            && slotstate.try_to_bool(vm)?
        {
            let Some(dict) = slotstate.downcast_ref::<PyDict>() else {
                return Err(new_unpickling_error(vm, "slot state is not a dictionary"));
            };
            for (key, value) in dict {
                let key = key
                    .downcast_ref::<PyStr>()
                    .ok_or_else(|| vm.new_type_error("attribute name must be string"))?;
                inst.set_attr(key, value, vm)?;
            }
        }
        Ok(())
    }

    fn get_extension(
        zelf: &Py<PyUnpickler>,
        code: i32,
        find_class_override: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let copyreg = vm.import("copyreg", 0)?;
        let cache = copyreg.get_attr("_extension_cache", vm)?;
        let code_obj = vm.ctx.new_int(code);
        if let Some(obj) = mapping_get(&cache, code_obj.as_object(), vm)? {
            return Ok(obj);
        }
        let inverted = copyreg.get_attr("_inverted_registry", vm)?;
        let key = mapping_get(&inverted, code_obj.as_object(), vm)?;
        let key = match key {
            Some(k) if k.clone().try_to_bool(vm)? => k,
            _ => {
                if code <= 0 {
                    return Err(new_unpickling_error(vm, "EXT specifies code <= 0"));
                }
                return Err(vm.new_value_error(format!("unregistered extension code {code}")));
            }
        };
        let key: PyTupleRef = key
            .downcast()
            .map_err(|_| vm.new_value_error("_inverted_registry values must be 2-tuples"))?;
        if key.len() != 2 {
            return Err(vm.new_value_error("_inverted_registry values must be 2-tuples"));
        }
        let module = key.as_slice()[0].clone();
        let name = key.as_slice()[1].clone();
        let obj = if find_class_override {
            zelf.as_object()
                .get_attr("find_class", vm)?
                .call((module, name), vm)?
        } else {
            find_class_impl(zelf, module, name, vm)?
        };
        cache.set_item(code_obj.as_object(), obj.clone(), vm)?;
        Ok(obj)
    }

    #[derive(FromArgs)]
    pub(super) struct LoadsArgs {
        #[pyarg(positional)]
        data: PyObjectRef,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        encoding: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        errors: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        buffers: OptionalArg<PyObjectRef>,
    }

    fn unpickler_config(
        fix_imports: OptionalArg<bool>,
        encoding: OptionalArg<PyObjectRef>,
        errors: OptionalArg<PyObjectRef>,
        buffers: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<UnpicklerConfig> {
        let encoding = match encoding {
            OptionalArg::Present(o) if !vm.is_none(&o) => o
                .downcast_ref::<PyStr>()
                .and_then(|s| s.to_str())
                .ok_or_else(|| vm.new_type_error("encoding must be a string"))?
                .to_owned(),
            _ => "ASCII".to_owned(),
        };
        let errors = match errors {
            OptionalArg::Present(o) if !vm.is_none(&o) => o
                .downcast_ref::<PyStr>()
                .and_then(|s| s.to_str())
                .ok_or_else(|| vm.new_type_error("errors must be a string"))?
                .to_owned(),
            _ => "strict".to_owned(),
        };
        let buffers = match buffers {
            OptionalArg::Present(o) if !vm.is_none(&o) => Some(o.get_iter(vm)?.into()),
            _ => None,
        };
        Ok(UnpicklerConfig {
            initialized: true,
            proto: 0,
            fix_imports: fix_imports.unwrap_or(true),
            encoding,
            errors,
            buffers,
        })
    }

    #[pyfunction]
    fn loads(args: LoadsArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if args.data.downcastable::<PyStr>() {
            return Err(vm.new_type_error("Can't load pickle from unicode string"));
        }
        let buf = PyBuffer::from_object(vm, &args.data, crate::vm::protocol::BufferFlags::SIMPLE)?;
        let data = match buf.as_contiguous() {
            Some(bytes) => bytes.to_vec(),
            None => {
                return Err(vm.new_type_error(
                    "a bytes-like object is required, not a non-contiguous buffer",
                ));
            }
        };
        let config = unpickler_config(
            args.fix_imports,
            args.encoding,
            args.errors,
            args.buffers,
            vm,
        )?;
        let unpickler = PyUnpickler {
            read_state: PyMutex::new(ReadState {
                buf: data,
                ..ReadState::default()
            }),
            memo: PyRwLock::new(Vec::new()),
            config: PyRwLock::new(config),
        }
        .into_ref(&vm.ctx);
        load_impl(&unpickler, vm)
    }

    #[derive(FromArgs)]
    pub(super) struct LoadArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        encoding: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        errors: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        buffers: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn load(args: LoadArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let peek = vm.get_attribute_opt(args.file.clone(), "peek")?;
        let read = vm.get_attribute_opt(args.file.clone(), "read")?;
        let readline = vm.get_attribute_opt(args.file, "readline")?;
        if read.is_none() || readline.is_none() {
            return Err(vm.new_type_error("file must have 'read' and 'readline' attributes"));
        }
        let config = unpickler_config(
            args.fix_imports,
            args.encoding,
            args.errors,
            args.buffers,
            vm,
        )?;
        let unpickler = PyUnpickler {
            read_state: PyMutex::new(ReadState {
                read,
                readline,
                peek,
                ..ReadState::default()
            }),
            memo: PyRwLock::new(Vec::new()),
            config: PyRwLock::new(config),
        }
        .into_ref(&vm.ctx);
        load_impl(&unpickler, vm)
    }

    // ---------------------------------------------------------------- Pickler

    const FRAME_HEADER_SIZE: usize = 9;
    const FRAME_SIZE_MIN: usize = 4;
    const FRAME_SIZE_TARGET: usize = 64 * 1024;
    const BATCH_SIZE: usize = 1000;

    /// Hash for object addresses: they are already well distributed, only the low
    /// alignment bits are wasted, so a single multiply is enough.
    #[derive(Default)]
    pub(super) struct PtrHasher(u64);

    impl core::hash::Hasher for PtrHasher {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            for b in bytes {
                self.0 = (self.0 ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
        fn write_usize(&mut self, i: usize) {
            self.0 = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }

    type MemoTable =
        HashMap<usize, (usize, PyObjectRef), core::hash::BuildHasherDefault<PtrHasher>>;

    #[derive(Debug, Default)]
    pub(super) struct Output {
        write: Option<PyObjectRef>,
        buf: Vec<u8>,
        framing: bool,
        frame_start: Option<usize>,
    }

    impl Output {
        fn write_bytes(&mut self, data: &[u8]) {
            if self.framing && self.frame_start.is_none() {
                self.frame_start = Some(self.buf.len());
                self.buf.extend_from_slice(&[0xfe; FRAME_HEADER_SIZE]);
            }
            self.buf.extend_from_slice(data);
        }

        /// Write an opcode plus its payload without building a temporary buffer.
        fn write_pair(&mut self, header: &[u8], data: &[u8]) {
            if self.framing && self.frame_start.is_none() {
                self.frame_start = Some(self.buf.len());
                self.buf.extend_from_slice(&[0xfe; FRAME_HEADER_SIZE]);
            }
            self.buf.reserve(header.len() + data.len());
            self.buf.extend_from_slice(header);
            self.buf.extend_from_slice(data);
        }

        fn commit_frame(&mut self) {
            let Some(start) = self.frame_start.take() else {
                return;
            };
            let frame_len = self.buf.len() - start - FRAME_HEADER_SIZE;
            if frame_len >= FRAME_SIZE_MIN {
                self.buf[start] = FRAME;
                self.buf[start + 1..start + FRAME_HEADER_SIZE]
                    .copy_from_slice(&(frame_len as u64).to_le_bytes());
            } else {
                self.buf.copy_within(start + FRAME_HEADER_SIZE.., start);
                self.buf.truncate(self.buf.len() - FRAME_HEADER_SIZE);
            }
        }

        fn flush_to_file(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            let Some(write) = self.write.clone() else {
                return Ok(());
            };
            if self.buf.is_empty() {
                return Ok(());
            }
            let data = vm.ctx.new_bytes(core::mem::take(&mut self.buf));
            write.call((data,), vm)?;
            Ok(())
        }

        /// Called at every opcode boundary: closes the frame once it is big enough
        /// and, when writing to a file, flushes it so memory stays bounded.
        fn opcode_boundary(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            let Some(start) = self.frame_start else {
                return Ok(());
            };
            if self.buf.len() - start - FRAME_HEADER_SIZE < FRAME_SIZE_TARGET {
                return Ok(());
            }
            self.commit_frame();
            if self.write.is_some() {
                self.flush_to_file(vm)?;
            }
            Ok(())
        }

        /// Write a large binary payload outside of any frame, in its own `write()`
        /// call, so that no copy of it is made.
        fn write_large_bytes(
            &mut self,
            header: &[u8],
            payload: &[u8],
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.commit_frame();
            let framing = self.framing;
            self.framing = false;
            self.write_bytes(header);
            if self.write.is_some() {
                self.flush_to_file(vm)?;
                let write = self.write.clone().unwrap();
                write.call((vm.ctx.new_bytes(payload.to_vec()),), vm)?;
            } else {
                self.buf.extend_from_slice(payload);
            }
            self.framing = framing;
            Ok(())
        }
    }

    #[derive(Debug)]
    pub(super) struct PicklerConfig {
        initialized: bool,
        proto: u8,
        bin: bool,
        fast: bool,
        fix_imports: bool,
        buffer_callback: Option<PyObjectRef>,
    }

    impl Default for PicklerConfig {
        fn default() -> Self {
            Self {
                initialized: false,
                proto: DEFAULT_PROTOCOL,
                bin: true,
                fast: false,
                fix_imports: false,
                buffer_callback: None,
            }
        }
    }

    #[pyattr]
    #[pyclass(module = "_pickle", name = "PicklerMemoProxy")]
    #[derive(Debug, PyPayload)]
    struct PicklerMemoProxy {
        pickler: PyRef<PyPickler>,
    }

    #[pyclass(flags(DISALLOW_INSTANTIATION))]
    impl PicklerMemoProxy {
        #[pymethod]
        fn clear(&self) {
            self.pickler.memo.write().clear();
        }

        #[pymethod]
        fn copy(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let dict = vm.ctx.new_dict();
            #[allow(clippy::iter_over_hash_type)] // insertion order does not matter here
            for (id, (idx, obj)) in self.pickler.memo.read().iter() {
                let key: PyObjectRef = vm.ctx.new_int(*id).into();
                let value: PyObjectRef = vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_int(*idx).into(), obj.clone()])
                    .into();
                dict.set_item(&*key, value, vm)?;
            }
            Ok(dict.into())
        }

        #[pymethod]
        fn __reduce__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            let contents = self.copy(vm)?;
            Ok(vm
                .ctx
                .new_tuple(vec![
                    vm.ctx.types.dict_type.to_owned().into(),
                    vm.ctx.new_tuple(vec![]).into(),
                    vm.ctx.none(),
                    vm.ctx.none(),
                    vm.call_method(&contents, "items", ())?,
                ])
                .into())
        }
    }

    #[pyattr]
    #[pyclass(module = "_pickle", name = "Pickler")]
    #[derive(Debug, PyPayload)]
    pub(super) struct PyPickler {
        out: PyMutex<Output>,
        memo: PyRwLock<MemoTable>,
        config: PyRwLock<PicklerConfig>,
    }

    #[derive(FromArgs)]
    pub(super) struct PicklerNewArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(any, optional)]
        protocol: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        buffer_callback: OptionalArg<PyObjectRef>,
    }

    fn resolve_protocol(protocol: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<u8> {
        let value = match protocol {
            OptionalArg::Present(o) if !vm.is_none(&o) => o
                .try_index(vm)?
                .as_bigint()
                .to_i64()
                .unwrap_or_else(|| i64::from(i32::MAX)),
            _ => i64::from(DEFAULT_PROTOCOL),
        };
        if value < 0 {
            return Ok(HIGHEST_PROTOCOL);
        }
        if value > i64::from(HIGHEST_PROTOCOL) {
            return Err(
                vm.new_value_error(format!("pickle protocol must be <= {HIGHEST_PROTOCOL}"))
            );
        }
        Ok(value as u8)
    }

    fn pickler_config(args: &PicklerNewArgs, vm: &VirtualMachine) -> PyResult<PicklerConfig> {
        let proto = resolve_protocol(args.protocol.clone(), vm)?;
        let buffer_callback = match &args.buffer_callback {
            OptionalArg::Present(o) if !vm.is_none(o) => Some(o.clone()),
            _ => None,
        };
        if buffer_callback.is_some() && proto < 5 {
            return Err(vm.new_value_error("buffer_callback needs protocol >= 5"));
        }
        Ok(PicklerConfig {
            initialized: true,
            proto,
            bin: proto >= 1,
            fast: false,
            fix_imports: args.fix_imports.unwrap_or(true) && proto < 3,
            buffer_callback,
        })
    }

    impl Constructor for PyPickler {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                out: PyMutex::new(Output::default()),
                memo: PyRwLock::new(MemoTable::default()),
                config: PyRwLock::new(PicklerConfig::default()),
            })
        }
    }

    impl Initializer for PyPickler {
        type Args = PicklerNewArgs;

        fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let Some(mut out) = zelf.out.try_lock() else {
                return Err(vm.new_runtime_error("Pickler.__init__() called recursively"));
            };
            let config = pickler_config(&args, vm)?;
            let write = args.file.get_attr("write", vm).map_err(|e| {
                if e.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                    vm.new_type_error("file must have a 'write' attribute")
                } else {
                    e
                }
            })?;
            *out = Output {
                write: Some(write),
                ..Output::default()
            };
            zelf.memo.write().clear();
            *zelf.config.write() = config;
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_DICT))]
    impl PyPickler {
        #[pymethod]
        fn dump(zelf: &Py<Self>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            dump_impl(zelf, obj, vm)
        }

        #[pymethod]
        fn clear_memo(&self) {
            self.memo.write().clear();
        }

        /// The default hook: nothing has a persistent id.
        #[pymethod]
        fn persistent_id(&self, _obj: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        #[pygetset]
        fn memo(zelf: PyRef<Self>) -> PicklerMemoProxy {
            PicklerMemoProxy { pickler: zelf }
        }

        #[pygetset(setter)]
        fn set_memo(zelf: &Py<Self>, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            let value = match value {
                PySetterValue::Assign(v) => v,
                PySetterValue::Delete => {
                    return Err(vm.new_type_error("attribute deletion is not supported"));
                }
            };
            let new_memo = if let Some(proxy) = value.downcast_ref::<PicklerMemoProxy>() {
                proxy.pickler.memo.read().clone()
            } else if let Some(dict) = value.downcast_ref::<PyDict>() {
                let mut memo = MemoTable::default();
                for (key, val) in dict {
                    let id = key
                        .downcast_ref::<PyInt>()
                        .and_then(|i| i.as_bigint().to_usize())
                        .ok_or_else(|| vm.new_type_error("memo key must be integers"))?;
                    let pair: PyTupleRef = val
                        .downcast()
                        .map_err(|_| vm.new_type_error("'memo' values must be 2-item tuples"))?;
                    if pair.len() != 2 {
                        return Err(vm.new_type_error("'memo' values must be 2-item tuples"));
                    }
                    let idx = pair.as_slice()[0]
                        .downcast_ref::<PyInt>()
                        .and_then(|i| i.as_bigint().to_usize())
                        .ok_or_else(|| vm.new_type_error("memo key must be integers"))?;
                    memo.insert(id, (idx, pair.as_slice()[1].clone()));
                }
                memo
            } else {
                return Err(vm.new_type_error(format!(
                    "'memo' attribute must be a PicklerMemoProxy object or dict, not {}",
                    value.class().name()
                )));
            };
            *zelf.memo.write() = new_memo;
            Ok(())
        }

        #[pygetset]
        fn fast(&self) -> bool {
            self.config.read().fast
        }

        #[pygetset(setter)]
        fn set_fast(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    self.config.write().fast = v.try_to_bool(vm)?;
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_type_error("attribute deletion is not supported"))
                }
            }
        }

        #[pygetset]
        fn bin(&self) -> bool {
            self.config.read().bin
        }

        #[pygetset(setter)]
        fn set_bin(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
            match value {
                PySetterValue::Assign(v) => {
                    self.config.write().bin = v.try_to_bool(vm)?;
                    Ok(())
                }
                PySetterValue::Delete => {
                    Err(vm.new_type_error("attribute deletion is not supported"))
                }
            }
        }
    }

    // ---------------------------------------------------------------- saving

    fn add_note(err: PyBaseExceptionRef, note: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let _ = err.clone().add_note(vm.ctx.new_str(note), vm);
        err
    }

    /// `pickle._T`: the name used for a type in error messages.
    fn type_repr_name(cls: &Py<PyType>, vm: &VirtualMachine) -> String {
        let qualname = cls
            .as_object()
            .get_attr("__qualname__", vm)
            .ok()
            .and_then(|o| {
                o.downcast_ref::<PyStr>()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| cls.name().to_string());
        let module = cls
            .as_object()
            .get_attr("__module__", vm)
            .ok()
            .and_then(|o| {
                o.downcast_ref::<PyStr>()
                    .map(|s| s.to_string_lossy().into_owned())
            });
        match module.as_deref() {
            None | Some("builtins" | "__main__") => qualname,
            Some(m) => format!("{m}.{qualname}"),
        }
    }

    fn obj_type_name(obj: &PyObject, vm: &VirtualMachine) -> String {
        type_repr_name(obj.class(), vm)
    }

    fn safe_repr(obj: &PyObject, vm: &VirtualMachine) -> String {
        obj.repr(vm).map_or_else(
            |_| "<unrepresentable>".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
    }

    struct SaveCtx<'a> {
        zelf: &'a Py<PyPickler>,
        out: &'a mut Output,
        proto: u8,
        bin: bool,
        fast: bool,
        fix_imports: bool,
        pers_func: Option<PyObjectRef>,
        dispatch_table: Option<PyObjectRef>,
        reducer_override: Option<PyObjectRef>,
        buffer_callback: Option<PyObjectRef>,
        copyreg: Option<PyObjectRef>,
    }

    impl SaveCtx<'_> {
        fn write(&mut self, data: &[u8]) {
            self.out.write_bytes(data);
        }

        fn write_pair(&mut self, header: &[u8], data: &[u8]) {
            self.out.write_pair(header, data);
        }

        fn write_put(&mut self, idx: usize) {
            if self.proto >= 4 {
                self.write(&[MEMOIZE]);
            } else if self.bin && idx < 256 {
                self.write(&[BINPUT, idx as u8]);
            } else if self.bin {
                let mut buf = [LONG_BINPUT, 0, 0, 0, 0];
                buf[1..].copy_from_slice(&(idx as u32).to_le_bytes());
                self.write(&buf);
            } else {
                self.write_pair(&[PUT], format!("{idx}\n").as_bytes());
            }
        }

        fn copyreg(&mut self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            if let Some(m) = &self.copyreg {
                return Ok(m.clone());
            }
            let m = vm.import("copyreg", 0)?;
            self.copyreg = Some(m.clone());
            Ok(m)
        }

        fn get_opcode(&self, idx: usize) -> Vec<u8> {
            if self.bin {
                if idx < 256 {
                    vec![BINGET, idx as u8]
                } else {
                    let mut v = vec![LONG_BINGET];
                    v.extend_from_slice(&(idx as u32).to_le_bytes());
                    v
                }
            } else {
                let mut v = vec![GET];
                v.extend_from_slice(idx.to_string().as_bytes());
                v.push(b'\n');
                v
            }
        }

        fn memo_lookup(&self, obj: &PyObject) -> Option<usize> {
            self.zelf
                .memo
                .read()
                .get(&obj.get_id())
                .map(|(idx, _)| *idx)
        }

        fn memoize(&mut self, obj: &PyObject) {
            if self.fast {
                return;
            }
            let idx = {
                let mut memo = self.zelf.memo.write();
                let idx = memo.len();
                memo.insert(obj.get_id(), (idx, obj.to_owned()));
                idx
            };
            self.write_put(idx);
        }

        fn write_memo_get(&mut self, idx: usize) {
            if self.bin && idx < 256 {
                self.write(&[BINGET, idx as u8]);
            } else if self.bin {
                let mut buf = [LONG_BINGET, 0, 0, 0, 0];
                buf[1..].copy_from_slice(&(idx as u32).to_le_bytes());
                self.write(&buf);
            } else {
                self.write_pair(&[GET], format!("{idx}\n").as_bytes());
            }
        }

        // -- atoms ------------------------------------------------------

        fn save_bool(&mut self, value: bool) {
            if self.proto >= 2 {
                self.write(&[if value { NEWTRUE } else { NEWFALSE }]);
            } else if value {
                self.write(b"I01\n");
            } else {
                self.write(b"I00\n");
            }
        }

        fn save_long(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let int = obj.downcast_ref::<PyInt>().expect("int");
            let value = int.as_bigint();
            if self.bin
                && let Some(v) = value.to_i64()
            {
                if (0..=0xff).contains(&v) {
                    self.write(&[BININT1, v as u8]);
                    return Ok(());
                }
                if (0..=0xffff).contains(&v) {
                    let mut buf = [BININT2, 0, 0];
                    buf[1..].copy_from_slice(&(v as u16).to_le_bytes());
                    self.write(&buf);
                    return Ok(());
                }
                if let Ok(v) = i32::try_from(v) {
                    let mut buf = [BININT, 0, 0, 0, 0];
                    buf[1..].copy_from_slice(&v.to_le_bytes());
                    self.write(&buf);
                    return Ok(());
                }
            }
            if self.proto >= 2 {
                let encoded = if value.is_zero() {
                    Vec::new()
                } else {
                    value.to_signed_bytes_le()
                };
                let n = encoded.len();
                let mut header = if n < 256 {
                    vec![LONG1, n as u8]
                } else {
                    let mut v = vec![LONG4];
                    let n = i32::try_from(n)
                        .map_err(|_| new_pickling_error(vm, "int too large to pickle"))?;
                    v.extend_from_slice(&n.to_le_bytes());
                    v
                };
                header.extend_from_slice(&encoded);
                self.write(&header);
                return Ok(());
            }
            let text = value.to_string();
            if value.to_i64().is_some_and(|v| i32::try_from(v).is_ok()) {
                self.write_pair(&[INT], format!("{text}\n").as_bytes());
            } else {
                self.write_pair(&[LONG], format!("{text}L\n").as_bytes());
            }
            Ok(())
        }

        fn save_float(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let value = obj.downcast_ref::<PyFloat>().expect("float").to_f64();
            if self.bin {
                let mut buf = [BINFLOAT, 0, 0, 0, 0, 0, 0, 0, 0];
                buf[1..].copy_from_slice(&value.to_be_bytes());
                self.write(&buf);
            } else {
                let text = obj.repr(vm)?;
                let mut buf = vec![FLOAT];
                buf.extend_from_slice(text.to_string_lossy().as_bytes());
                buf.push(b'\n');
                self.write(&buf);
            }
            Ok(())
        }

        // -- bytes and strings ------------------------------------------

        fn save_bytes_no_memo(&mut self, data: &[u8], vm: &VirtualMachine) -> PyResult<()> {
            let n = data.len();
            if n <= 0xff {
                self.write_pair(&[SHORT_BINBYTES, n as u8], data);
            } else if n > 0xffff_ffff && self.proto >= 4 {
                let mut header = vec![BINBYTES8];
                header.extend_from_slice(&(n as u64).to_le_bytes());
                self.out.write_large_bytes(&header, data, vm)?;
            } else if n >= FRAME_SIZE_TARGET {
                let mut header = vec![BINBYTES];
                header.extend_from_slice(&(n as u32).to_le_bytes());
                self.out.write_large_bytes(&header, data, vm)?;
            } else {
                let mut header = [BINBYTES, 0, 0, 0, 0];
                header[1..].copy_from_slice(&(n as u32).to_le_bytes());
                self.write_pair(&header, data);
            }
            Ok(())
        }

        fn save_bytearray_no_memo(&mut self, data: &[u8], vm: &VirtualMachine) -> PyResult<()> {
            let n = data.len();
            let mut header = [BYTEARRAY8, 0, 0, 0, 0, 0, 0, 0, 0];
            header[1..].copy_from_slice(&(n as u64).to_le_bytes());
            if n >= FRAME_SIZE_TARGET {
                self.out.write_large_bytes(&header, data, vm)?;
            } else {
                self.write_pair(&header, data);
            }
            Ok(())
        }

        fn save_bytes(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let data = obj.downcast_ref::<PyBytes>().expect("bytes");
            if self.proto < 3 {
                if data.as_bytes().is_empty() {
                    let bytes_type: PyObjectRef = vm.ctx.types.bytes_type.to_owned().into();
                    return self.save_reduce(
                        bytes_type,
                        vm.ctx.new_tuple(vec![]).into(),
                        None,
                        None,
                        None,
                        None,
                        Some(obj),
                        vm,
                    );
                }
                let latin1: String = data.as_bytes().iter().map(|b| *b as char).collect();
                let codecs = vm.import("codecs", 0)?;
                let encode = codecs.get_attr("encode", vm)?;
                let args = vm.ctx.new_tuple(vec![
                    vm.ctx.new_str(latin1).into(),
                    vm.ctx.new_str("latin1").into(),
                ]);
                return self.save_reduce(
                    encode,
                    args.into(),
                    None,
                    None,
                    None,
                    None,
                    Some(obj),
                    vm,
                );
            }
            let data = data.as_bytes().to_vec();
            self.save_bytes_no_memo(&data, vm)?;
            self.memoize(obj);
            Ok(())
        }

        fn save_bytearray(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let data = obj
                .downcast_ref::<PyByteArray>()
                .expect("bytearray")
                .borrow_buf()
                .to_vec();
            if self.proto < 5 {
                let bytearray_type: PyObjectRef = vm.ctx.types.bytearray_type.to_owned().into();
                let args = if data.is_empty() {
                    vm.ctx.new_tuple(vec![])
                } else {
                    vm.ctx.new_tuple(vec![vm.ctx.new_bytes(data).into()])
                };
                return self.save_reduce(
                    bytearray_type,
                    args.into(),
                    None,
                    None,
                    None,
                    None,
                    Some(obj),
                    vm,
                );
            }
            self.save_bytearray_no_memo(&data, vm)?;
            self.memoize(obj);
            Ok(())
        }

        fn save_str(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let s = obj.downcast_ref::<PyStr>().expect("str");
            if self.bin {
                // RustPython strings are stored as WTF-8, which is exactly what
                // `str.encode('utf-8', 'surrogatepass')` produces.
                let encoded = s.as_bytes();
                let n = encoded.len();
                if n <= 0xff && self.proto >= 4 {
                    self.write_pair(&[SHORT_BINUNICODE, n as u8], encoded);
                } else if n > 0xffff_ffff && self.proto >= 4 {
                    let mut header = vec![BINUNICODE8];
                    header.extend_from_slice(&(n as u64).to_le_bytes());
                    let payload = encoded.to_vec();
                    self.out.write_large_bytes(&header, &payload, vm)?;
                } else if n >= FRAME_SIZE_TARGET {
                    let mut header = vec![BINUNICODE];
                    header.extend_from_slice(&(n as u32).to_le_bytes());
                    let payload = encoded.to_vec();
                    self.out.write_large_bytes(&header, &payload, vm)?;
                } else {
                    let mut header = [BINUNICODE, 0, 0, 0, 0];
                    header[1..].copy_from_slice(&(n as u32).to_le_bytes());
                    self.write_pair(&header, encoded);
                }
            } else {
                let mut buf = vec![UNICODE];
                raw_unicode_escape(s.as_wtf8(), &mut buf);
                buf.push(b'\n');
                self.write(&buf);
            }
            self.memoize(obj);
            Ok(())
        }

        // -- containers -------------------------------------------------

        fn save_tuple(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let tuple = obj.downcast_ref::<PyTuple>().expect("tuple");
            let n = tuple.len();
            if n == 0 {
                if self.bin {
                    self.write(&[EMPTY_TUPLE]);
                } else {
                    self.write(&[MARK, TUPLE]);
                }
                return Ok(());
            }
            let items = tuple.as_slice();
            if n <= 3 && self.proto >= 2 {
                for (i, item) in items.iter().enumerate() {
                    self.save(item, false, vm).map_err(|e| {
                        add_note(
                            e,
                            format!("when serializing {} item {i}", obj_type_name(obj, vm)),
                            vm,
                        )
                    })?;
                }
                if let Some(idx) = self.memo_lookup(obj) {
                    let mut buf = vec![POP; n];
                    buf.extend_from_slice(&self.get_opcode(idx));
                    self.write(&buf);
                } else {
                    self.write(&[[EMPTY_TUPLE, TUPLE1, TUPLE2, TUPLE3][n]]);
                    self.memoize(obj);
                }
                return Ok(());
            }
            self.write(&[MARK]);
            for (i, item) in items.iter().enumerate() {
                self.save(item, false, vm).map_err(|e| {
                    add_note(
                        e,
                        format!("when serializing {} item {i}", obj_type_name(obj, vm)),
                        vm,
                    )
                })?;
            }
            if let Some(idx) = self.memo_lookup(obj) {
                let get = self.get_opcode(idx);
                let mut buf = if self.bin {
                    vec![POP_MARK]
                } else {
                    vec![POP; n + 1]
                };
                buf.extend_from_slice(&get);
                self.write(&buf);
                return Ok(());
            }
            self.write(&[TUPLE]);
            self.memoize(obj);
            Ok(())
        }

        fn save_list(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if self.bin {
                self.write(&[EMPTY_LIST]);
            } else {
                self.write(&[MARK, LIST]);
            }
            self.memoize(obj);
            let list = obj.downcast_ref::<PyList>().expect("list");
            self.batch_appends_list(list, obj, vm)
        }

        fn save_item(
            &mut self,
            item: &PyObject,
            index: usize,
            obj: &PyObject,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.save(item, false, vm).map_err(|e| {
                add_note(
                    e,
                    format!("when serializing {} item {index}", obj_type_name(obj, vm)),
                    vm,
                )
            })
        }

        fn batch_appends_list(
            &mut self,
            list: &Py<PyList>,
            obj: &PyObject,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let initial_len = list.borrow_vec().len();
            let mut start = 0usize;
            loop {
                let batch: Vec<PyObjectRef> = {
                    let items = list.borrow_vec();
                    if start >= items.len() {
                        break;
                    }
                    items[start..items.len().min(start + BATCH_SIZE)].to_vec()
                };
                if batch.is_empty() {
                    break;
                }
                if !self.bin {
                    for (k, item) in batch.iter().enumerate() {
                        self.save_item(item, start + k, obj, vm)?;
                        self.write(&[APPEND]);
                    }
                } else if batch.len() != 1 {
                    self.write(&[MARK]);
                    for (k, item) in batch.iter().enumerate() {
                        self.save_item(item, start + k, obj, vm)?;
                    }
                    self.write(&[APPENDS]);
                } else {
                    self.save_item(&batch[0], start, obj, vm)?;
                    self.write(&[APPEND]);
                }
                start += batch.len();
                if list.borrow_vec().len() != initial_len {
                    return Err(vm.new_runtime_error("list changed size during iteration"));
                }
            }
            Ok(())
        }

        fn batch_appends_iter(
            &mut self,
            items: &PyObject,
            obj: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name = |vm: &VirtualMachine| obj.map_or_else(String::new, |o| obj_type_name(o, vm));
            let iter = items.to_owned().get_iter(vm)?;
            let mut index = 0usize;
            let mut pending: Vec<PyObjectRef> = Vec::new();
            let mut exhausted = false;
            while !exhausted {
                pending.clear();
                while pending.len() < BATCH_SIZE {
                    match iter.next(vm)? {
                        PyIterReturn::Return(o) => pending.push(o),
                        PyIterReturn::StopIteration(_) => {
                            exhausted = true;
                            break;
                        }
                    }
                }
                if pending.is_empty() {
                    break;
                }
                if !self.bin {
                    for item in &pending {
                        self.save(item, false, vm).map_err(|e| {
                            add_note(e, format!("when serializing {} item {index}", name(vm)), vm)
                        })?;
                        self.write(&[APPEND]);
                        index += 1;
                    }
                } else if pending.len() != 1 {
                    self.write(&[MARK]);
                    for item in &pending {
                        self.save(item, false, vm).map_err(|e| {
                            add_note(e, format!("when serializing {} item {index}", name(vm)), vm)
                        })?;
                        index += 1;
                    }
                    self.write(&[APPENDS]);
                } else {
                    self.save(&pending[0], false, vm).map_err(|e| {
                        add_note(e, format!("when serializing {} item {index}", name(vm)), vm)
                    })?;
                    self.write(&[APPEND]);
                    index += 1;
                }
            }
            Ok(())
        }

        fn save_dict(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if self.bin {
                self.write(&[EMPTY_DICT]);
            } else {
                self.write(&[MARK, DICT]);
            }
            self.memoize(obj);
            let dict = obj.downcast_ref::<PyDict>().expect("dict");
            let initial_len = dict.__len__();
            let entries: Vec<(PyObjectRef, PyObjectRef)> = dict.into_iter().collect();
            let mut start = 0usize;
            while start < entries.len() {
                let end = entries.len().min(start + BATCH_SIZE);
                let batch = &entries[start..end];
                if !self.bin {
                    for (k, v) in batch {
                        self.save(k, false, vm)?;
                        self.save_value(v, k, obj, vm)?;
                        self.write(&[SETITEM]);
                    }
                } else if batch.len() != 1 {
                    self.write(&[MARK]);
                    for (k, v) in batch {
                        self.save(k, false, vm)?;
                        self.save_value(v, k, obj, vm)?;
                    }
                    self.write(&[SETITEMS]);
                } else {
                    let (k, v) = &batch[0];
                    self.save(k, false, vm)?;
                    self.save_value(v, k, obj, vm)?;
                    self.write(&[SETITEM]);
                }
                start = end;
                if dict.__len__() != initial_len {
                    return Err(vm.new_runtime_error("dictionary changed size during iteration"));
                }
            }
            Ok(())
        }

        fn save_value(
            &mut self,
            value: &PyObject,
            key: &PyObject,
            obj: &PyObject,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.save(value, false, vm).map_err(|e| {
                add_note(
                    e,
                    format!(
                        "when serializing {} item {}",
                        obj_type_name(obj, vm),
                        safe_repr(key, vm)
                    ),
                    vm,
                )
            })
        }

        fn batch_setitems_iter(
            &mut self,
            items: &PyObject,
            obj: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let iter = items.to_owned().get_iter(vm)?;
            let mut pending: Vec<(PyObjectRef, PyObjectRef)> = Vec::new();
            let mut exhausted = false;
            while !exhausted {
                pending.clear();
                while pending.len() < BATCH_SIZE {
                    match iter.next(vm)? {
                        PyIterReturn::Return(o) => {
                            let pair: PyTupleRef = o.downcast().map_err(|_| {
                                vm.new_type_error("dict items iterator must return 2-tuples")
                            })?;
                            if pair.len() != 2 {
                                return Err(
                                    vm.new_type_error("dict items iterator must return 2-tuples")
                                );
                            }
                            pending.push((pair.as_slice()[0].clone(), pair.as_slice()[1].clone()));
                        }
                        PyIterReturn::StopIteration(_) => {
                            exhausted = true;
                            break;
                        }
                    }
                }
                if pending.is_empty() {
                    break;
                }
                let single = pending.len() == 1;
                if self.bin && !single {
                    self.write(&[MARK]);
                }
                for (k, v) in &pending {
                    self.save(k, false, vm)?;
                    match obj {
                        Some(o) => self.save_value(v, k, o, vm)?,
                        None => self.save(v, false, vm)?,
                    }
                    if !self.bin || single {
                        self.write(&[SETITEM]);
                    }
                }
                if self.bin && !single {
                    self.write(&[SETITEMS]);
                }
            }
            Ok(())
        }

        fn save_set(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let set = obj.downcast_ref::<PySet>().expect("set");
            if self.proto < 4 {
                let elements = vm.ctx.new_list(set.elements());
                let set_type: PyObjectRef = vm.ctx.types.set_type.to_owned().into();
                return self.save_reduce(
                    set_type,
                    vm.ctx.new_tuple(vec![elements.into()]).into(),
                    None,
                    None,
                    None,
                    None,
                    Some(obj),
                    vm,
                );
            }
            self.write(&[EMPTY_SET]);
            self.memoize(obj);
            let elements = set.elements();
            for batch in elements.chunks(BATCH_SIZE) {
                self.write(&[MARK]);
                for item in batch {
                    self.save(item, false, vm).map_err(|e| {
                        add_note(
                            e,
                            format!("when serializing {} element", obj_type_name(obj, vm)),
                            vm,
                        )
                    })?;
                }
                self.write(&[ADDITEMS]);
            }
            Ok(())
        }

        fn save_frozenset(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let set = obj.downcast_ref::<PyFrozenSet>().expect("frozenset");
            if self.proto < 4 {
                let elements = vm.ctx.new_list(set.elements());
                let frozenset_type: PyObjectRef = vm.ctx.types.frozenset_type.to_owned().into();
                return self.save_reduce(
                    frozenset_type,
                    vm.ctx.new_tuple(vec![elements.into()]).into(),
                    None,
                    None,
                    None,
                    None,
                    Some(obj),
                    vm,
                );
            }
            self.write(&[MARK]);
            for item in set.elements() {
                self.save(&item, false, vm).map_err(|e| {
                    add_note(
                        e,
                        format!("when serializing {} element", obj_type_name(obj, vm)),
                        vm,
                    )
                })?;
            }
            if let Some(idx) = self.memo_lookup(obj) {
                let mut buf = vec![POP_MARK];
                buf.extend_from_slice(&self.get_opcode(idx));
                self.write(&buf);
                return Ok(());
            }
            self.write(&[FROZENSET]);
            self.memoize(obj);
            Ok(())
        }
    }

    fn raw_unicode_escape(s: &crate::common::wtf8::Wtf8, buf: &mut Vec<u8>) {
        for cp in s.code_points() {
            let c = cp.to_u32();
            match c {
                0x5c => buf.extend_from_slice(b"\\u005c"),
                0x00 => buf.extend_from_slice(b"\\u0000"),
                0x0a => buf.extend_from_slice(b"\\u000a"),
                0x0d => buf.extend_from_slice(b"\\u000d"),
                0x1a => buf.extend_from_slice(b"\\u001a"),
                0x01..=0xff => buf.push(c as u8),
                0x100..=0xffff => {
                    buf.extend_from_slice(format!("\\u{c:04x}").as_bytes());
                }
                _ => {
                    buf.extend_from_slice(format!("\\U{c:08x}").as_bytes());
                }
            }
        }
    }

    impl SaveCtx<'_> {
        fn save(&mut self, obj: &PyObject, pers_save: bool, vm: &VirtualMachine) -> PyResult<()> {
            vm.with_recursion("while pickling an object", || {
                self.save_inner(obj, pers_save, vm)
            })
        }

        fn save_inner(
            &mut self,
            obj: &PyObject,
            pers_save: bool,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.out.opcode_boundary(vm)?;

            if !pers_save && let Some(pers) = self.pers_func.clone() {
                let pid = pers.call((obj.to_owned(),), vm)?;
                if !vm.is_none(&pid) {
                    return self.save_pers(&pid, vm);
                }
            }

            if vm.is_none(obj) {
                self.write(&[NONE]);
                return Ok(());
            }
            let cls = obj.class();
            if cls.is(vm.ctx.types.bool_type) {
                self.save_bool(obj.to_owned().try_to_bool(vm)?);
                return Ok(());
            }
            if cls.is(vm.ctx.types.int_type) {
                return self.save_long(obj, vm);
            }
            if cls.is(vm.ctx.types.float_type) {
                return self.save_float(obj, vm);
            }

            if let Some(idx) = self.memo_lookup(obj) {
                self.write_memo_get(idx);
                return Ok(());
            }

            if cls.is(vm.ctx.types.bytes_type) {
                return self.save_bytes(obj, vm);
            }
            if cls.is(vm.ctx.types.str_type) {
                return self.save_str(obj, vm);
            }
            if cls.is(vm.ctx.types.dict_type) {
                return self.save_dict(obj, vm);
            }
            if cls.is(vm.ctx.types.set_type) {
                return self.save_set(obj, vm);
            }
            if cls.is(vm.ctx.types.frozenset_type) {
                return self.save_frozenset(obj, vm);
            }
            if cls.is(vm.ctx.types.list_type) {
                return self.save_list(obj, vm);
            }
            if cls.is(vm.ctx.types.tuple_type) {
                return self.save_tuple(obj, vm);
            }
            if cls.is(vm.ctx.types.bytearray_type) {
                return self.save_bytearray(obj, vm);
            }
            if obj.downcastable::<PyPickleBuffer>() {
                return self.save_picklebuffer(obj, vm);
            }

            let mut rv: Option<PyObjectRef> = None;
            if let Some(ro) = self.reducer_override.clone() {
                let value = ro.call((obj.to_owned(),), vm)?;
                if !value.is(&vm.ctx.not_implemented) {
                    rv = Some(value);
                }
            }

            if rv.is_none() {
                if cls.is(vm.ctx.types.type_type) {
                    return self.save_type(obj, vm);
                }
                if cls.is(vm.ctx.types.function_type) {
                    return self.save_global(obj, None, vm);
                }
                let reduce = match self.dispatch_table.clone() {
                    Some(dt) => mapping_get(&dt, cls.as_object(), vm)?,
                    None => {
                        let copyreg = self.copyreg(vm)?;
                        let dt = copyreg.get_attr("dispatch_table", vm)?;
                        mapping_get(&dt, cls.as_object(), vm)?
                    }
                };
                if let Some(reduce) = reduce {
                    rv = Some(reduce.call((obj.to_owned(),), vm)?);
                } else if cls.fast_issubclass(vm.ctx.types.type_type) {
                    return self.save_global(obj, None, vm);
                } else if let Some(reduce) =
                    vm.get_attribute_opt(obj.to_owned(), "__reduce_ex__")?
                {
                    rv = Some(reduce.call((self.proto,), vm)?);
                } else if let Some(reduce) = vm.get_attribute_opt(obj.to_owned(), "__reduce__")? {
                    rv = Some(reduce.call((), vm)?);
                } else {
                    return Err(new_pickling_error(
                        vm,
                        format!("Can't pickle {} object", type_repr_name(cls, vm)),
                    ));
                }
            }

            let rv = rv.expect("reduce value");
            if rv.downcastable::<PyStr>() {
                return self.save_global(obj, Some(rv), vm);
            }
            self.save_reduce_value(rv, obj, vm).map_err(|e| {
                add_note(
                    e,
                    format!("when serializing {} object", obj_type_name(obj, vm)),
                    vm,
                )
            })
        }

        fn save_reduce_value(
            &mut self,
            rv: PyObjectRef,
            obj: &PyObject,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let tuple: PyTupleRef = rv.downcast().map_err(|rv| {
                new_pickling_error(
                    vm,
                    format!(
                        "__reduce__ must return a string or tuple, not {}",
                        obj_type_name(&rv, vm)
                    ),
                )
            })?;
            let items = tuple.as_slice();
            if !(2..=6).contains(&items.len()) {
                return Err(new_pickling_error(
                    vm,
                    "tuple returned by __reduce__ must contain 2 through 6 elements",
                ));
            }
            let opt = |i: usize| -> Option<PyObjectRef> {
                items.get(i).filter(|o| !vm.is_none(o)).cloned()
            };
            self.save_reduce(
                items[0].clone(),
                items[1].clone(),
                opt(2),
                opt(3),
                opt(4),
                opt(5),
                Some(obj),
                vm,
            )
        }

        #[allow(clippy::too_many_arguments)]
        fn save_reduce(
            &mut self,
            func: PyObjectRef,
            args: PyObjectRef,
            state: Option<PyObjectRef>,
            listitems: Option<PyObjectRef>,
            dictitems: Option<PyObjectRef>,
            state_setter: Option<PyObjectRef>,
            obj: Option<&PyObject>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if !func.is_callable() {
                return Err(new_pickling_error(
                    vm,
                    format!(
                        "first item of the tuple returned by __reduce__ must be callable, not {}",
                        obj_type_name(&func, vm)
                    ),
                ));
            }
            let args: PyTupleRef = args.downcast().map_err(|args| {
                new_pickling_error(
                    vm,
                    format!(
                        "second item of the tuple returned by __reduce__ must be a tuple, not {}",
                        obj_type_name(&args, vm)
                    ),
                )
            })?;

            if let Some(setter) = &state_setter
                && !setter.is_callable()
            {
                return Err(new_pickling_error(
                    vm,
                    format!(
                        "sixth item of the tuple returned by __reduce__ must be callable, not {}",
                        obj_type_name(setter, vm)
                    ),
                ));
            }
            if let Some(items) = &listitems
                && !PyIter::check(items)
            {
                return Err(new_pickling_error(
                    vm,
                    format!(
                        "fourth item of the tuple returned by __reduce__ must be an iterator, not {}",
                        obj_type_name(items, vm)
                    ),
                ));
            }
            if let Some(items) = &dictitems
                && !PyIter::check(items)
            {
                return Err(new_pickling_error(
                    vm,
                    format!(
                        "fifth item of the tuple returned by __reduce__ must be an iterator, not {}",
                        obj_type_name(items, vm)
                    ),
                ));
            }

            let obj_name =
                |vm: &VirtualMachine| obj.map_or_else(String::new, |o| obj_type_name(o, vm));
            let func_name = vm
                .get_attribute_opt(func.clone(), "__name__")?
                .and_then(|o| {
                    o.downcast_ref::<PyStr>()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_default();

            if self.proto >= 2 && func_name == "__newobj_ex__" {
                let parts = args.as_slice();
                if parts.len() != 3 {
                    return Err(vm.new_value_error(format!(
                        "not enough values to unpack (expected 3, got {})",
                        parts.len()
                    )));
                }
                let (cls, newargs, kwargs) = (parts[0].clone(), parts[1].clone(), parts[2].clone());
                if vm.get_attribute_opt(cls.clone(), "__new__")?.is_none() {
                    return Err(new_pickling_error(
                        vm,
                        "first argument to __newobj_ex__() has no __new__",
                    ));
                }
                if let Some(o) = obj {
                    let obj_class = o.get_attr("__class__", vm)?;
                    if !cls.is(&obj_class) {
                        return Err(new_pickling_error(
                            vm,
                            format!(
                                "first argument to __newobj_ex__() must be {}, not {}",
                                safe_repr(&obj_class, vm),
                                safe_repr(&cls, vm)
                            ),
                        ));
                    }
                }
                if !newargs.downcastable::<PyTuple>() {
                    return Err(new_pickling_error(
                        vm,
                        format!(
                            "second argument to __newobj_ex__() must be a tuple, not {}",
                            obj_type_name(&newargs, vm)
                        ),
                    ));
                }
                if !kwargs.downcastable::<PyDict>() {
                    return Err(new_pickling_error(
                        vm,
                        format!(
                            "third argument to __newobj_ex__() must be a dict, not {}",
                            obj_type_name(&kwargs, vm)
                        ),
                    ));
                }
                if self.proto >= 4 {
                    self.save(&cls, false, vm).map_err(|e| {
                        add_note(e, format!("when serializing {} class", obj_name(vm)), vm)
                    })?;
                    self.save(&newargs, false, vm)
                        .and_then(|()| self.save(&kwargs, false, vm))
                        .map_err(|e| {
                            add_note(
                                e,
                                format!("when serializing {} __new__ arguments", obj_name(vm)),
                                vm,
                            )
                        })?;
                    self.write(&[NEWOBJ_EX]);
                } else {
                    let functools = vm.import("functools", 0)?;
                    let partial = functools.get_attr("partial", vm)?;
                    let new = cls.get_attr("__new__", vm)?;
                    let newargs: PyTupleRef = newargs.downcast().expect("tuple");
                    let kwargs_dict: PyDictRef = kwargs.downcast().expect("dict");
                    let mut call_args = FuncArgs::from(
                        core::iter::once(new)
                            .chain(core::iter::once(cls.clone()))
                            .chain(newargs.as_slice().iter().cloned())
                            .collect::<Vec<_>>(),
                    );
                    for (k, v) in &kwargs_dict {
                        let k = k
                            .downcast_ref::<PyStr>()
                            .ok_or_else(|| vm.new_type_error("keywords must be strings"))?
                            .as_wtf8()
                            .to_owned();
                        call_args.kwargs.insert(k, v);
                    }
                    let func = partial.call(call_args, vm)?;
                    self.save(&func, false, vm).map_err(|e| {
                        add_note(
                            e,
                            format!("when serializing {} reconstructor", obj_name(vm)),
                            vm,
                        )
                    })?;
                    let empty: PyObjectRef = vm.ctx.new_tuple(vec![]).into();
                    self.save(&empty, false, vm)?;
                    self.write(&[REDUCE]);
                }
            } else if self.proto >= 2 && func_name == "__newobj__" {
                let parts = args.as_slice();
                if parts.is_empty() {
                    return Err(vm.new_index_error("tuple index out of range"));
                }
                let cls = parts[0].clone();
                if vm.get_attribute_opt(cls.clone(), "__new__")?.is_none() {
                    return Err(new_pickling_error(
                        vm,
                        "first argument to __newobj__() has no __new__",
                    ));
                }
                if let Some(o) = obj {
                    let obj_class = o.get_attr("__class__", vm)?;
                    if !cls.is(&obj_class) {
                        return Err(new_pickling_error(
                            vm,
                            format!(
                                "first argument to __newobj__() must be {}, not {}",
                                safe_repr(&obj_class, vm),
                                safe_repr(&cls, vm)
                            ),
                        ));
                    }
                }
                let rest: PyObjectRef = vm.ctx.new_tuple(parts[1..].to_vec()).into();
                self.save(&cls, false, vm).map_err(|e| {
                    add_note(e, format!("when serializing {} class", obj_name(vm)), vm)
                })?;
                self.save(&rest, false, vm).map_err(|e| {
                    add_note(
                        e,
                        format!("when serializing {} __new__ arguments", obj_name(vm)),
                        vm,
                    )
                })?;
                self.write(&[NEWOBJ]);
            } else {
                self.save(&func, false, vm).map_err(|e| {
                    add_note(
                        e,
                        format!("when serializing {} reconstructor", obj_name(vm)),
                        vm,
                    )
                })?;
                let args_obj: PyObjectRef = args.into();
                self.save(&args_obj, false, vm).map_err(|e| {
                    add_note(
                        e,
                        format!("when serializing {} reconstructor arguments", obj_name(vm)),
                        vm,
                    )
                })?;
                self.write(&[REDUCE]);
            }

            if let Some(o) = obj {
                if let Some(idx) = self.memo_lookup(o) {
                    let mut buf = vec![POP];
                    buf.extend_from_slice(&self.get_opcode(idx));
                    self.write(&buf);
                } else {
                    self.memoize(o);
                }
            }

            if let Some(listitems) = listitems {
                self.batch_appends_iter(&listitems, obj, vm)?;
            }
            if let Some(dictitems) = dictitems {
                self.batch_setitems_iter(&dictitems, obj, vm)?;
            }
            if let Some(state) = state {
                match state_setter {
                    None => {
                        self.save(&state, false, vm).map_err(|e| {
                            add_note(e, format!("when serializing {} state", obj_name(vm)), vm)
                        })?;
                        self.write(&[BUILD]);
                    }
                    Some(setter) => {
                        self.save(&setter, false, vm).map_err(|e| {
                            add_note(
                                e,
                                format!("when serializing {} state setter", obj_name(vm)),
                                vm,
                            )
                        })?;
                        if let Some(o) = obj {
                            self.save(o, false, vm)?;
                        } else {
                            let none = vm.ctx.none();
                            self.save(&none, false, vm)?;
                        }
                        self.save(&state, false, vm).map_err(|e| {
                            add_note(e, format!("when serializing {} state", obj_name(vm)), vm)
                        })?;
                        self.write(&[TUPLE2, REDUCE, POP]);
                    }
                }
            }
            Ok(())
        }

        fn save_pers(&mut self, pid: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if self.bin {
                self.save(pid, true, vm)?;
                self.write(&[BINPERSID]);
                return Ok(());
            }
            let text = pid.str(vm)?;
            let ascii = text.to_str().filter(|s| s.is_ascii()).ok_or_else(|| {
                new_pickling_error(vm, "persistent IDs in protocol 0 must be ASCII strings")
            })?;
            let mut buf = vec![PERSID];
            buf.extend_from_slice(ascii.as_bytes());
            buf.push(b'\n');
            self.write(&buf);
            Ok(())
        }

        fn save_type(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            let type_type: PyObjectRef = vm.ctx.types.type_type.to_owned().into();
            let singleton = if obj.is(vm.ctx.types.none_type.as_object()) {
                Some(vm.ctx.none())
            } else if obj.is(vm.ctx.types.not_implemented_type.as_object()) {
                Some(vm.ctx.not_implemented())
            } else if obj.is(vm.ctx.types.ellipsis_type.as_object()) {
                Some(vm.ctx.ellipsis.clone().into())
            } else {
                None
            };
            match singleton {
                Some(value) => self.save_reduce(
                    type_type,
                    vm.ctx.new_tuple(vec![value]).into(),
                    None,
                    None,
                    None,
                    None,
                    Some(obj),
                    vm,
                ),
                None => self.save_global(obj, None, vm),
            }
        }

        fn save_picklebuffer(&mut self, obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
            if self.proto < 5 {
                return Err(new_pickling_error(
                    vm,
                    "PickleBuffer can only be pickled with protocol >= 5",
                ));
            }
            let pb = obj.downcast_ref::<PyPickleBuffer>().expect("PickleBuffer");
            let buffer = pb.get(vm)?;
            if !buffer.desc.is_contiguous() {
                return Err(new_pickling_error(
                    vm,
                    "PickleBuffer can not be pickled when pointing to a non-contiguous buffer",
                ));
            }
            let in_band = match self.buffer_callback.clone() {
                Some(callback) => callback.call((obj.to_owned(),), vm)?.try_to_bool(vm)?,
                None => true,
            };
            if in_band {
                let data = buffer
                    .as_contiguous()
                    .map(|b| b.to_vec())
                    .expect("contiguous");
                if buffer.desc.readonly {
                    self.save_bytes_no_memo(&data, vm)?;
                } else {
                    self.save_bytearray_no_memo(&data, vm)?;
                }
                self.memoize(obj);
            } else {
                self.write(&[NEXT_BUFFER]);
                if buffer.desc.readonly {
                    self.write(&[READONLY_BUFFER]);
                }
            }
            Ok(())
        }

        fn save_global(
            &mut self,
            obj: &PyObject,
            name: Option<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let name: PyRef<PyStr> = match name {
                Some(n) => n
                    .downcast()
                    .map_err(|_| new_pickling_error(vm, "expected a string as the global name"))?,
                None => {
                    let value = match vm.get_attribute_opt(obj.to_owned(), "__qualname__")? {
                        Some(q) if !vm.is_none(&q) => q,
                        _ => obj.get_attr("__name__", vm)?,
                    };
                    value
                        .downcast()
                        .map_err(|_| new_pickling_error(vm, "the global name must be a string"))?
                }
            };
            let module_name = whichmodule(obj, &name, vm)?;

            if self.proto >= 2 {
                let copyreg = self.copyreg(vm)?;
                let registry = copyreg.get_attr("_extension_registry", vm)?;
                let key: PyObjectRef = vm
                    .ctx
                    .new_tuple(vec![module_name.clone().into(), name.clone().into()])
                    .into();
                if let Some(code) = mapping_get(&registry, &key, vm)? {
                    let code = code
                        .downcast_ref::<PyInt>()
                        .and_then(|i| i.as_bigint().to_i64())
                        .ok_or_else(|| vm.new_value_error("extension code must be an integer"))?;
                    if code <= 0xff {
                        if code == 0 {
                            return Err(vm.new_runtime_error("extension code 0 is out of range"));
                        }
                        self.write(&[EXT1, code as u8]);
                    } else if code <= 0xffff {
                        let mut buf = [EXT2, 0, 0];
                        buf[1..].copy_from_slice(&(code as u16).to_le_bytes());
                        self.write(&buf);
                    } else {
                        let mut buf = [EXT4, 0, 0, 0, 0];
                        buf[1..].copy_from_slice(&(code as i32).to_le_bytes());
                        self.write(&buf);
                    }
                    return Ok(());
                }
            }

            if self.proto >= 4 {
                let module_obj: PyObjectRef = module_name.clone().into();
                self.save(&module_obj, false, vm)?;
                let name_obj: PyObjectRef = name.clone().into();
                self.save(&name_obj, false, vm)?;
                self.write(&[STACK_GLOBAL]);
            } else if name.as_bytes().contains(&b'.') {
                let mut parts = split_dotted(&name, vm);
                let head = parts.remove(0);
                let dotted = parts;
                let getattr = vm.builtins.get_attr("getattr", vm)?;
                for _ in &dotted {
                    self.save(&getattr, false, vm)?;
                    if self.proto < 2 {
                        self.write(&[MARK]);
                    }
                }
                self.save_toplevel_by_name(&module_name, &head, vm)?;
                for attrname in &dotted {
                    let attr: PyObjectRef = attrname.clone().into();
                    self.save(&attr, false, vm)?;
                    if self.proto < 2 {
                        self.write(&[TUPLE]);
                    } else {
                        self.write(&[TUPLE2]);
                    }
                    self.write(&[REDUCE]);
                }
            } else {
                self.save_toplevel_by_name(&module_name, &name, vm)?;
            }

            self.memoize(obj);
            Ok(())
        }

        fn save_toplevel_by_name(
            &mut self,
            module_name: &Py<PyStr>,
            name: &Py<PyStr>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut module: PyRef<PyStr> = module_name.to_owned();
            let mut global_name: PyRef<PyStr> = name.to_owned();
            let encoding =
                if self.proto >= 3 {
                    "utf-8"
                } else {
                    if self.fix_imports {
                        let compat = vm.import("_compat_pickle", 0)?;
                        let r_name = compat.get_attr("REVERSE_NAME_MAPPING", vm)?;
                        let key: PyObjectRef = vm
                            .ctx
                            .new_tuple(vec![module.clone().into(), global_name.clone().into()])
                            .into();
                        if let Some(mapped) = mapping_get(&r_name, &key, vm)? {
                            let pair: PyTupleRef = mapped.downcast().map_err(|_| {
                                vm.new_runtime_error("bad REVERSE_NAME_MAPPING entry")
                            })?;
                            module = pair.as_slice()[0].clone().downcast().map_err(|_| {
                                vm.new_runtime_error("bad REVERSE_NAME_MAPPING entry")
                            })?;
                            global_name = pair.as_slice()[1].clone().downcast().map_err(|_| {
                                vm.new_runtime_error("bad REVERSE_NAME_MAPPING entry")
                            })?;
                        } else {
                            let r_import = compat.get_attr("REVERSE_IMPORT_MAPPING", vm)?;
                            let key: PyObjectRef = module.clone().into();
                            if let Some(mapped) = mapping_get(&r_import, &key, vm)? {
                                module = mapped.downcast().map_err(|_| {
                                    vm.new_runtime_error("bad REVERSE_IMPORT_MAPPING entry")
                                })?;
                            }
                        }
                    }
                    "ascii"
                };
            let proto = self.proto;
            let module_bytes = vm
                .state
                .codec_registry
                .encode_text(module.clone(), encoding, None, vm)
                .map_err(|e| {
                    let err = new_pickling_error(
                        vm,
                        format!(
                            "can't pickle module identifier {} using pickle protocol {proto}",
                            safe_repr(module.as_object(), vm)
                        ),
                    );
                    err.set___context__(Some(e));
                    err
                })?;
            let name_bytes = vm
                .state
                .codec_registry
                .encode_text(global_name.clone(), encoding, None, vm)
                .map_err(|e| {
                    let err = new_pickling_error(
                        vm,
                        format!(
                            "can't pickle global identifier {} using pickle protocol {proto}",
                            safe_repr(global_name.as_object(), vm)
                        ),
                    );
                    err.set___context__(Some(e));
                    err
                })?;
            let mut buf = vec![GLOBAL];
            buf.extend_from_slice(module_bytes.as_bytes());
            buf.push(b'\n');
            buf.extend_from_slice(name_bytes.as_bytes());
            buf.push(b'\n');
            self.write(&buf);
            Ok(())
        }
    }

    fn getattr_path(
        obj: &PyObject,
        path: &[PyRef<PyStr>],
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut current = obj.to_owned();
        for part in path {
            current = current.get_attr(part, vm)?;
        }
        Ok(current)
    }

    /// Split a dotted name without going through `str`: names may contain lone
    /// surrogates, which a lossy conversion would destroy.
    fn split_dotted(name: &Py<PyStr>, vm: &VirtualMachine) -> Vec<PyRef<PyStr>> {
        name.as_bytes()
            .split(|b| *b == b'.')
            .map(|part| {
                let text = Wtf8Buf::from_bytes(part.to_vec()).unwrap_or_default();
                vm.ctx.new_str(text)
            })
            .collect()
    }

    fn whichmodule(
        obj: &PyObject,
        name: &Py<PyStr>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyStr>> {
        let full = name.to_string_lossy().into_owned();
        let dotted = split_dotted(name, vm);
        if dotted.iter().any(|p| p.as_bytes() == b"<locals>") {
            return Err(new_pickling_error(
                vm,
                format!("Can't pickle local object {}", safe_repr(obj, vm)),
            ));
        }
        let sys_modules = vm.sys_module.get_attr("modules", vm)?;
        let module_attr = vm.get_attribute_opt(obj.to_owned(), "__module__")?;
        let module_name: PyRef<PyStr> = match module_attr {
            Some(m) if !vm.is_none(&m) => m
                .downcast()
                .map_err(|_| new_pickling_error(vm, "__module__ must be a string"))?,
            _ => {
                let mut found: Option<PyRef<PyStr>> = None;
                let snapshot = vm.call_method(&sys_modules, "copy", ())?;
                let items = vm.call_method(&snapshot, "items", ())?;
                let iter = items.get_iter(vm)?;
                while let PyIterReturn::Return(item) = iter.next(vm)? {
                    let pair: PyTupleRef = match item.downcast() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if pair.len() != 2 {
                        continue;
                    }
                    let (key, module) = (pair.as_slice()[0].clone(), pair.as_slice()[1].clone());
                    let Some(key_str) = key.downcast_ref::<PyStr>() else {
                        continue;
                    };
                    let key_text = key_str.to_string_lossy();
                    if key_text == "__main__" || key_text == "__mp_main__" || vm.is_none(&module) {
                        continue;
                    }
                    if let Ok(candidate) = getattr_path(&module, &dotted, vm)
                        && candidate.is(obj)
                    {
                        found = Some(key.downcast().expect("str"));
                        break;
                    }
                }
                match found {
                    Some(m) => return Ok(m),
                    None => vm.ctx.new_str("__main__"),
                }
            }
        };

        let imported = vm.import(&module_name, 0).and_then(|_| {
            let m: PyObjectRef = module_name.clone().into();
            sys_modules.get_item(&*m, vm)
        });
        let module = match imported {
            Ok(m) => m,
            Err(e) => {
                let msg = e
                    .as_object()
                    .str(vm)
                    .map_or_else(|_| String::new(), |s| s.to_string_lossy().into_owned());
                let err =
                    new_pickling_error(vm, format!("Can't pickle {}: {msg}", safe_repr(obj, vm)));
                err.set___context__(Some(e));
                return Err(err);
            }
        };
        match getattr_path(&module, &dotted, vm) {
            Ok(candidate) if candidate.is(obj) => Ok(module_name),
            Ok(_) => Err(new_pickling_error(
                vm,
                format!(
                    "Can't pickle {}: it's not the same object as {}.{full}",
                    safe_repr(obj, vm),
                    module_name.to_string_lossy()
                ),
            )),
            Err(e) => {
                let err = new_pickling_error(
                    vm,
                    format!(
                        "Can't pickle {}: it's not found as {}.{full}",
                        safe_repr(obj, vm),
                        module_name.to_string_lossy()
                    ),
                );
                err.set___context__(Some(e));
                Err(err)
            }
        }
    }

    /// Look up a hook such as `persistent_id` that a subclass or the instance may
    /// override, returning `None` when the base class implementation is in effect.
    fn resolve_hook(
        obj: &PyObject,
        base: &Py<PyType>,
        name: &'static str,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyObjectRef>> {
        if let Some(dict) = obj.dict()
            && let Some(value) = dict.get_item_opt(name, vm)?
        {
            return Ok(Some(value));
        }
        let interned = vm.ctx.intern_str(name);
        let own = obj.class().lookup_ref(interned, vm);
        let base_attr = base.lookup_ref(interned, vm);
        match (own, base_attr) {
            (Some(a), Some(b)) if a.is(&b) => Ok(None),
            (None, None) => Ok(None),
            _ => Ok(Some(obj.get_attr(name, vm)?)),
        }
    }

    fn dump_impl(zelf: &Py<PyPickler>, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let Some(mut out) = zelf.out.try_lock() else {
            return Err(vm.new_runtime_error("Pickler.dump() called recursively"));
        };
        let (initialized, proto, bin, fast, fix_imports, buffer_callback) = {
            let cfg = zelf.config.read();
            (
                cfg.initialized,
                cfg.proto,
                cfg.bin,
                cfg.fast,
                cfg.fix_imports,
                cfg.buffer_callback.clone(),
            )
        };
        if !initialized {
            return Err(new_pickling_error(
                vm,
                format!(
                    "Pickler.__init__() was not called by {}.__init__()",
                    zelf.class().name()
                ),
            ));
        }
        let pers_func = resolve_hook(
            zelf.as_object(),
            PyPickler::class(&vm.ctx),
            "persistent_id",
            vm,
        )?;
        let reducer_override = vm.get_attribute_opt(zelf.to_owned().into(), "reducer_override")?;
        let dispatch_table = vm.get_attribute_opt(zelf.to_owned().into(), "dispatch_table")?;

        out.buf.clear();
        out.frame_start = None;
        out.framing = false;
        if proto >= 2 {
            out.write_bytes(&[PROTO, proto]);
        }
        if proto >= 4 {
            out.framing = true;
        }
        let mut ctx = SaveCtx {
            zelf,
            out: &mut out,
            proto,
            bin,
            fast,
            fix_imports,
            pers_func,
            dispatch_table,
            reducer_override,
            buffer_callback,
            copyreg: None,
        };
        ctx.save(&obj, false, vm)?;
        ctx.write(&[STOP]);
        out.commit_frame();
        out.flush_to_file(vm)?;
        Ok(())
    }

    #[derive(FromArgs)]
    pub(super) struct DumpArgs {
        #[pyarg(any)]
        obj: PyObjectRef,
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(any, optional)]
        protocol: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        buffer_callback: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn dump(args: DumpArgs, vm: &VirtualMachine) -> PyResult<()> {
        let new_args = PicklerNewArgs {
            file: args.file,
            protocol: args.protocol,
            fix_imports: args.fix_imports,
            buffer_callback: args.buffer_callback,
        };
        let config = pickler_config(&new_args, vm)?;
        let write = new_args.file.get_attr("write", vm).map_err(|e| {
            if e.fast_isinstance(vm.ctx.exceptions.attribute_error) {
                vm.new_type_error("file must have a 'write' attribute")
            } else {
                e
            }
        })?;
        let pickler = PyPickler {
            out: PyMutex::new(Output {
                write: Some(write),
                ..Output::default()
            }),
            memo: PyRwLock::new(MemoTable::default()),
            config: PyRwLock::new(config),
        }
        .into_ref(&vm.ctx);
        dump_impl(&pickler, args.obj, vm)
    }

    #[derive(FromArgs)]
    pub(super) struct DumpsArgs {
        #[pyarg(any)]
        obj: PyObjectRef,
        #[pyarg(any, optional)]
        protocol: OptionalArg<PyObjectRef>,
        #[pyarg(named, optional)]
        fix_imports: OptionalArg<bool>,
        #[pyarg(named, optional)]
        buffer_callback: OptionalArg<PyObjectRef>,
    }

    #[pyfunction]
    fn dumps(args: DumpsArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let new_args = PicklerNewArgs {
            file: vm.ctx.none(),
            protocol: args.protocol,
            fix_imports: args.fix_imports,
            buffer_callback: args.buffer_callback,
        };
        let config = pickler_config(&new_args, vm)?;
        let pickler = PyPickler {
            out: PyMutex::new(Output::default()),
            memo: PyRwLock::new(MemoTable::default()),
            config: PyRwLock::new(config),
        }
        .into_ref(&vm.ctx);
        dump_impl(&pickler, args.obj, vm)?;
        let data = core::mem::take(&mut pickler.out.lock().buf);
        Ok(vm.ctx.new_bytes(data).into())
    }
}
