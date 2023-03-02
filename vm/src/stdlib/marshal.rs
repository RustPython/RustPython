pub(crate) use decl::make_module;

#[pymodule(name = "marshal")]
mod decl {
    use crate::builtins::code::{CodeObject, Literal, PyObjBag};
    use crate::class::StaticType;
    use crate::{
        builtins::{
            PyBool, PyByteArray, PyBytes, PyCode, PyComplex, PyDict, PyEllipsis, PyFloat,
            PyFrozenSet, PyInt, PyList, PyNone, PySet, PyStopIteration, PyStr, PyTuple,
        },
        convert::ToPyObject,
        function::{ArgBytesLike, OptionalArg},
        object::AsObject,
        protocol::PyBuffer,
        PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    };
    use num_bigint::BigInt;
    use num_complex::Complex64;
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
                    f(Str(pystr.as_str()))
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

    #[pyfunction]
    fn dumps(
        value: PyObjectRef,
        _version: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<PyBytes> {
        use marshal::Dumpable;
        let mut buf = Vec::new();
        value
            .with_dump(|val| marshal::serialize_value(&mut buf, val))
            .unwrap_or_else(Err)
            .map_err(|DumpError| {
                vm.new_not_implemented_error(
                    "TODO: not implemented yet or marshal unsupported type".to_owned(),
                )
            })?;
        Ok(PyBytes::from(buf))
    }

    #[pyfunction]
    fn dump(
        value: PyObjectRef,
        f: PyObjectRef,
        version: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let dumped = dumps(value, version, vm)?;
        vm.call_method(&f, "write", (dumped,))?;
        Ok(())
    }

    #[derive(Copy, Clone)]
    struct PyMarshalBag<'a>(&'a VirtualMachine);

    impl<'a> marshal::MarshalBag for PyMarshalBag<'a> {
        type Value = PyObjectRef;
        fn make_bool(&self, value: bool) -> Self::Value {
            self.0.ctx.new_bool(value).into()
        }
        fn make_none(&self) -> Self::Value {
            self.0.ctx.none()
        }
        fn make_ellipsis(&self) -> Self::Value {
            self.0.ctx.ellipsis()
        }
        fn make_float(&self, value: f64) -> Self::Value {
            self.0.ctx.new_float(value).into()
        }
        fn make_complex(&self, value: Complex64) -> Self::Value {
            self.0.ctx.new_complex(value).into()
        }
        fn make_str(&self, value: &str) -> Self::Value {
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
            self.0.ctx.new_code(code).into()
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
            let vm = self.0;
            let set = PySet::new_ref(&vm.ctx);
            for elem in it {
                set.add(elem, vm).unwrap()
            }
            Ok(set.into())
        }
        fn make_frozenset(
            &self,
            it: impl Iterator<Item = Self::Value>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let vm = self.0;
            Ok(PyFrozenSet::from_iter(vm, it).unwrap().to_pyobject(vm))
        }
        fn make_dict(
            &self,
            it: impl Iterator<Item = (Self::Value, Self::Value)>,
        ) -> Result<Self::Value, marshal::MarshalError> {
            let vm = self.0;
            let dict = vm.ctx.new_dict();
            for (k, v) in it {
                dict.set_item(&*k, v, vm).unwrap()
            }
            Ok(dict.into())
        }
        type ConstantBag = PyObjBag<'a>;
        fn constant_bag(self) -> Self::ConstantBag {
            PyObjBag(&self.0.ctx)
        }
    }

    #[pyfunction]
    fn loads(pybuffer: PyBuffer, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let buf = pybuffer.as_contiguous().ok_or_else(|| {
            vm.new_buffer_error("Buffer provided to marshal.loads() is not contiguous".to_owned())
        })?;
        marshal::deserialize_value(&mut &buf[..], PyMarshalBag(vm)).map_err(|e| match e {
            marshal::MarshalError::Eof => vm.new_exception_msg(
                vm.ctx.exceptions.eof_error.to_owned(),
                "marshal data too short".to_owned(),
            ),
            marshal::MarshalError::InvalidBytecode => {
                vm.new_value_error("Couldn't deserialize python bytecode".to_owned())
            }
            marshal::MarshalError::InvalidUtf8 => {
                vm.new_value_error("invalid utf8 in marshalled string".to_owned())
            }
            marshal::MarshalError::BadType => {
                vm.new_value_error("bad marshal data (unknown type code)".to_owned())
            }
        })
    }

    #[pyfunction]
    fn load(f: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let read_res = vm.call_method(&f, "read", ())?;
        let bytes = ArgBytesLike::try_from_object(vm, read_res)?;
        loads(PyBuffer::from(bytes), vm)
    }
}
