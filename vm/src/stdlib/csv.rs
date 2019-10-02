use std::cell::RefCell;
use std::fmt::{self, Debug, Formatter};
use std::io::Cursor;
use std::rc::Rc;

use crate::VirtualMachine;
use crate::pyobject::{IntoPyObject, PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::obj::objtype::PyClassRef;
use crate::obj::objiter;

use crate::obj::objbytes::PyBytes;
use crate::obj::objstr::PyString;

// external crates go here.
use csv as rust_csv;

#[pyclass(name = "Reader")]
pub struct Reader {
  reader: Rc<RefCell<rust_csv::StringRecordsIntoIter<Cursor<Vec<u8>>>>>,
}

impl Reader {
  fn new(bytes: Vec<u8>) -> Self {

    let read = Cursor::new(bytes);
    let reader = rust_csv::ReaderBuilder::new()
                  .has_headers(false)
                  .from_reader(read)
                  .into_records();

    let reader = Rc::new(RefCell::new(reader));
    Reader { reader }
  }
}

impl Debug for Reader {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    write!(f, "_csv.reader")
  }
}

impl PyValue for Reader {
  fn class(vm: &VirtualMachine) -> PyClassRef {
    vm.class("csv", "Reader")
  }
}

#[pyimpl]
impl Reader {
  #[pymethod(name = "__iter__")]
  fn iter(&self, vm: &VirtualMachine) -> PyResult {
    let reader = Rc::clone(&self.reader);
    let reader = Reader { reader };
    Ok(reader
      .into_ref(vm)
      .into_object())
  }

  #[pymethod(name = "__next__")]
  fn next(&self, vm: &VirtualMachine) -> PyResult {
    if let Some(record) = self.reader.borrow_mut().next() {
      match record {
        Ok(record) => {
          let iter = record
                      .into_iter()
                      .map(|bytes| bytes.into_pyobject(vm))
                      .collect::<PyResult<Vec<_>>>()?;
          Ok(vm.ctx.new_list(iter))
        }
        Err(_utf8_err) => {
          let msg = String::from("Decode Error");
          let decode_error = vm.new_unicode_decode_error(msg);
          Err(decode_error)
        }
      }
    } else {
      Err(objiter::new_stop_iteration(vm))
    }
  }
}

fn build_reader(seq: PyObjectRef, vm: &VirtualMachine) -> PyResult {

  let bytes = match_class!(match seq {
    s @ PyString => {
      Vec::from(s.as_str().as_bytes())
    }
    b @ PyBytes => {
      Vec::from(b.get_value())
    }
    _obj => {
      let msg = String::from("argument 1 must be an iterator");
      return Err(vm.new_type_error(msg))
    }
  });

  let reader = Reader::new(bytes);
  Ok(reader.into_ref(vm).into_object())
}

fn csv_reader(expect_file: PyObjectRef, vm: &VirtualMachine) -> PyResult {
  
  // py_string or py_bytes
  let contents = {
    let args = vec![];
    vm.call_method(&expect_file, "read", args)
  }?;

  build_reader(contents, vm)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
  let ctx = &vm.ctx;

  let reader_type = Reader::make_class(ctx);

  py_module!(vm, "csv", {
    "reader" => ctx.new_rustfunc(csv_reader),
    "Reader" => reader_type,
  })
}
