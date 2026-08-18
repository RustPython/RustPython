// spell-checker:ignore pyfrozen pycomplex
pub(crate) use decl::module_def;

#[pymodule(name = "marshal")]
mod decl {
    use crate::builtins::code::{CodeObject, Literal, PyVmBag};
    use crate::class::StaticType;
    use crate::common::wtf8::Wtf8;
    use crate::{
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
        builtins::{
            PyBaseExceptionRef, PyBool, PyByteArray, PyBytes, PyCode, PyComplex, PyDict,
            PyEllipsis, PyFloat, PyFrozenSet, PyInt, PyList, PyNone, PySet, PyStopIteration, PyStr,
            PyTuple,
        },
        convert::ToPyObject,
        function::{ArgBytesLike, OptionalArg},
        object::{AsObject, PyPayload},
    };
    use core::cell::RefCell;
    use malachite_bigint::BigInt;
    use num_traits::Zero;
    use rustpython_compiler_core::marshal::{self, DumpableValue};

    #[pyattr(name = "version")]
    use marshal::FORMAT_VERSION;

    pub struct DumpError;

    impl marshal::Dumpable for PyObjectRef {
        type Error = DumpError;
        type Constant = Literal;

        fn with_dump<R>(
            &self,
            f: impl FnOnce(DumpableValue<'_, Self>) -> R,
        ) -> Result<R, Self::Error> {
            if self.is(PyStopIteration::static_type()) {
                return Ok(f(DumpableValue::StopIter));
            }

            let ret = match_class!(match self {
                PyNone => f(DumpableValue::None),
                PyEllipsis => f(DumpableValue::Ellipsis),
                ref pyint @ PyInt => {
                    if self.class().is(PyBool::static_type()) {
                        f(DumpableValue::Boolean(!pyint.as_bigint().is_zero()))
                    } else {
                        f(DumpableValue::Integer(pyint.as_bigint()))
                    }
                }
                ref pyfloat @ PyFloat => {
                    f(DumpableValue::Float(pyfloat.to_f64()))
                }
                ref pycomplex @ PyComplex => {
                    f(DumpableValue::Complex(pycomplex.to_complex64()))
                }
                ref pystr @ PyStr => {
                    f(DumpableValue::Str(pystr.as_wtf8()))
                }
                ref pylist @ PyList => {
                    f(DumpableValue::List(&pylist.borrow_vec()))
                }
                ref pyset @ PySet => {
                    let elements = pyset.elements();
                    f(DumpableValue::Set(&elements))
                }
                ref pyfrozen @ PyFrozenSet => {
                    let elements = pyfrozen.elements();
                    f(DumpableValue::Frozenset(&elements))
                }
                ref pytuple @ PyTuple => {
                    f(DumpableValue::Tuple(pytuple.as_slice()))
                }
                ref pydict @ PyDict => {
                    let entries = pydict.into_iter().collect::<Vec<_>>();
                    f(DumpableValue::Dict(&entries))
                }
                ref bytes @ PyBytes => {
                    f(DumpableValue::Bytes(bytes.as_bytes()))
                }
                ref bytes @ PyByteArray => {
                    f(DumpableValue::Bytes(&bytes.borrow_buf()))
                }
                ref co @ PyCode => {
                    f(DumpableValue::Code(co))
                }
                _ => return Err(DumpError),
            });
            Ok(ret)
        }
    }

    #[derive(FromArgs)]
    struct DumpsArgs {
        value: PyObjectRef,
        #[pyarg(any, optional)]
        _version: OptionalArg<i32>,
        #[pyarg(named, default = true)]
        allow_code: bool,
    }

    #[pyfunction]
    fn dumps(args: DumpsArgs, vm: &VirtualMachine) -> PyResult<PyBytes> {
        let DumpsArgs {
            value,
            allow_code,
            _version,
        } = args;
        let version = _version.unwrap_or(marshal::FORMAT_VERSION as i32);

        if let Ok(audit) = vm.sys_module.get_attr("audit", vm) {
            audit.call(
                (vm.ctx.new_str("marshal.dumps"), value.clone(), version),
                vm,
            )?;
        }

        check_exact_type(&value, vm)?;
        let mut buf = Vec::new();
        let mut refs = if version >= 3 {
            Some(WriterRefTable::new())
        } else {
            None
        };
        write_object(&mut buf, &value, &mut refs, version, allow_code, vm)?;
        Ok(PyBytes::from(buf))
    }

