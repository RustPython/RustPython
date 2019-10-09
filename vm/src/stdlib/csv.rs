use std::cell::RefCell;
use std::fmt::{self, Debug, Formatter};

use csv as rust_csv;
use itertools::join;

use crate::obj::objiter;
use crate::obj::objstr::PyString;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{IntoPyObject, TryFromObject, TypeProtocol};
use crate::pyobject::{PyClassImpl, PyIterable, PyObjectRef, PyRef, PyResult, PyValue};
use crate::types::create_type;
use crate::VirtualMachine;

#[repr(i32)]
pub enum QuoteStyle {
    QuoteMinimal,
    QuoteAll,
    QuoteNonnumeric,
    QuoteNone,
}

pub fn build_reader(iterable: PyIterable<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
    Reader::new(iterable).into_ref(vm).into_pyobject(vm)
}

fn into_strings(iterable: &PyIterable<PyObjectRef>, vm: &VirtualMachine) -> PyResult<Vec<String>> {
    iterable
        .iter(vm)?
        .map(|py_obj_ref| {
            match_class!(match py_obj_ref? {
                py_str @ PyString => Ok(py_str.as_str().trim().to_owned()),
                obj => {
                    let msg = format!(
            "iterator should return strings, not {} (did you open the file in text mode?)",
            obj.class().name
          );
                    Err(vm.new_type_error(msg))
                }
            })
        })
        .collect::<PyResult<Vec<String>>>()
}

type MemIO = std::io::Cursor<Vec<u8>>;

#[allow(dead_code)]
enum ReadState {
    PyIter(PyIterable<PyObjectRef>),
    CsvIter(rust_csv::StringRecordsIntoIter<MemIO>),
}

impl ReadState {
    fn new(iter: PyIterable) -> Self {
        ReadState::PyIter(iter)
    }

    fn cast_to_reader(&mut self, vm: &VirtualMachine) -> PyResult<()> {
        if let ReadState::PyIter(ref iterable) = self {
            let lines = into_strings(iterable, vm)?;
            let contents = join(lines, "\n");

            let bytes = Vec::from(contents.as_bytes());
            let reader = MemIO::new(bytes);

            let csv_iter = rust_csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(reader)
                .into_records();

            *self = ReadState::CsvIter(csv_iter);
        }
        Ok(())
    }
}

#[pyclass(name = "Reader")]
struct Reader {
    state: RefCell<ReadState>,
}

impl Debug for Reader {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "_csv.reader")
    }
}

impl PyValue for Reader {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("_csv", "Reader")
    }
}

impl Reader {
    fn new(iter: PyIterable<PyObjectRef>) -> Self {
        let state = RefCell::new(ReadState::new(iter));
        Reader { state }
    }
}

#[pyimpl]
impl Reader {
    #[pymethod(name = "__iter__")]
    fn iter(this: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        this.state.borrow_mut().cast_to_reader(vm)?;
        this.into_pyobject(vm)
    }

    #[pymethod(name = "__next__")]
    fn next(&self, vm: &VirtualMachine) -> PyResult {
        let mut state = self.state.borrow_mut();
        state.cast_to_reader(vm)?;

        if let ReadState::CsvIter(ref mut reader) = &mut *state {
            if let Some(row) = reader.next() {
                match row {
                    Ok(records) => {
                        let iter = records
                            .into_iter()
                            .map(|bytes| bytes.into_pyobject(vm))
                            .collect::<PyResult<Vec<_>>>()?;
                        Ok(vm.ctx.new_list(iter))
                    }
                    Err(_) => {
                        let msg = String::from("Decode Error");
                        let decode_error = vm.new_unicode_decode_error(msg);
                        Err(decode_error)
                    }
                }
            } else {
                Err(objiter::new_stop_iteration(vm))
            }
        } else {
            unreachable!()
        }
    }
}

fn csv_reader(fp: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    if let Ok(iterable) = PyIterable::<PyObjectRef>::try_from_object(vm, fp) {
        build_reader(iterable, vm)
    } else {
        Err(vm.new_type_error("argument 1 must be an iterator".to_string()))
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let reader_type = Reader::make_class(ctx);

    let error = create_type(
        "Error",
        &ctx.types.type_type,
        &ctx.exceptions.exception_type,
    );

    py_module!(vm, "_csv", {
      "reader" => ctx.new_rustfunc(csv_reader),
      "Reader" => reader_type,
      "Error"  => error,
      // constants
      "QUOTE_MINIMAL" => ctx.new_int(QuoteStyle::QuoteMinimal as i32),
      "QUOTE_ALL" => ctx.new_int(QuoteStyle::QuoteAll as i32),
      "QUOTE_NONNUMERIC" => ctx.new_int(QuoteStyle::QuoteNonnumeric as i32),
      "QUOTE_NONE" => ctx.new_int(QuoteStyle::QuoteNone as i32),
    })
}
