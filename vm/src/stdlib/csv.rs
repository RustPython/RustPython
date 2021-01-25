use itertools::{self, Itertools};
use std::fmt::{self, Debug, Formatter};

use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::common::lock::PyMutex;
use crate::function::{ArgumentError, FromArgs, FuncArgs};
use crate::iterator;
use crate::pyobject::{
    BorrowValue, PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
    TypeProtocol,
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

struct ReaderOption {
    delimiter: u8,
    quotechar: u8,
}

impl FromArgs for ReaderOption {
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

        Ok(ReaderOption {
            delimiter,
            quotechar,
        })
    }
}

impl ReaderOption {
    fn to_reader(&self) -> csv_core::Reader {
        csv_core::ReaderBuilder::new()
            .delimiter(self.delimiter)
            .quote(self.quotechar)
            .build()
    }
}

struct ReadState {
    buffer: Vec<u8>,
    output_ends: Vec<usize>,
    reader: csv_core::Reader,
}

#[pyclass(module = "csv", name = "Reader")]
struct Reader {
    iter: PyObjectRef,
    state: PyMutex<ReadState>,
}

impl Debug for Reader {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
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
                csv_core::ReadRecordResult::OutputFull => {
                    let new_size = buffer.len() * 2;
                    buffer.resize(new_size, 0u8);
                }
                csv_core::ReadRecordResult::OutputEndsFull => {
                    let new_size = output_ends.len() * 2;
                    output_ends.resize(new_size, 0);
                }
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
    options: ReaderOption,
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

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let reader_type = Reader::make_class(ctx);

    let error = create_simple_type("Error", &ctx.exceptions.exception_type);

    py_module!(vm, "_csv", {
        "reader" => named_function!(ctx, _csv, reader),
        "Reader" => reader_type,
        "Error"  => error,
        // constants
        "QUOTE_MINIMAL" => ctx.new_int(QuoteStyle::QuoteMinimal as i32),
        "QUOTE_ALL" => ctx.new_int(QuoteStyle::QuoteAll as i32),
        "QUOTE_NONNUMERIC" => ctx.new_int(QuoteStyle::QuoteNonnumeric as i32),
        "QUOTE_NONE" => ctx.new_int(QuoteStyle::QuoteNone as i32),
    })
}