    struct WriterRefEntry {
        idx: u32,
        /// Set between `reserve` and `complete` for the object kinds whose
        /// immutable representation cannot be rebuilt from a back-reference.
        incomplete: bool,
    }

    struct WriterRefTable {
        map: std::collections::HashMap<usize, WriterRefEntry>,
        next_idx: u32,
    }

    impl WriterRefTable {
        fn new() -> Self {
            Self {
                map: std::collections::HashMap::new(),
                next_idx: 0,
            }
        }
        /// `w_ref`: write a back-reference to an object already in the table.
        /// Reaching an entry that is still being written is a recursion the
        /// reader could not rebuild, so it is an error rather than a `TYPE_REF`.
        fn try_ref(&mut self, buf: &mut Vec<u8>, obj: &PyObjectRef) -> Result<bool, ()> {
            use marshal::Write;
            let Some(entry) = self.map.get(&obj.get_id()) else {
                return Ok(false);
            };
            if entry.incomplete {
                return Err(());
            }
            buf.write_u8(b'r');
            buf.write_u32(entry.idx);
            Ok(true)
        }
        fn reserve(&mut self, obj: &PyObjectRef, incomplete: bool) -> u32 {
            let idx = self.next_idx;
            self.map
                .insert(obj.get_id(), WriterRefEntry { idx, incomplete });
            self.next_idx += 1;
            idx
        }
        /// `w_complete`: the object's contents are on the stream, so a later
        /// occurrence may reference it.
        fn complete(&mut self, obj: &PyObjectRef) {
            if let Some(entry) = self.map.get_mut(&obj.get_id()) {
                entry.incomplete = false;
            }
        }
    }

    fn write_object(
        buf: &mut Vec<u8>,
        obj: &PyObjectRef,
        refs: &mut Option<WriterRefTable>,
        version: i32,
        allow_code: bool,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        write_object_depth(
            buf,
            obj,
            refs,
            version,
            allow_code,
            vm,
            marshal::MAX_MARSHAL_STACK_DEPTH,
        )
    }

    fn write_object_depth(
        buf: &mut Vec<u8>,
        obj: &PyObjectRef,
        refs: &mut Option<WriterRefTable>,
        version: i32,
        allow_code: bool,
        vm: &VirtualMachine,
        depth: usize,
    ) -> PyResult<()> {
        use marshal::Write;
        if depth == 0 {
            return Err(vm.new_value_error("object too deeply nested to marshal"));
        }

        // Singletons: no FLAG_REF needed
        let is_singleton = vm.is_none(obj)
            || obj.class().is(PyBool::static_type())
            || obj.is(PyStopIteration::static_type())
            || obj.downcast_ref::<crate::builtins::PyEllipsis>().is_some();

        // FLAG_REF: check if already written, otherwise reserve slot
        if !is_singleton && let Some(rt) = refs.as_mut() {
            match rt.try_ref(buf, obj) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(()) => {
                    return Err(vm.new_value_error(format!(
                        "cannot marshal recursion {} objects",
                        obj.class().name()
                    )));
                }
            }
        }
        let type_pos = buf.len();
        let use_ref = refs.is_some() && !is_singleton;
        // A code or slice entry stays incomplete until its contents are
        // written: the reader rebuilds both from their fields, so a
        // back-reference issued while those fields are still being emitted
        // would name an object that does not exist yet.
        let requires_completion = obj.downcast_ref::<PyCode>().is_some()
            || obj.downcast_ref::<crate::builtins::PySlice>().is_some();
        if use_ref {
            refs.as_mut().unwrap().reserve(obj, requires_completion);
        }

