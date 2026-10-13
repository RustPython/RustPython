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
        AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
        buffer::{FormatSpec, new_struct_error, struct_error_type},
        builtins::{PyBytes, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        common::{
            lock::{PyMappedRwLockReadGuard, PyRwLock, PyRwLockReadGuard},
            rc::PyRc,
        },
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
            // Keep ASCII formats as str and reject non-ASCII str or bytes with
            // the same ValueError as CPython's format parser.
            let fmt = match_class!(match obj {
                s @ PyStr => {
                    if !s.isascii() {
                        return Err(vm.new_value_error("non-ASCII character in struct format"));
                    }
                    s
                }
                b @ PyBytes => {
                    let ascii_str = ascii::AsciiStr::from_ascii(&b)
                        .map_err(|_| vm.new_value_error("non-ASCII character in struct format"))?;
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

        fn cached_format_spec(&self, vm: &VirtualMachine) -> PyResult<PyRc<FormatSpec>> {
            vm.state
                .struct_format_cache
                .get_or_parse(self.0.as_bytes(), vm)
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
        fmt.cached_format_spec(vm)?.pack(args.into_vec(), vm)
    }

    #[pyfunction]
    fn pack_into(
        fmt: IntoStructFormatBytes,
        buffer: ArgMemoryBuffer,
        offset: isize,
        args: PosArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let format_spec = fmt.cached_format_spec(vm)?;
        let offset = get_buffer_offset(buffer.len(), offset, format_spec.size, true, vm)?;
        buffer.with_ref(|data| format_spec.pack_into(&mut data[offset..], args.into_vec(), vm))
    }

    #[pyfunction]
    fn unpack(
        format: IntoStructFormatBytes,
        buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = format.cached_format_spec(vm)?;
        buffer.with_ref(|buf| format_spec.unpack(buf, vm))
    }

    #[derive(FromArgs)]
    struct UpdateFromArgs {
        buffer: ArgBytesLike,
        #[pyarg(any, default)]
        offset: isize,
    }

    #[pyfunction]
    fn unpack_from(
        format: IntoStructFormatBytes,
        args: UpdateFromArgs,
        vm: &VirtualMachine,
    ) -> PyResult<PyTupleRef> {
        let format_spec = format.cached_format_spec(vm)?;
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
        format_spec: PyRc<FormatSpec>,
        buffer: ArgBytesLike,
        #[pytraverse(skip)]
        offset: AtomicCell<usize>,
    }

    impl UnpackIterator {
        fn with_buffer(
            vm: &VirtualMachine,
            format_spec: PyRc<FormatSpec>,
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
        fn __length_hint__(zelf: &Py<Self>) -> usize {
            zelf.buffer.len().saturating_sub(zelf.offset.load()) / zelf.format_spec.size
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
        format: IntoStructFormatBytes,
        buffer: ArgBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<UnpackIterator> {
        let format_spec = format.cached_format_spec(vm)?;
        UnpackIterator::with_buffer(vm, format_spec, buffer)
    }

    #[pyfunction]
    fn calcsize(format: IntoStructFormatBytes, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(format.cached_format_spec(vm)?.size)
    }

    /// What a `Struct` is once a format has been read into it. Held apart
    /// from the object because `__new__` hands out a `Struct` that `__init__`
    /// has not filled in yet, and `__init__` may be called again on one that
    /// already holds a format.
    #[derive(Debug)]
    struct StructSpec {
        spec: PyRc<FormatSpec>,
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

    #[derive(FromArgs)]
    struct StructArgs {
        #[pyarg(any)]
        format: IntoStructFormatBytes,
    }

    impl Initializer for PyStruct {
        type Args = StructArgs;

        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let fmt = args.format;
            // The format is read before anything is replaced, so a format that
            // cannot be read leaves the object as it was.
            let spec = fmt.format_spec(vm)?;
            *zelf.inner.write() = Some(StructSpec {
                spec: PyRc::new(spec),
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
        fn format(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            zelf.inner
                .read()
                .as_ref()
                .map(|inner| inner.format.clone())
                .ok_or_else(|| {
                    vm.new_no_attribute_error(zelf.as_object().to_owned(), vm.ctx.new_str("format"))
                })
        }

        // The size an uninitialized `Struct` reports, which no format has
        // yet given a value.
        #[pygetset]
        fn size(zelf: &Py<Self>) -> isize {
            zelf.inner
                .read()
                .as_ref()
                .map_or(-1, |inner| inner.spec.size as isize)
        }

        #[pymethod]
        fn pack(zelf: &Py<Self>, args: PosArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            zelf.ready(vm)?.spec.pack(args.into_vec(), vm)
        }

        #[pymethod]
        fn pack_into(
            zelf: &Py<Self>,
            buffer: ArgMemoryBuffer,
            offset: isize,
            args: PosArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let inner = zelf.ready(vm)?;
            let offset = get_buffer_offset(buffer.len(), offset, inner.spec.size, true, vm)?;
            buffer.with_ref(|data| {
                inner
                    .spec
                    .pack_into(&mut data[offset..], args.into_vec(), vm)
            })
        }

        #[pymethod]
        fn unpack(
            zelf: &Py<Self>,
            buffer: ArgBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let inner = zelf.ready(vm)?;
            buffer.with_ref(|buf| inner.spec.unpack(buf, vm))
        }

        #[pymethod]
        fn unpack_from(
            zelf: &Py<Self>,
            args: UpdateFromArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let inner = zelf.ready(vm)?;
            let size = inner.spec.size;
            let offset = get_buffer_offset(args.buffer.len(), args.offset, size, false, vm)?;
            args.buffer
                .with_ref(|buf| inner.spec.unpack(&buf[offset..][..size], vm))
        }

        #[pymethod]
        fn iter_unpack(
            zelf: &Py<Self>,
            buffer: ArgBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<UnpackIterator> {
            let spec = zelf.ready(vm)?.spec.clone();
            UnpackIterator::with_buffer(vm, spec, buffer)
        }

        #[pymethod]
        fn __sizeof__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<usize> {
            let inner = zelf.ready(vm)?;
            Ok(core::mem::size_of::<Self>() + inner.spec.codes_sizeof())
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
    #[pyfunction]
    fn _clearcache(vm: &VirtualMachine) {
        vm.state.struct_format_cache.clear();
    }

    #[pyattr(name = "error")]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        struct_error_type(vm).to_owned()
    }
}
