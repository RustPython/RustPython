//! Python struct module.
//!
//! Docs: <https://docs.python.org/3/library/struct.html>
//!
//! Use this rust module to do byte packing:
//! <https://docs.rs/byteorder/1.2.6/byteorder/>

pub(crate) use _struct::module_def;

#[pymodule]
pub(crate) mod _struct {
    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, TryFromObject, VirtualMachine,
        buffer::{FormatSpec, new_struct_error, struct_error_type},
        builtins::{PyBytes, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        common::lock::{PyMappedRwLockReadGuard, PyRwLock, PyRwLockReadGuard},
        function::{ArgBytesLike, ArgMemoryBuffer, FuncArgs, PosArgs},
        match_class,
        protocol::PyIterReturn,
        types::{Constructor, Initializer, IterNext, Iterable, Representable, SelfIter},
    };
    use crossbeam_utils::atomic::AtomicCell;
    use rustpython_common::wtf8::{Wtf8Buf, wtf8_concat};

    #[derive(Traverse)]
    struct IntoStructFormatBytes(PyStrRef);

    impl TryFromObject for IntoStructFormatBytes {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            // CPython turns str to bytes (via str.encode('ascii')) but we keep str.
            // The error reporting for non-ASCII input still matches CPython:
            // - str input with non-ASCII char: UnicodeEncodeError, the same exception
            //   str.encode('ascii') would produce.
            // - bytes input with non-ASCII byte: struct.error("bad char in struct format"),
            //   matching CPython where bytes are passed through to the format parser.
            let fmt = match_class!(match obj {
                s @ PyStr => {
                    if !s.isascii() {
                        let start = s
                            .as_wtf8()
                            .code_points()
                            .position(|cp| !cp.to_char().is_some_and(|c| c.is_ascii()))
                            .unwrap_or(0);
                        return Err(vm.new_unicode_encode_error_real(
                            vm.ctx.new_str("ascii"),
                            s,
                            start,
                            start + 1,
                            vm.ctx.new_str("ordinal not in range(128)"),
                        ));
                    }
                    s
                }
                b @ PyBytes => {
                    let ascii_str = ascii::AsciiStr::from_ascii(&b)
                        .map_err(|_| new_struct_error(vm, "bad char in struct format"))?;
                    vm.ctx.new_str(ascii_str)
                }
                other =>
                    return Err(vm.new_type_error(format!(
                        "Struct() argument 1 must be a str or bytes object, not {}",
                        other.class().name()
                    ))),
            });
            Ok(Self(fmt))
        }
    }

    impl IntoStructFormatBytes {
        fn format_spec(&self, vm: &VirtualMachine) -> PyResult<FormatSpec> {
            FormatSpec::parse(self.0.as_bytes(), vm)
        }
    }

    fn get_buffer_offset(
        buffer_len: usize,
        offset: isize,
        needed: usize,
        is_pack: bool,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let offset_from_start = if offset < 0 {
            if (-offset) as usize > buffer_len {
                return Err(new_struct_error(
                    vm,
                    format!("offset {offset} out of range for {buffer_len}-byte buffer"),
                ));
            }
            buffer_len - (-offset as usize)
        } else {
            let offset = offset as usize;
            let (op, op_action) = if is_pack {
                ("pack_into", "packing")
            } else {
                ("unpack_from", "unpacking")
            };
            if offset + needed > buffer_len {
                let msg = format!(
                    "{op} requires a buffer of at least {required} bytes for {op_action} {needed} \
                    bytes at offset {offset} (actual buffer size is {buffer_len})",
                    op = op,
                    op_action = op_action,
                    required = needed + offset,
                    needed = needed,
                    offset = offset,
                    buffer_len = buffer_len
                );
                return Err(new_struct_error(vm, msg));
            }
            offset
        };

        if (buffer_len - offset_from_start) < needed {
            Err(new_struct_error(
                vm,
                if is_pack {
                    format!("no space to pack {needed} bytes at offset {offset}")
                } else {
                    format!("not enough data to unpack {needed} bytes at offset {offset}")
                },
            ))
        } else {
            Ok(offset_from_start)
        }
    }

    #[pyfunction]
    fn pack(fmt: IntoStructFormatBytes, args: PosArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        fmt.format_spec(vm)?.pack(args.into_vec(), vm)
    }

    #[pyfunction]
    fn pack_into(
        fmt: IntoStructFormatBytes,
        buffer: ArgMemoryBuffer,
        offset: isize,
        args: PosArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let format_spec = fmt.format_spec(vm)?;
        let offset = get_buffer_offset(buffer.len(), offset, format_spec.size, true, vm)?;
        buffer.with_ref(|data| format_spec.pack_into(&mut data[offset..], args.into_vec(), vm))
    }

    #[pyfunction]
    fn unpack(
        fmt: IntoStructFormatBytes,
        buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = fmt.format_spec(vm)?;
        buffer.with_ref(|buf| format_spec.unpack(buf, vm))
    }

    #[derive(FromArgs)]
    struct UpdateFromArgs {
        buffer: ArgBytesLike,
        #[pyarg(any, default = 0)]
        offset: isize,
    }

    #[pyfunction]
    fn unpack_from(
        fmt: IntoStructFormatBytes,
        args: UpdateFromArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = fmt.format_spec(vm)?;
        let offset =
            get_buffer_offset(args.buffer.len(), args.offset, format_spec.size, false, vm)?;
        args.buffer
            .with_ref(|buf| format_spec.unpack(&buf[offset..][..format_spec.size], vm))
    }

    #[pyattr]
    #[pyclass(name = "unpack_iterator", traverse)]
    #[derive(Debug, PyPayload)]
    struct UnpackIterator {
        #[pytraverse(skip)]
        format_spec: FormatSpec,
        buffer: ArgBytesLike,
        #[pytraverse(skip)]
        offset: AtomicCell<usize>,
    }

    impl UnpackIterator {
        fn with_buffer(
            vm: &VirtualMachine,
            format_spec: FormatSpec,
            buffer: ArgBytesLike,
        ) -> PyResult<Self> {
            if format_spec.size == 0 {
                Err(new_struct_error(
                    vm,
                    "cannot iteratively unpack with a struct of length 0",
                ))
            } else if !buffer.len().is_multiple_of(format_spec.size) {
                Err(new_struct_error(
                    vm,
                    format!(
                        "iterative unpacking requires a buffer of a multiple of {} bytes",
                        format_spec.size
                    ),
                ))
            } else {
                Ok(Self {
                    format_spec,
                    buffer,
                    offset: AtomicCell::new(0),
                })
            }
        }
    }

    #[pyclass(with(IterNext, Iterable), flags(DISALLOW_INSTANTIATION))]
    impl UnpackIterator {
        #[pymethod]
        fn __length_hint__(&self) -> usize {
            self.buffer.len().saturating_sub(self.offset.load()) / self.format_spec.size
        }
    }
    impl SelfIter for UnpackIterator {}

    impl IterNext for UnpackIterator {
        fn next(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let size = zelf.format_spec.size;
            let offset = zelf.offset.fetch_add(size);
            zelf.buffer.with_ref(|buf| {
                if let Some(buf) = buf.get(offset..offset + size) {
                    zelf.format_spec
                        .unpack(buf, vm)
                        .map(|x| PyIterReturn::Return(x.into()))
                } else {
                    Ok(PyIterReturn::StopIteration(None))
                }
            })
        }
    }

    #[pyfunction]
    fn iter_unpack(
        fmt: IntoStructFormatBytes,
        buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<UnpackIterator> {
        let format_spec = fmt.format_spec(vm)?;
        UnpackIterator::with_buffer(vm, format_spec, buffer)
    }

    #[pyfunction]
    fn calcsize(fmt: IntoStructFormatBytes, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(fmt.format_spec(vm)?.size)
    }

    /// What a `Struct` is once a format has been read into it. Held apart
    /// from the object because `__new__` hands out a `Struct` that `__init__`
    /// has not filled in yet, and `__init__` may be called again on one that
    /// already holds a format.
    #[derive(Debug)]
    struct StructSpec {
        spec: FormatSpec,
        format: PyStrRef,
    }

    #[pyattr]
    #[pyclass(name = "Struct", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyStruct {
        #[pytraverse(skip)]
        inner: PyRwLock<Option<StructSpec>>,
    }

    impl Constructor for PyStruct {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: PyRwLock::new(None),
            })
        }
    }

    impl Initializer for PyStruct {
        type Args = IntoStructFormatBytes;

        fn init(zelf: PyRef<Self>, fmt: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            // The format is read before anything is replaced, so a format that
            // cannot be read leaves the object as it was.
            let spec = fmt.format_spec(vm)?;
            *zelf.inner.write() = Some(StructSpec {
                spec,
                format: fmt.0,
            });
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer, Representable), flags(BASETYPE))]
    impl PyStruct {
        /// The format this was initialized with, or an error if `__init__`
        /// never ran.
        fn ready(&self, vm: &VirtualMachine) -> PyResult<PyMappedRwLockReadGuard<'_, StructSpec>> {
            PyRwLockReadGuard::try_map(self.inner.read(), Option::as_ref)
                .map_err(|_| vm.new_runtime_error("Struct object is not initialized"))
        }

        #[pygetset]
        fn format(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Ok(self.ready(vm)?.format.clone())
        }

        /// The size an uninitialized `Struct` reports, which no format has
        /// yet given a value.
        #[pygetset]
        fn size(&self) -> isize {
            self.inner
                .read()
                .as_ref()
                .map_or(-1, |inner| inner.spec.size as isize)
        }

        #[pymethod]
        fn pack(&self, args: PosArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.ready(vm)?.spec.pack(args.into_vec(), vm)
        }

        #[pymethod]
        fn pack_into(
            &self,
            buffer: ArgMemoryBuffer,
            offset: isize,
            args: PosArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let inner = self.ready(vm)?;
            let offset = get_buffer_offset(buffer.len(), offset, inner.spec.size, true, vm)?;
            buffer.with_ref(|data| {
                inner
                    .spec
                    .pack_into(&mut data[offset..], args.into_vec(), vm)
            })
        }

        #[pymethod]
        fn unpack(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let inner = self.ready(vm)?;
            data.with_ref(|buf| inner.spec.unpack(buf, vm))
        }

        #[pymethod]
        fn unpack_from(&self, args: UpdateFromArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let inner = self.ready(vm)?;
            let size = inner.spec.size;
            let offset = get_buffer_offset(args.buffer.len(), args.offset, size, false, vm)?;
            args.buffer
                .with_ref(|buf| inner.spec.unpack(&buf[offset..][..size], vm))
        }

        #[pymethod]
        fn iter_unpack(
            &self,
            buffer: ArgBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<UnpackIterator> {
            let spec = self.ready(vm)?.spec.clone();
            UnpackIterator::with_buffer(vm, spec, buffer)
        }
    }

    impl Representable for PyStruct {
        #[inline]
        fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
            Ok(wtf8_concat!(
                "Struct('",
                zelf.ready(vm)?.format.as_wtf8(),
                "')"
            ))
        }
    }

    // seems weird that this is part of the "public" API, but whatever
    // TODO: implement a format code->spec cache like CPython does?
    #[pyfunction]
    const fn _clearcache() {}

    #[pyattr(name = "error")]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        struct_error_type(vm).clone()
    }
}