        if vm.is_none(obj) {
            buf.write_u8(b'N');
        } else if obj.is(PyStopIteration::static_type()) {
            buf.write_u8(b'S');
        } else if obj.class().is(PyBool::static_type()) {
            let val = obj
                .downcast_ref::<PyInt>()
                .is_some_and(|i| !i.as_bigint().is_zero());
            buf.write_u8(if val { b'T' } else { b'F' });
        } else if obj.downcast_ref::<crate::builtins::PyEllipsis>().is_some() {
            buf.write_u8(b'.');
        } else if let Some(i) = obj.downcast_ref::<PyInt>() {
            // TYPE_INT for i32 range, TYPE_LONG for larger
            if let Ok(val) = i32::try_from(i.as_bigint()) {
                buf.write_u8(b'i');
                buf.write_u32(val as u32);
            } else {
                buf.write_u8(b'l');
                let (sign, raw) = i.as_bigint().to_bytes_le();
                let mut digits = Vec::new();
                let mut accum: u32 = 0;
                let mut bits = 0u32;
                for &byte in &raw {
                    accum |= (byte as u32) << bits;
                    bits += 8;
                    while bits >= 15 {
                        digits.push((accum & 0x7fff) as u16);
                        accum >>= 15;
                        bits -= 15;
                    }
                }
                if accum > 0 || digits.is_empty() {
                    digits.push(accum as u16);
                }
                while digits.len() > 1 && *digits.last().unwrap() == 0 {
                    digits.pop();
                }
                let n = digits.len() as i32;
                let n = if sign == malachite_bigint::Sign::Minus {
                    -n
                } else {
                    n
                };
                buf.write_u32(n as u32);
                for d in &digits {
                    buf.write_u16(*d);
                }
            }
        } else if let Some(f) = obj.downcast_ref::<PyFloat>() {
            buf.write_u8(b'g');
            buf.write_u64(f.to_f64().to_bits());
        } else if let Some(c) = obj.downcast_ref::<PyComplex>() {
            buf.write_u8(b'y');
            let cv = c.to_complex64();
            buf.write_u64(cv.re.to_bits());
            buf.write_u64(cv.im.to_bits());
        } else if let Some(s) = obj.downcast_ref::<PyStr>() {
            let bytes = s.as_wtf8().as_bytes();
            let interned = version >= 3;
            if bytes.len() < 256 && bytes.is_ascii() {
                buf.write_u8(if interned { b'Z' } else { b'z' });
                buf.write_u8(bytes.len() as u8);
            } else {
                buf.write_u8(if interned { b't' } else { b'u' });
                buf.write_u32(bytes.len() as u32);
            }
            buf.write_slice(bytes);
        } else if let Some(b) = obj.downcast_ref::<PyBytes>() {
            buf.write_u8(b's');
            let data = b.as_bytes();
            buf.write_u32(data.len() as u32);
            buf.write_slice(data);
        } else if let Some(b) = obj.downcast_ref::<PyByteArray>() {
            buf.write_u8(b's');
            let data = b.borrow_buf();
            buf.write_u32(data.len() as u32);
            buf.write_slice(&data);
        } else if let Some(t) = obj.downcast_ref::<PyTuple>() {
            buf.write_u8(b'(');
            buf.write_u32(t.len() as u32);
            for elem in t.as_slice() {
                write_object_depth(buf, elem, refs, version, allow_code, vm, depth - 1)?;
            }
        } else if let Some(l) = obj.downcast_ref::<PyList>() {
            buf.write_u8(b'[');
            let items = l.borrow_vec();
            buf.write_u32(items.len() as u32);
            for elem in items.iter() {
                write_object_depth(buf, elem, refs, version, allow_code, vm, depth - 1)?;
            }
        } else if let Some(d) = obj.downcast_ref::<PyDict>() {
            buf.write_u8(b'{');
            for (k, v) in d {
                write_object_depth(buf, &k, refs, version, allow_code, vm, depth - 1)?;
                write_object_depth(buf, &v, refs, version, allow_code, vm, depth - 1)?;
            }
            buf.write_u8(b'0'); // TYPE_NULL terminator
        } else if let Some(s) = obj.downcast_ref::<PySet>() {
            buf.write_u8(b'<');
            let elems = s.elements();
            buf.write_u32(elems.len() as u32);
            for elem in &elems {
                write_object_depth(buf, elem, refs, version, allow_code, vm, depth - 1)?;
            }
        } else if let Some(s) = obj.downcast_ref::<PyFrozenSet>() {
            buf.write_u8(b'>');
            let elems = s.elements();
            buf.write_u32(elems.len() as u32);
            for elem in &elems {
                write_object_depth(buf, elem, refs, version, allow_code, vm, depth - 1)?;
            }
        } else if let Some(co) = obj.downcast_ref::<PyCode>() {
            if !allow_code {
                return Err(vm.new_value_error("marshalling code objects is disallowed"));
            }
            buf.write_u8(b'c');
            // `Literal` holds the exact object a constant was built from, so
            // route `co_consts` back through the object writer: it reaches the
            // values `BorrowedConstant` cannot describe and shares the one
            // reference table the reader indexes against.
            marshal::serialize_code_with(buf, &co.code, |buf, constant| {
                let constant = PyObjectRef::from(constant.clone());
                write_object_depth(buf, &constant, refs, version, allow_code, vm, depth - 1)
            })?;
        } else if let Some(sl) = obj.downcast_ref::<crate::builtins::PySlice>() {
            if version < 5 {
                return Err(vm.new_value_error("unmarshallable object"));
            }
            buf.write_u8(b':');
            let none: PyObjectRef = vm.ctx.none();
            write_object_depth(
                buf,
                sl.start.as_ref().unwrap_or(&none),
                refs,
                version,
                allow_code,
                vm,
                depth - 1,
            )?;
            write_object_depth(buf, &sl.stop, refs, version, allow_code, vm, depth - 1)?;
            write_object_depth(
                buf,
                sl.step.as_ref().unwrap_or(&none),
                refs,
                version,
                allow_code,
                vm,
                depth - 1,
            )?;
        } else if let Ok(bytes_like) = ArgBytesLike::try_from_object(vm, obj.clone()) {
            buf.write_u8(b's');
            let data = bytes_like.borrow_buf();
            buf.write_u32(data.len() as u32);
            buf.write_slice(&data);
        } else {
            return Err(vm.new_value_error("unmarshallable object"));
        }

