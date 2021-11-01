use crate::vm::{PyClassImpl, PyObjectRef, VirtualMachine};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    _csv::Reader::make_class(ctx);
    _csv::Writer::make_class(ctx);
    _csv::make_module(vm)
}

#[pymodule]
mod _csv {
    use crate::common::lock::PyMutex;
    use crate::vm::{
        builtins::{PyStr, PyStrRef, PyType, PyTypeRef},
        function::{ArgIterable, ArgumentError, FromArgs, FuncArgs},
        match_class,
        protocol::{PyIter, PyIterReturn},
        types::{IterNext, IterNextIterable},
        PyObjectRef, PyObjectView, PyResult, PyValue, TryFromObject, TypeProtocol, VirtualMachine,
    };
    use itertools::{self, Itertools};
    use std::fmt;

    #[pyattr]
    const QUOTE_MINIMAL: i32 = QuoteStyle::Minimal as i32;
    #[pyattr]
    const QUOTE_ALL: i32 = QuoteStyle::All as i32;
    #[pyattr]
    const QUOTE_NONNUMERIC: i32 = QuoteStyle::Nonnumeric as i32;
    #[pyattr]
    const QUOTE_NONE: i32 = QuoteStyle::None as i32;

    #[pyattr(name = "Error")]
    fn error(vm: &VirtualMachine) -> PyTypeRef {
        PyType::new_simple_ref("_csv.Error", &vm.ctx.exceptions.exception_type).unwrap()
    }

    #[pyfunction]
    fn reader(
        iter: PyIter,
        options: FormatOptions,
        // TODO: handle quote style, etc
        _rest: FuncArgs,
        _vm: &VirtualMachine,
    ) -> PyResult<Reader> {
        Ok(Reader {
            iter,
            state: PyMutex::new(ReadState {
                buffer: vec![0; 1024],
                output_ends: vec![0; 16],
                reader: options.to_reader(),
            }),
        })
    }

    #[pyfunction]
    fn writer(
        file: PyObjectRef,
        options: FormatOptions,
        // TODO: handle quote style, etc
        _rest: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<Writer> {
        let write = match vm.get_attribute_opt(file.clone(), "write")? {
            Some(write_meth) => write_meth,
            None if vm.is_callable(&file) => file,
            None => {
                return Err(vm.new_type_error("argument 1 must have a \"write\" method".to_owned()))
            }
        };

        Ok(Writer {
            write,
            state: PyMutex::new(WriteState {
                buffer: vec![0; 1024],
                writer: options.to_writer(),
            }),
        })
    }

    #[inline]
    fn resize_buf<T: num_traits::PrimInt>(buf: &mut Vec<T>) {
        let new_size = buf.len() * 2;
        buf.resize(new_size, T::zero());
    }

    #[repr(i32)]
    pub enum QuoteStyle {
        Minimal = 0,
        All = 1,
        Nonnumeric = 2,
        None = 3,
    }

    struct FormatOptions {
        delimiter: u8,
        quotechar: u8,
    }

    impl FromArgs for FormatOptions {
        fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
            let delimiter = if let Some(delimiter) = args.kwargs.remove("delimiter") {
                PyStrRef::try_from_object(vm, delimiter)?
                    .as_str()
                    .bytes()
                    .exactly_one()
                    .map_err(|_| {
                        let msg = r#""delimiter" must be a 1-character string"#;
                        vm.new_type_error(msg.to_owned())
                    })?
            } else {
                b','
            };

            let quotechar = if let Some(quotechar) = args.kwargs.remove("quotechar") {
                PyStrRef::try_from_object(vm, quotechar)?
                    .as_str()
                    .bytes()
                    .exactly_one()
                    .map_err(|_| {
                        let msg = r#""quotechar" must be a 1-character string"#;
                        vm.new_type_error(msg.to_owned())
                    })?
            } else {
                b'"'
            };

            Ok(FormatOptions {
                delimiter,
                quotechar,
            })
        }
    }

    impl FormatOptions {
        fn to_reader(&self) -> csv_core::Reader {
            csv_core::ReaderBuilder::new()
                .delimiter(self.delimiter)
                .quote(self.quotechar)
                .terminator(csv_core::Terminator::CRLF)
                .build()
        }
        fn to_writer(&self) -> csv_core::Writer {
            csv_core::WriterBuilder::new()
                .delimiter(self.delimiter)
                .quote(self.quotechar)
                .terminator(csv_core::Terminator::CRLF)
                .build()
        }
    }

    struct ReadState {
        buffer: Vec<u8>,
        output_ends: Vec<usize>,
        reader: csv_core::Reader,
    }

    #[pyclass(noattr, module = "_csv", name = "reader")]
    #[derive(PyValue)]
    pub(super) struct Reader {
        iter: PyIter,
        state: PyMutex<ReadState>,
    }

