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
        AsObject, FromArgs, Py, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject,
        VirtualMachine,
        buffer::{FormatSpec, new_struct_error, struct_error_type},
        builtins::{PyBytes, PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        common::{lock::PyRwLock, rc::PyRc},
        function::{ArgBytesLike, ArgMemoryBuffer, FuncArgs, PosArgs},
        match_class,
        protocol::PyIterReturn,
        stdlib::_warnings,
        types::{Constructor, Initializer, IterNext, Iterable, Representable, SelfIter},
    };
    use crossbeam_utils::atomic::AtomicCell;
    use rustpython_common::wtf8::{Wtf8Buf, wtf8_concat};

    #[derive(Traverse)]
    struct IntoStructFormatBytes(PyStrRef);

    impl TryFromObject for IntoStructFormatBytes {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            // CPython decodes bytes with ASCII and surrogateescape, then rejects
            // every non-ASCII format before parsing, for both str and bytes.
            let fmt = match_class!(match obj {
                s @ PyStr => {
                    if !s.isascii() {
                        return Err(vm.new_value_error("non-ASCII character in struct format"));
                    }
                    if s.class().is(vm.ctx.types.str_type) {
                        s
                    } else {
                        vm.ctx.new_str(s.as_wtf8())
                    }
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
    /// because `__new__` can still hand out an uninitialized `Struct`, and
    /// `__init__` may be called again on one that already holds a format.
    #[derive(Clone, Debug)]
    struct StructSpec {
        spec: PyRc<FormatSpec>,
        format: PyStrRef,
    }

    #[derive(FromArgs)]
    struct StructArgs {
        #[pyarg(any)]
        format: PyObjectRef,
    }

    impl StructArgs {
        fn bind(args: FuncArgs, vm: &VirtualMachine) -> PyResult<Self> {
            let nargs = args.args.len() + args.kwargs.len();
            if nargs > 1 {
                let keyword = if args.args.is_empty() { "keyword " } else { "" };
                return Err(vm.new_type_error(format!(
                    "Struct() takes at most 1 {keyword}argument ({nargs} given)"
                )));
            }
            args.bind_for(vm, "Struct")
        }
    }

    #[pyattr]
    #[pyclass(name = "Struct", traverse)]
    #[derive(Debug, PyPayload)]
    struct PyStruct {
        #[pytraverse(skip)]
        inner: PyRwLock<Option<StructSpec>>,
        #[pytraverse(skip)]
        init_called: AtomicCell<bool>,
    }

    impl PyStruct {
        fn uses_struct_init(cls: &Py<PyType>, vm: &VirtualMachine) -> bool {
            cls.slots()
                .init
                .load()
                .zip(Self::class(&vm.ctx).slots().init.load())
                .is_some_and(|(init, struct_init)| core::ptr::fn_addr_eq(init, struct_init))
        }

        fn parse_format(format: PyObjectRef, vm: &VirtualMachine) -> PyResult<StructSpec> {
            let format = IntoStructFormatBytes::try_from_object(vm, format)?;
            Ok(StructSpec {
                spec: PyRc::new(format.format_spec(vm)?),
                format: format.0,
            })
        }

        fn same_format(format: &PyStr, candidate: &PyObject) -> bool {
            candidate
                .downcast_ref::<PyStr>()
                .is_some_and(|candidate| candidate.as_bytes() == format.as_bytes())
                || candidate
                    .downcast_ref::<PyBytes>()
                    .is_some_and(|candidate| candidate.as_bytes() == format.as_bytes())
        }
    }

    impl Constructor for PyStruct {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let uses_struct_new = cls
                .slots()
                .new
                .load()
                .zip(Self::class(&vm.ctx).slots().new.load())
                .is_some_and(|(new, struct_new)| core::ptr::fn_addr_eq(new, struct_new));
            let uses_struct_init = Self::uses_struct_init(&cls, vm);
            let format = if !uses_struct_new {
                // A subclass explicitly called Struct.__new__ from its own __new__.
                let args = StructArgs::bind(args, vm)?;
                Some(args.format)
            } else {
                let format = match args.args.as_slice() {
                    [format] if args.kwargs.is_empty() => Some(format.clone()),
                    [] if args.kwargs.len() == 1 => args.kwargs.get("format").cloned(),
                    _ => None,
                };
                if format.is_none() && !uses_struct_init {
                    let nargs = args.args.len() + args.kwargs.len();
                    let message = if nargs > 1 {
                        format!("Struct() takes at most 1 argument ({nargs} given)")
                    } else {
                        "Struct() missing required argument 'format' (pos 1)".to_owned()
                    };
                    _warnings::warn(vm.ctx.exceptions.deprecation_warning, message, 1, vm)?;
                }
                format
            };
            // Allocate before parsing so a failing subclass constructor still
            // finalizes its half-initialized instance.
            let zelf = Self::py_new(&cls, FuncArgs::default(), vm)?.into_ref_with_type(vm, cls)?;
            if let Some(format) = format {
                match Self::parse_format(format, vm) {
                    Ok(inner) => {
                        let old = zelf.inner.write().replace(inner);
                        drop(old);
                    }
                    Err(err) if uses_struct_new && !uses_struct_init => {
                        _warnings::warn(
                            vm.ctx.exceptions.deprecation_warning,
                            format!(
                                "Invalid 'format' argument for Struct.__new__(): {}",
                                err.as_object().str(vm)?
                            ),
                            1,
                            vm,
                        )?;
                    }
                    Err(err) => return Err(err),
                }
            }
            Ok(zelf.into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            Ok(Self {
                inner: PyRwLock::new(None),
                init_called: AtomicCell::new(false),
            })
        }
    }

    impl Initializer for PyStruct {
        type Args = FuncArgs;

        fn init(zelf: &Py<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
            let format = zelf.inner.read().as_ref().map(|inner| inner.format.clone());
            if !zelf.init_called.load()
                && Self::uses_struct_init(zelf.class(), vm)
                && format.is_some()
            {
                // The implicit __init__ must not reinterpret a custom __new__'s arguments.
                zelf.init_called.store(true);
                return Ok(());
            }
            let args = StructArgs::bind(args, vm)?;
            if let Some(format) = format {
                if Self::same_format(&format, &args.format) {
                    zelf.init_called.store(true);
                    return Ok(());
                }
                let message = if zelf.init_called.load() {
                    "Re-initialization of Struct by calling the __init__() method \
                     will not work in future Python versions"
                } else {
                    "Different format arguments for __new__() and __init__() \
                     methods of Struct"
                };
                _warnings::warn(vm.ctx.exceptions.future_warning, message.to_owned(), 1, vm)?;
            }
            // Warn and parse before replacing the format. Warning hooks may reenter,
            // and either a warning or a conversion error must preserve the old state.
            let inner = Self::parse_format(args.format, vm)?;
            let old = zelf.inner.write().replace(inner);
            drop(old);
            zelf.init_called.store(true);
            Ok(())
        }
    }

    #[pyclass(with(Constructor, Initializer, Representable), flags(BASETYPE))]
    impl PyStruct {
        // Hold an owned snapshot while conversions and warning hooks can reenter.
        fn ready(&self, vm: &VirtualMachine) -> PyResult<StructSpec> {
            let inner = self.inner.read().clone();
            inner.ok_or_else(|| vm.new_runtime_error("Struct object is not initialized"))
        }

        #[pygetset]
        fn format(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let format = zelf.inner.read().as_ref().map(|inner| inner.format.clone());
            format.ok_or_else(|| {
                vm.new_no_attribute_error(zelf.to_owned().into(), vm.ctx.new_str("format"))
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
            let spec = zelf.ready(vm)?.spec;
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
