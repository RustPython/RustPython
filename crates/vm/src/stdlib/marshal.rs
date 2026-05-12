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
            PyBool, PyByteArray, PyBytes, PyCode, PyComplex, PyDict, PyEllipsis, PyFloat,
            PyFrozenSet, PyInt, PyList, PyNone, PySet, PyStopIteration, PyStr, PyTuple,
        },
        convert::ToPyObject,
        function::{ArgBytesLike, OptionalArg},
        object::{AsObject, PyPayload},
        protocol::PyBuffer,
    };
    use malachite_bigint::BigInt;
    use num_traits::Zero;
    use rustpython_compiler_core::marshal;

    #[pyattr(name = "version")]
    use marshal::FORMAT_VERSION;

    pub struct DumpError;

    impl marshal::Dumpable for PyObjectRef {
        type Error = DumpError;
        type Constant = Literal;

        fn with_dump<R>(
            &self,
            f: impl FnOnce(marshal::DumpableValue<'_, Self>) -> R,
        ) -> Result<R, Self::Error> {
            use marshal::DumpableValue::*;
            if self.is(PyStopIteration::static_type()) {
                return Ok(f(StopIter));
            }
            let ret = match_class!(match self {
                PyNone => f(None),
                PyEllipsis => f(Ellipsis),
                ref pyint @ PyInt => {
                    if self.class().is(PyBool::static_type()) {
                        f(Boolean(!pyint.as_bigint().is_zero()))
                    } else {
                        f(Integer(pyint.as_bigint()))
                    }
                }
                ref pyfloat @ PyFloat => {
                    f(Float(pyfloat.to_f64()))
                }
                ref pycomplex @ PyComplex => {
                    f(Complex(pycomplex.to_complex64()))
                }
                ref pystr @ PyStr => {
                    f(Str(pystr.as_wtf8()))
                }
                ref pylist @ PyList => {
                    f(List(&pylist.borrow_vec()))
                }
                ref pyset @ PySet => {
                    let elements = pyset.elements();
                    f(Set(&elements))
                }
                ref pyfrozen @ PyFrozenSet => {
                    let elements = pyfrozen.elements();
                    f(Frozenset(&elements))
                }
                ref pytuple @ PyTuple => {
                    f(Tuple(pytuple.as_slice()))
                }
                ref pydict @ PyDict => {
                    let entries = pydict.into_iter().collect::<Vec<_>>();
                    f(Dict(&entries))
                }
                ref bytes @ PyBytes => {
                    f(Bytes(bytes.as_bytes()))
                }
                ref bytes @ PyByteArray => {
                    f(Bytes(&bytes.borrow_buf()))
                }
                ref co @ PyCode => {
                    f(Code(co))
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
        if !allow_code {
            check_no_code(&value, vm)?;
        }
        check_exact_type(&value, vm)?;
        let mut buf = Vec::new();
        let mut refs = if version >= 3 {
            Some(WriterRefTable::new())
        } else {
            None
        };
        write_object(&mut buf, &value, &mut refs, version, vm)?;
        Ok(PyBytes::from(buf))
    }

    struct WriterRefTable {
        map: std::collections::HashMap<usize, u32>,
        next_idx: u32,
    }

    impl WriterRefTable {
        fn new() -> Self {
            Self {
                map: std::collections::HashMap::new(),
                next_idx: 0,
            }
        }
        fn try_ref(&mut self, buf: &mut Vec<u8>, obj: &PyObjectRef) -> bool {
            use marshal::Write;
            let id = obj.get_id();
            if let Some(&idx) = self.map.get(&id) {
                buf.write_u8(b'r');
                buf.write_u32(idx);
                true
            } else {
                false
            }
        }
        fn reserve(&mut self, obj: &PyObjectRef) -> u32 {
            let idx = self.next_idx;
            self.map.insert(obj.get_id(), idx);
            self.next_idx += 1;
            idx
        }
    }

    fn write_object(
        buf: &mut Vec<u8>,
        obj: &PyObjectRef,
        refs: &mut Option<WriterRefTable>,
        version: i32,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        write_object_depth(
            buf,
            obj,
            refs,
            version,
            vm,
            marshal::MAX_MARSHAL_STACK_DEPTH,
        )
    }

    fn write_object_depth(
        buf: &mut Vec<u8>,
        obj: &PyObjectRef,
        refs: &mut Option<WriterRefTable>,
        version: i32,
        vm: &VirtualMachine,
        depth: usize,
    ) -> PyResult<()> {
        use marshal::Write;
        if depth == 0 {
            return Err(vm.new_value_error("object too deeply nested to marshal".to_string()));
        }

        // Singletons: no FLAG_REF needed
        let is_singleton = vm.is_none(obj)
            || obj.class().is(PyBool::static_type())
            || obj.is(PyStopIteration::static_type())
            || obj.downcast_ref::<crate::builtins::PyEllipsis>().is_some();

        // FLAG_REF: check if already written, otherwise reserve slot
        if !is_singleton
            && let Some(rt) = refs.as_mut()
            && rt.try_ref(buf, obj)
        {
            return Ok(());
        }
        let type_pos = buf.len();
        let use_ref = refs.is_some() && !is_singleton;
        if use_ref {
            refs.as_mut().unwrap().reserve(obj);
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
                write_object_depth(buf, elem, refs, version, vm, depth - 1)?;
            }
        } else if let Some(l) = obj.downcast_ref::<PyList>() {
            buf.write_u8(b'[');
            let items = l.borrow_vec();
            buf.write_u32(items.len() as u32);
            for elem in items.iter() {
                write_object_depth(buf, elem, refs, version, vm, depth - 1)?;
            }
        } else if let Some(d) = obj.downcast_ref::<PyDict>() {
            buf.write_u8(b'{');
            for (k, v) in d {
                write_object_depth(buf, &k, refs, version, vm, depth - 1)?;
                write_object_depth(buf, &v, refs, version, vm, depth - 1)?;
            }
            buf.write_u8(b'0'); // TYPE_NULL terminator
        } else if let Some(s) = obj.downcast_ref::<PySet>() {
            buf.write_u8(b'<');
            let elems = s.elements();
            buf.write_u32(elems.len() as u32);
            for elem in &elems {
                write_object_depth(buf, elem, refs, version, vm, depth - 1)?;
            }
        } else if let Some(s) = obj.downcast_ref::<PyFrozenSet>() {
            buf.write_u8(b'>');
            let elems = s.elements();
            buf.write_u32(elems.len() as u32);
            for elem in &elems {
                write_object_depth(buf, elem, refs, version, vm, depth - 1)?;
            }
        } else if let Some(co) = obj.downcast_ref::<PyCode>() {
            buf.write_u8(b'c');
            marshal::serialize_code(buf, &co.code);
        } else if let Some(sl) = obj.downcast_ref::<crate::builtins::PySlice>() {
            if version < 5 {
                return Err(vm.new_value_error("unmarshallable object".to_string()));
            }
            buf.write_u8(b':');
            let none: PyObjectRef = vm.ctx.none();
            write_object_depth(
                buf,
                sl.start.as_ref().unwrap_or(&none),
                refs,
                version,
                vm,
                depth - 1,
            )?;
            write_object_depth(buf, &sl.stop, refs, version, vm, depth - 1)?;
            write_object_depth(
                buf,
                sl.step.as_ref().unwrap_or(&none),
                refs,
                version,
                vm,
                depth - 1,
            )?;
        } else if let Ok(bytes_like) = ArgBytesLike::try_from_object(vm, obj.clone()) {
            buf.write_u8(b's');
            let data = bytes_like.borrow_buf();
            buf.write_u32(data.len() as u32);
            buf.write_slice(&data);
        } else {
            return Err(vm.new_value_error("unmarshallable object".to_string()));
        }

        if use_ref {
            buf[type_pos] |= marshal::FLAG_REF;
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
    struct PyMarshalBag<'a>(&'a VirtualMachine);

    impl<'a> marshal::MarshalBag for PyMarshalBag<'a> {
        type Value = PyObjectRef;
        type ConstantBag = PyVmBag<'a>;

        fn make_bool(&self, value: bool) -> Self::Value {
            self.0.ctx.new_bool(value).into()
        }
        fn make_none(&self) -> Self::Value {
            self.0.ctx.none()
        }
        fn make_ellipsis(&self) -> Self::Value {
            self.0.ctx.ellipsis.clone().into()
        }
        fn make_float(&self, value: f64) -> Self::Value {
            self.0.ctx.new_float(value).into()
        }
        fn make_complex(&self, value: num_complex::Complex64) -> Self::Value {
            self.0.ctx.new_complex(value).into()
        }
        fn make_str(&self, value: &Wtf8) -> Self::Value {
            self.0.ctx.new_str(value).into()
        }
        fn make_bytes(&self, value: &[u8]) -> Self::Value {
            self.0.ctx.new_bytes(value.to_vec()).into()
        }
        fn make_int(&self, value: BigInt) -> Self::Value {
            self.0.ctx.new_int(value).into()
        }
        fn make_tuple(&self, elements: impl Iterator<Item = Self::Value>) -> Self::Value {
            self.0.ctx.new_tuple(elements.collect()).into()
        }
        fn make_code(&self, code: CodeObject) -> Self::Value {
            crate::builtins::PyCode::new_ref_with_bag(self.0, code).into()
        }
        fn make_stop_iter(&self) -> Result<Self::Value, marshal::MarshalError> {
            Ok(self.0.ctx.exceptions.stop_iteration.to_owned().into())
        }
        fn make_list(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            Ok(self.0.ctx.new_list(it.collect()).into())
        }
        fn make_set(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let set = PySet::default().into_ref(&self.0.ctx);
            for elem in it {
                set.add(elem, self.0).unwrap()
            }
            Ok(set.into())
        }
        fn make_frozenset(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            Ok(PyFrozenSet::from_iter(self.0, it)
                .unwrap()
                .to_pyobject(self.0))
        }
        fn make_dict(
            &self,
            it: impl Iterator<Item = (Self::Value, Self::Value)>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let dict = self.0.ctx.new_dict();
            for (k, v) in it {
                dict.set_item(&*k, v, self.0).unwrap()
            }
            Ok(dict.into())
        }
        fn make_slice(
            &self,
            start: Self::Value,
            stop: Self::Value,
            step: Self::Value,
        ) -> Result<Self::Value, marshal::MarshalError> {
            use crate::builtins::PySlice;
            let vm = self.0;
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
            PyVmBag(self.0)
        }
    }

    #[derive(FromArgs)]
    struct LoadsArgs {
        #[pyarg(any)]
        data: PyBuffer,
        #[pyarg(named, default = true)]
        allow_code: bool,
    }

    #[pyfunction]
    fn loads(args: LoadsArgs, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let LoadsArgs {
            data: pybuffer,
            allow_code,
        } = args;
        let buf = pybuffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("Buffer provided to marshal.loads() is not contiguous")
        })?;

        let result =
            marshal::deserialize_value(&mut &buf[..], PyMarshalBag(vm)).map_err(|e| match e {
                marshal::MarshalError::Eof => vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "marshal data too short".into(),
                ),
                _ => vm.new_value_error("bad marshal data"),
            })?;
        if !allow_code {
            check_no_code(&result, vm)?;
        }
        Ok(result)
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
        let buf = bytes.borrow_buf();

        let mut rdr: &[u8] = &buf;
        let len_before = rdr.len();
        let result =
            marshal::deserialize_value(&mut rdr, PyMarshalBag(vm)).map_err(|e| match e {
                marshal::MarshalError::Eof => vm.new_exception_msg(
                    vm.ctx.exceptions.eof_error.to_owned(),
                    "marshal data too short".into(),
                ),
                _ => vm.new_value_error("bad marshal data"),
            })?;
        let consumed = len_before - rdr.len();

        // Seek file to just after the consumed bytes
        let new_pos = tell_before + consumed as i64;
        vm.call_method(&args.f, "seek", (new_pos,))?;

        if !args.allow_code {
            check_no_code(&result, vm)?;
        }
        Ok(result)
    }

    /// Reject subclasses of marshallable types (int, float, complex, tuple, etc.).
    /// Recursively check that no code objects are present.
    fn check_no_code(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if obj.downcast_ref::<PyCode>().is_some() {
            return Err(vm.new_value_error("unmarshalling code objects is disallowed".to_string()));
        }
        if let Some(tup) = obj.downcast_ref::<PyTuple>() {
            for elem in tup.as_slice() {
                check_no_code(elem, vm)?;
            }
        } else if let Some(list) = obj.downcast_ref::<PyList>() {
            for elem in list.borrow_vec().iter() {
                check_no_code(elem, vm)?;
            }
        } else if let Some(set) = obj.downcast_ref::<PySet>() {
            for elem in set.elements() {
                check_no_code(&elem, vm)?;
            }
        } else if let Some(fset) = obj.downcast_ref::<PyFrozenSet>() {
            for elem in fset.elements() {
                check_no_code(&elem, vm)?;
            }
        } else if let Some(dict) = obj.downcast_ref::<PyDict>() {
            for (k, v) in dict {
                check_no_code(&k, vm)?;
                check_no_code(&v, vm)?;
            }
        }
        Ok(())
    }

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
                return Err(vm.new_value_error("unmarshallable object".to_string()));
            }
        }
        Ok(())
    }
}
