//! Python struct module.
//!
//! Docs: https://docs.python.org/3/library/struct.html
//!
//! Use this rust module to do byte packing:
//! https://docs.rs/byteorder/1.2.6/byteorder/

pub(crate) use _struct::make_module;

#[pymodule]
pub(crate) mod _struct {
    use crate::vm::{
        buffer::{new_struct_error, struct_error_type, FormatSpec},
        builtins::{PyBytes, PyStr, PyStrRef, PyTupleRef, PyTypeRef},
        function::{ArgBytesLike, ArgMemoryBuffer, PosArgs},
        match_class,
        protocol::PyIterReturn,
        types::{Constructor, IterNext, IterNextIterable},
        AsObject, PyObjectRef, PyObjectView, PyPayload, PyResult, TryFromObject, VirtualMachine,
    };
    use crossbeam_utils::atomic::AtomicCell;

    struct IntoStructFormatBytes(PyStrRef);

    impl TryFromObject for IntoStructFormatBytes {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            // CPython turns str to bytes but we do reversed way here
            // The only performance difference is this transition cost
            let fmt = match_class! {
                match obj {
                    s @ PyStr => if s.is_ascii() {
                        Some(s)
                    } else {
                        None
                    },
                    b @ PyBytes => if b.is_ascii() {
                        Some(unsafe {
                            PyStr::new_ascii_unchecked(b.as_bytes().to_vec())
                        }.into_ref(vm))
                    } else {
                        None
                    },
                    other => return Err(vm.new_type_error(format!("Struct() argument 1 must be a str or bytes object, not {}", other.class().name()))),
                }
            }.ok_or_else(|| vm.new_unicode_decode_error("Struct format must be a ascii string".to_owned()))?;
            Ok(IntoStructFormatBytes(fmt))
        }
    }

    impl IntoStructFormatBytes {
        fn format_spec(&self, vm: &VirtualMachine) -> PyResult<FormatSpec> {
            FormatSpec::parse(self.0.as_str().as_bytes(), vm)
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
                    format!(
                        "offset {} out of range for {}-byte buffer",
                        offset, buffer_len
                    ),
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
            if offset >= buffer_len {
                let msg = format!(
                    "{op} requires a buffer of at least {required} bytes for {op_action} {needed} \
                    bytes at offset {offset} (actual buffer size is {buffer_len})",
                    op = op,
                    op_action = op_action,
                    required = needed + offset as usize,
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
                    format!("no space to pack {} bytes at offset {}", needed, offset)
                } else {
                    format!(
                        "not enough data to unpack {} bytes at offset {}",
                        needed, offset
                    )
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
        #[pyarg(any, default = "0")]
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
    #[pyclass(name = "unpack_iterator")]
    #[derive(Debug, PyPayload)]
    struct UnpackIterator {
        format_spec: FormatSpec,
        buffer: ArgBytesLike,
        offset: AtomicCell<usize>,
    }

    impl UnpackIterator {
        fn new(
            vm: &VirtualMachine,
            format_spec: FormatSpec,
            buffer: ArgBytesLike,
        ) -> PyResult<UnpackIterator> {
            if format_spec.size == 0 {
                Err(new_struct_error(
                    vm,
                    "cannot iteratively unpack with a struct of length 0".to_owned(),
                ))
            } else if buffer.len() % format_spec.size != 0 {
                Err(new_struct_error(
                    vm,
                    format!(
                        "iterative unpacking requires a buffer of a multiple of {} bytes",
                        format_spec.size
                    ),
                ))
            } else {
                Ok(UnpackIterator {
                    format_spec,
                    buffer,
                    offset: AtomicCell::new(0),
                })
            }
        }
    }

    #[pyimpl(with(IterNext))]
    impl UnpackIterator {
        #[pymethod(magic)]
        fn length_hint(&self) -> usize {
            self.buffer.len().saturating_sub(self.offset.load()) / self.format_spec.size
        }
    }
    impl IterNextIterable for UnpackIterator {}
    impl IterNext for UnpackIterator {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
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
        UnpackIterator::new(vm, format_spec, buffer)
    }

    #[pyfunction]
    fn calcsize(fmt: IntoStructFormatBytes, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(fmt.format_spec(vm)?.size)
    }

    #[pyattr]
    #[pyclass(name = "Struct")]
    #[derive(Debug, PyPayload)]
    struct PyStruct {
        spec: FormatSpec,
        format: PyStrRef,
    }

    impl Constructor for PyStruct {
        type Args = IntoStructFormatBytes;

        fn py_new(cls: PyTypeRef, fmt: Self::Args, vm: &VirtualMachine) -> PyResult {
            let spec = fmt.format_spec(vm)?;
            let format = fmt.0;
            PyStruct { spec, format }.into_pyresult_with_type(vm, cls)
        }
    }

    #[pyimpl(with(Constructor))]
    impl PyStruct {
        #[pyproperty]
        fn format(&self) -> PyStrRef {
            self.format.clone()
        }

        #[pyproperty]
        #[inline]
        fn size(&self) -> usize {
            self.spec.size
        }

        #[pymethod]
        fn pack(&self, args: PosArgs, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.spec.pack(args.into_vec(), vm)
        }

        #[pymethod]
        fn pack_into(
            &self,
            buffer: ArgMemoryBuffer,
            offset: isize,
            args: PosArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let offset = get_buffer_offset(buffer.len(), offset, self.size(), true, vm)?;
            buffer.with_ref(|data| {
                self.spec
                    .pack_into(&mut data[offset..], args.into_vec(), vm)
            })
        }

        #[pymethod]
        fn unpack(&self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            data.with_ref(|buf| self.spec.unpack(buf, vm))
        }

        #[pymethod]
        fn unpack_from(&self, args: UpdateFromArgs, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            let offset = get_buffer_offset(args.buffer.len(), args.offset, self.size(), false, vm)?;
            args.buffer
                .with_ref(|buf| self.spec.unpack(&buf[offset..][..self.size()], vm))
        }

        #[pymethod]
        fn iter_unpack(
            &self,
            buffer: ArgBytesLike,
            vm: &VirtualMachine,
        ) -> PyResult<UnpackIterator> {
            UnpackIterator::new(vm, self.spec.clone(), buffer)
        }
    }

    // seems weird that this is part of the "public" API, but whatever
    // TODO: implement a format code->spec cache like CPython does?
    #[pyfunction]
    fn _clearcache() {}

    #[pyattr(name = "error")]
    fn error_type(vm: &VirtualMachine) -> PyTypeRef {
        struct_error_type(vm).clone()
    }
}
