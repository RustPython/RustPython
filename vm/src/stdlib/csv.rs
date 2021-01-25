use itertools::{self, Itertools};
use std::fmt;

use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::common::lock::PyMutex;
use crate::function::{ArgumentError, FromArgs, FuncArgs};
use crate::iterator;
use crate::pyobject::{
    BorrowValue, PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
    TryFromObject, TypeProtocol,
};
use crate::slots::PyIter;
use crate::types::create_simple_type;
use crate::VirtualMachine;

#[repr(i32)]
pub enum QuoteStyle {
    QuoteMinimal,
    QuoteAll,
    QuoteNonnumeric,
    QuoteNone,
}

struct FormatOptions {
    delimiter: u8,
    quotechar: u8,
}

impl FromArgs for FormatOptions {
    fn from_args(vm: &VirtualMachine, args: &mut FuncArgs) -> Result<Self, ArgumentError> {
        let delimiter = if let Some(delimiter) = args.kwargs.remove("delimiter") {
            PyStrRef::try_from_object(vm, delimiter)?
                .borrow_value()
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
                .borrow_value()
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

#[pyclass(module = "_csv", name = "reader")]
struct Reader {
    iter: PyObjectRef,
    state: PyMutex<ReadState>,
}

impl fmt::Debug for Reader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_csv.reader")
    }
}

impl PyValue for Reader {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

#[pyimpl(with(PyIter))]
impl Reader {}
impl PyIter for Reader {
    fn next(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let string = iterator::call_next(vm, &zelf.iter)?;
        let string = string.downcast::<PyStr>().map_err(|obj| {
            vm.new_type_error(format!(
                "iterator should return strings, not {} (the file should be opened in text mode)",
                obj.class().name
            ))
        })?;
        let input = string.borrow_value().as_bytes();

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
                csv_core::ReadRecordResult::End => return Err(vm.new_stop_iteration()),
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
                std::str::from_utf8(&buffer[range])
                    .map(|s| vm.ctx.new_str(s.to_owned()))
                    // not sure if this is possible - the input was all strings
                    .map_err(|_e| vm.new_unicode_decode_error("csv not utf8".to_owned()))
            })
            .collect::<Result<_, _>>()?;
        Ok(vm.ctx.new_list(out))
    }
}

fn _csv_reader(
    iter: PyObjectRef,
    options: FormatOptions,
    // TODO: handle quote style, etc
    _rest: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<Reader> {
    let iter = iterator::get_iter(vm, iter)?;
    Ok(Reader {
        iter,
        state: PyMutex::new(ReadState {
            buffer: vec![0; 1024],
            output_ends: vec![0; 16],
            reader: options.to_reader(),
        }),
    })
}

struct WriteState {
    buffer: Vec<u8>,
    writer: csv_core::Writer,
}

#[pyclass(module = "_csv", name = "writer")]
struct Writer {
    write: PyObjectRef,
    state: PyMutex<WriteState>,
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "_csv.writer")
    }
}

impl PyValue for Writer {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
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

        let row = PyIterable::try_from_object(vm, row)?;
        for field in row.iter(vm)? {
            let field: PyObjectRef = field?;
            let stringified;
            let data: &[u8] = match_class!(match field {
                ref s @ PyStr => s.borrow_value().as_bytes(),
                crate::builtins::PyNone => b"",
                ref obj => {
                    stringified = vm.to_str(obj)?;
                    stringified.borrow_value().as_bytes()
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
    fn writerows(&self, rows: PyIterable, vm: &VirtualMachine) -> PyResult<()> {
        for row in rows.iter(vm)? {
            self.writerow(row?, vm)?;
        }
        Ok(())
    }
}

fn _csv_writer(
    file: PyObjectRef,
    options: FormatOptions,
    // TODO: handle quote style, etc
    _rest: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult<Writer> {
    let write = match vm.get_attribute_opt(file.clone(), "write")? {
        Some(write_meth) => write_meth,
        None if vm.is_callable(&file) => file,
        None => return Err(vm.new_type_error("argument 1 must have a \"write\" method".to_owned())),
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    Reader::make_class(ctx);
    Writer::make_class(ctx);

    let error = create_simple_type("Error", &ctx.exceptions.exception_type);

    py_module!(vm, "_csv", {
        "reader" => named_function!(ctx, _csv, reader),
        "writer" => named_function!(ctx, _csv, writer),
        "Error"  => error,
        // constants
        "QUOTE_MINIMAL" => ctx.new_int(QuoteStyle::QuoteMinimal as i32),
        "QUOTE_ALL" => ctx.new_int(QuoteStyle::QuoteAll as i32),
        "QUOTE_NONNUMERIC" => ctx.new_int(QuoteStyle::QuoteNonnumeric as i32),
        "QUOTE_NONE" => ctx.new_int(QuoteStyle::QuoteNone as i32),
    })
}