    impl fmt::Debug for Reader {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "_csv.reader")
        }
    }

    #[pyimpl(with(IterNext))]
    impl Reader {}
    impl IterNextIterable for Reader {}
    impl IterNext for Reader {
        fn next(zelf: &PyObjectView<Self>, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
            let string = match zelf.iter.next(vm)? {
                PyIterReturn::Return(obj) => obj,
                PyIterReturn::StopIteration(v) => return Ok(PyIterReturn::StopIteration(v)),
            };
            let string = string.downcast::<PyStr>().map_err(|obj| {
                vm.new_type_error(format!(
                "iterator should return strings, not {} (the file should be opened in text mode)",
                obj.class().name()
            ))
            })?;
            let input = string.as_str().as_bytes();

            let mut state = zelf.state.lock();
            let ReadState {
                buffer,
                output_ends,
                reader,
            } = &mut *state;

            let mut input_offset = 0;
            let mut output_offset = 0;
            let mut output_ends_offset = 0;

            loop {
                let (res, nread, nwritten, nends) = reader.read_record(
                    &input[input_offset..],
                    &mut buffer[output_offset..],
                    &mut output_ends[output_ends_offset..],
                );
                input_offset += nread;
                output_offset += nwritten;
                output_ends_offset += nends;
                match res {
                    csv_core::ReadRecordResult::InputEmpty => {}
                    csv_core::ReadRecordResult::OutputFull => resize_buf(buffer),
                    csv_core::ReadRecordResult::OutputEndsFull => resize_buf(output_ends),
                    csv_core::ReadRecordResult::Record => break,
                    csv_core::ReadRecordResult::End => {
                        return Ok(PyIterReturn::StopIteration(None))
                    }
                }
            }
            let rest = &input[input_offset..];
            if !rest.iter().all(|&c| matches!(c, b'\r' | b'\n')) {
                return Err(vm.new_value_error(
                    "new-line character seen in unquoted field - \
                    do you need to open the file in universal-newline mode?"
                        .to_owned(),
                ));
            }

            let mut prev_end = 0;
            let out = output_ends[..output_ends_offset]
                .iter()
                .map(|&end| {
                    let range = prev_end..end;
                    prev_end = end;
                    let s = std::str::from_utf8(&buffer[range])
                        // not sure if this is possible - the input was all strings
                        .map_err(|_e| vm.new_unicode_decode_error("csv not utf8".to_owned()))?;
                    Ok(vm.ctx.new_str(s).into())
                })
                .collect::<Result<_, _>>()?;
            Ok(PyIterReturn::Return(vm.ctx.new_list(out).into()))
        }
    }

    struct WriteState {
        buffer: Vec<u8>,
        writer: csv_core::Writer,
    }

    #[pyclass(noattr, module = "_csv", name = "writer")]
    #[derive(PyValue)]
    pub(super) struct Writer {
        write: PyObjectRef,
        state: PyMutex<WriteState>,
    }

    impl fmt::Debug for Writer {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "_csv.writer")
        }
    }

    #[pyimpl]
    impl Writer {
        #[pymethod]
        fn writerow(&self, row: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let mut state = self.state.lock();
            let WriteState { buffer, writer } = &mut *state;

            let mut buffer_offset = 0;

            macro_rules! handle_res {
                ($x:expr) => {{
                    let (res, nwritten) = $x;
                    buffer_offset += nwritten;
                    match res {
                        csv_core::WriteResult::InputEmpty => break,
                        csv_core::WriteResult::OutputFull => resize_buf(buffer),
                    }
                }};
            }

            let row = ArgIterable::try_from_object(vm, row)?;
            for field in row.iter(vm)? {
                let field: PyObjectRef = field?;
                let stringified;
                let data: &[u8] = match_class!(match field {
                    ref s @ PyStr => s.as_str().as_bytes(),
                    crate::builtins::PyNone => b"",
                    ref obj => {
                        stringified = obj.str(vm)?;
                        stringified.as_str().as_bytes()
                    }
                });

                let mut input_offset = 0;

                loop {
                    let (res, nread, nwritten) =
                        writer.field(&data[input_offset..], &mut buffer[buffer_offset..]);
                    input_offset += nread;
                    handle_res!((res, nwritten));
                }

                loop {
                    handle_res!(writer.delimiter(&mut buffer[buffer_offset..]));
                }
            }

            loop {
                handle_res!(writer.terminator(&mut buffer[buffer_offset..]));
            }

            let s = std::str::from_utf8(&buffer[..buffer_offset])
                .map_err(|_| vm.new_unicode_decode_error("csv not utf8".to_owned()))?;

            vm.invoke(&self.write, (s.to_owned(),))
        }

        #[pymethod]
        fn writerows(&self, rows: ArgIterable, vm: &VirtualMachine) -> PyResult<()> {
            for row in rows.iter(vm)? {
                self.writerow(row?, vm)?;
            }
            Ok(())
        }
    }
}