        if use_ref {
            buf[type_pos] |= marshal::FLAG_REF;
            if requires_completion {
                refs.as_mut().unwrap().complete(obj);
            }
        }
        Ok(())
    }

    #[derive(FromArgs)]
    struct DumpArgs {
        value: PyObjectRef,
        f: PyObjectRef,
        #[pyarg(any, optional)]
        _version: OptionalArg<i32>,
        #[pyarg(named, default = true)]
        allow_code: bool,
    }

    #[pyfunction]
    fn dump(args: DumpArgs, vm: &VirtualMachine) -> PyResult<()> {
        let dumped = dumps(
            DumpsArgs {
                value: args.value,
                _version: args._version,
                allow_code: args.allow_code,
            },
            vm,
        )?;
        vm.call_method(&args.f, "write", (dumped,))?;
        Ok(())
    }

    #[derive(Copy, Clone)]
    struct PyMarshalBag<'a> {
        vm: &'a VirtualMachine,
        pending_error: &'a RefCell<Option<PyBaseExceptionRef>>,
        allow_code: bool,
    }

    impl<'a> PyMarshalBag<'a> {
        fn new(
            vm: &'a VirtualMachine,
            pending_error: &'a RefCell<Option<PyBaseExceptionRef>>,
            allow_code: bool,
        ) -> Self {
            Self {
                vm,
                pending_error,
                allow_code,
            }
        }

        /// Room for a container the decoder publishes before it reads what
        /// goes in it. The length is the input's to choose, so the room is
        /// asked for rather than assumed: a length no allocator can serve is
        /// a MemoryError, not an aborted process.
        fn placeholder_elements(
            &self,
            len: usize,
        ) -> Result<Vec<PyObjectRef>, marshal::MarshalError> {
            let mut elements = Vec::new();
            elements
                .try_reserve_exact(len)
                .map_err(|_| self.remember_python_error(self.vm.no_memory_error()))?;
            elements.resize(len, self.vm.ctx.none());
            Ok(elements)
        }

        fn remember_python_error(&self, error: PyBaseExceptionRef) -> marshal::MarshalError {
            let mut pending = self.pending_error.borrow_mut();
            if pending.is_none() {
                *pending = Some(error);
            }
            marshal::MarshalError::BadType
        }
    }

    impl<'a> marshal::MarshalBag for PyMarshalBag<'a> {
        type Value = PyObjectRef;
        type ConstantBag = PyVmBag<'a>;

        fn make_bool(&self, value: bool) -> Self::Value {
            self.vm.ctx.new_bool(value).into()
        }
        fn make_none(&self) -> Self::Value {
            self.vm.ctx.none()
        }
        fn make_ellipsis(&self) -> Self::Value {
            self.vm.ctx.ellipsis.clone().into()
        }
        fn make_float(&self, value: f64) -> Self::Value {
            self.vm.ctx.new_float(value).into()
        }
        fn make_complex(&self, value: num_complex::Complex64) -> Self::Value {
            self.vm.ctx.new_complex(value).into()
        }
        fn make_str(&self, value: &Wtf8) -> Self::Value {
            self.vm.ctx.new_str(value).into()
        }
        fn make_interned_str(&self, value: &Wtf8) -> Self::Value {
            self.vm.ctx.intern_str(value).to_owned().into()
        }
        fn make_bytes(&self, value: &[u8]) -> Self::Value {
            self.vm.ctx.new_bytes(value.to_vec()).into()
        }
        fn make_int(&self, value: BigInt) -> Self::Value {
            self.vm.ctx.new_int(value).into()
        }
        fn make_tuple(&self, elements: impl Iterator<Item = Self::Value>) -> Self::Value {
            self.vm.ctx.new_tuple(elements.collect()).into()
        }
        fn make_tuple_placeholder(
            &self,
            len: usize,
        ) -> Result<Option<Self::Value>, marshal::MarshalError> {
            let elements = self.placeholder_elements(len)?;
            Ok(Some(PyTuple::new_ref(elements, &self.vm.ctx).into()))
        }
        fn set_tuple_item(
            &self,
            tuple: &Self::Value,
            index: usize,
            value: Self::Value,
        ) -> Result<(), marshal::MarshalError> {
            let tuple = tuple
                .downcast_ref::<PyTuple>()
                .ok_or(marshal::MarshalError::BadType)?;
            // SAFETY: compiler-core calls this only on a fresh placeholder,
            // once per index, before returning it to Python code.
            unsafe { tuple.set_marshal_item(index, value) };
            Ok(())
        }
        fn make_code(&self, code: CodeObject) -> Result<Self::Value, marshal::MarshalError> {
            if !self.allow_code {
                return Err(self.remember_python_error(
                    self.vm
                        .new_value_error("unmarshalling code objects is disallowed"),
                ));
            }
            Ok(crate::builtins::PyCode::new_ref_with_bag(self.vm, code).into())
        }
        fn make_stop_iter(&self) -> Result<Self::Value, marshal::MarshalError> {
            Ok(self.vm.ctx.exceptions.stop_iteration.to_owned().into())
        }
        fn make_list(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            Ok(self.vm.ctx.new_list(it.collect()).into())
        }
        fn make_list_placeholder(
            &self,
            len: usize,
        ) -> Result<Option<Self::Value>, marshal::MarshalError> {
            let elements = self.placeholder_elements(len)?;
            Ok(Some(self.vm.ctx.new_list(elements).into()))
        }
        fn set_list_item(
            &self,
            list: &Self::Value,
            index: usize,
            value: Self::Value,
        ) -> Result<(), marshal::MarshalError> {
            let list = list
                .downcast_ref::<PyList>()
                .ok_or(marshal::MarshalError::BadType)?;
            list.borrow_vec_mut()[index] = value;
            Ok(())
        }
        fn make_set(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let set = PySet::default().into_ref(&self.vm.ctx);
            for elem in it {
                set.add(elem, self.vm)
                    .map_err(|error| self.remember_python_error(error))?;
            }
            Ok(set.into())
        }
        fn make_set_placeholder(&self) -> Option<Self::Value> {
            Some(PySet::default().into_ref(&self.vm.ctx).into())
        }
        fn insert_set_item(
            &self,
            set: &Self::Value,
            value: Self::Value,
        ) -> Result<(), marshal::MarshalError> {
            let set = set
                .downcast_ref::<PySet>()
                .ok_or(marshal::MarshalError::BadType)?;
            set.add(value, self.vm)
                .map_err(|error| self.remember_python_error(error))
        }
        fn make_frozenset(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            PyFrozenSet::from_iter(self.vm, it)
                .map(|set| set.to_pyobject(self.vm))
                .map_err(|error| self.remember_python_error(error))
        }
        fn make_dict(
            &self,
            it: impl Iterator<Item = (Self::Value, Self::Value)>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let dict = self.vm.ctx.new_dict();
            for (k, v) in it {
                dict.set_item(&*k, v, self.vm)
                    .map_err(|error| self.remember_python_error(error))?;
            }
            Ok(dict.into())
        }
        fn make_dict_placeholder(&self) -> Option<Self::Value> {
            Some(self.vm.ctx.new_dict().into())
        }
        fn insert_dict_item(
            &self,
            dict: &Self::Value,
            key: Self::Value,
            value: Self::Value,
        ) -> Result<(), marshal::MarshalError> {
            let dict = dict
                .downcast_ref::<PyDict>()
                .ok_or(marshal::MarshalError::BadType)?;
            dict.set_item(&*key, value, self.vm)
                .map_err(|error| self.remember_python_error(error))
        }
        fn make_slice(
            &self,
            start: Self::Value,
            stop: Self::Value,
            step: Self::Value,
        ) -> Result<Self::Value, marshal::MarshalError> {
            use crate::builtins::PySlice;
            let vm = self.vm;
            Ok(PySlice {
                start: if vm.is_none(&start) {
                    None
                } else {
                    Some(start)
                },
                stop,
                step: if vm.is_none(&step) { None } else { Some(step) },
            }
            .into_ref(&vm.ctx)
            .into())
        }
        fn constant_bag(self) -> Self::ConstantBag {
            PyVmBag(self.vm)
        }
        /// `Literal` wraps any object, so a decoded `co_consts` entry is
        /// already its own compiler-side constant — no placeholder is needed
        /// and `make_code_with_constants` keeps the default.
        fn constant_ref_from_value(&self, value: &Self::Value) -> Option<Literal> {
            Some(Literal::from(value.clone()))
        }
        fn bytes_from_value(&self, value: &Self::Value) -> Option<Vec<u8>> {
            value
                .downcast_ref::<PyBytes>()
                .map(|bytes| bytes.as_bytes().to_vec())
        }
        fn str_from_value(&self, value: &Self::Value) -> Option<String> {
            value
                .downcast_ref::<PyStr>()
                .map(|str| str.to_string_lossy().into_owned())
        }
        fn tuple_elements_from_value(&self, value: &Self::Value) -> Option<Vec<Self::Value>> {
            value
                .downcast_ref::<PyTuple>()
                .map(|tuple| tuple.as_slice().to_vec())
        }
    }

    fn deserialize_value(
        rdr: &mut impl marshal::Read,
        allow_code: bool,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let pending_error = RefCell::new(None);
        match marshal::deserialize_value(rdr, PyMarshalBag::new(vm, &pending_error, allow_code)) {
            Ok(value) => Ok(value),
            Err(error) => Err(pending_error.into_inner().unwrap_or_else(|| match error {
                marshal::MarshalError::Eof => vm.new_eof_error("marshal data too short"),
                error @ marshal::MarshalError::NullObject => vm.new_type_error(error.to_string()),
                error @ (marshal::MarshalError::BadSize(_)
                | marshal::MarshalError::UnknownType
                | marshal::MarshalError::InvalidRef) => {
                    vm.new_value_error(format!("bad marshal data ({error})"))
                }
                _ => vm.new_value_error("bad marshal data"),
            })),
        }
    }

    #[derive(FromArgs)]
    struct LoadsArgs {
        #[pyarg(any)]
        // marshal_loads_impl takes `bytes: Py_buffer`, a y* argument.
        data: ArgBytesLike,
        #[pyarg(named, default = true)]
        allow_code: bool,
    }

    #[pyfunction]
    fn loads(args: LoadsArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let LoadsArgs { data, allow_code } = args;
        let buf = data.borrow_buf();

        deserialize_value(&mut &buf[..], allow_code, vm)
    }

    #[derive(FromArgs)]
    struct LoadArgs {
        f: PyObjectRef,
        #[pyarg(named, default = true)]
        allow_code: bool,
    }

    #[pyfunction]
    fn load(args: LoadArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // Read from file object into a buffer, one object at a time.
        // We read all available data, deserialize one object, then seek
        // back to just after the consumed bytes.
        let tell_before = vm
            .call_method(&args.f, "tell", ())?
            .try_into_value::<i64>(vm)?;
        let read_res = vm.call_method(&args.f, "read", ())?;
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;

        // The borrow ends here: seek() below is the caller's, and reaching the
        // same buffer from it would deadlock on a borrow still held.
        let (result, consumed) = {
            let buf = bytes.borrow_buf();
            let mut rdr: &[u8] = &buf;
            let len_before = rdr.len();
            let result = deserialize_value(&mut rdr, args.allow_code, vm)?;
            (result, len_before - rdr.len())
        };

        // Seek file to just after the consumed bytes
        let new_pos = tell_before + consumed as i64;
        vm.call_method(&args.f, "seek", (new_pos,))?;

        Ok(result)
    }

    /// Reject subclasses of marshallable types (int, float, complex, tuple, etc.).
    fn check_exact_type(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let cls = obj.class();
        // bool is a subclass of int but is marshallable
        if cls.is(PyBool::static_type()) {
            return Ok(());
        }
        for base in [
            PyInt::static_type(),
            PyFloat::static_type(),
            PyComplex::static_type(),
            PyTuple::static_type(),
            PyList::static_type(),
            PyDict::static_type(),
            PySet::static_type(),
            PyFrozenSet::static_type(),
        ] {
            if cls.fast_issubclass(base) && !cls.is(base) {
                return Err(vm.new_value_error("unmarshallable object"));
            }
        }
        Ok(())
    }
}
