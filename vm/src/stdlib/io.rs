/*
 * I/O core tools.
 */

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::path::PathBuf;

use num_bigint::ToBigInt;
use num_traits::ToPrimitive;

use super::os;
use crate::function::PyFuncArgs;
use crate::import;
use crate::obj::objbytearray::PyByteArray;
use crate::obj::objbytes;
use crate::obj::objint;
use crate::obj::objstr;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{BufferProtocol, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

fn compute_c_flag(mode: &str) -> u16 {
    match mode {
        "w" => 512,
        "x" => 512,
        "a" => 8,
        "+" => 2,
        _ => 0,
    }
}

#[derive(Debug)]
struct PyStringIO {
    data: RefCell<String>,
}

type PyStringIORef = PyRef<PyStringIO>;

impl PyValue for PyStringIO {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("io", "StringIO")
    }
}

impl PyStringIORef {
    fn write(self, data: objstr::PyStringRef, _vm: &VirtualMachine) {
        let data = data.value.clone();
        self.data.borrow_mut().push_str(&data);
    }

    fn getvalue(self, _vm: &VirtualMachine) -> String {
        self.data.borrow().clone()
    }
}

fn string_io_new(cls: PyClassRef, vm: &VirtualMachine) -> PyResult<PyStringIORef> {
    PyStringIO {
        data: RefCell::new(String::default()),
    }
    .into_ref_with_type(vm, cls)
}

fn bytes_io_init(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
    // TODO
    Ok(vm.get_none())
}

fn bytes_io_getvalue(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO
    Ok(vm.get_none())
}

fn io_base_cm_enter(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(instance, None)]);
    Ok(instance.clone())
}

fn io_base_cm_exit(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        // The context manager protocol requires these, but we don't use them
        required = [
            (_instance, None),
            (_exception_type, None),
            (_exception_value, None),
            (_traceback, None)
        ]
    );
    Ok(vm.get_none())
}

// TODO Check if closed, then if so raise ValueError
fn io_base_flush(_zelf: PyObjectRef, _vm: &VirtualMachine) {}

fn buffered_io_base_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(buffered, None), (raw, None)]);
    vm.set_attr(buffered, "raw", raw.clone())?;
    Ok(vm.get_none())
}

fn buffered_reader_read(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(buffered, None)]);
    let buff_size = 8 * 1024;
    let buffer = vm.ctx.new_bytearray(vec![0; buff_size]);

    //buffer method
    let mut result = vec![];
    let mut length = buff_size;

    let raw = vm.get_attribute(buffered.clone(), "raw").unwrap();

    //Iterates through the raw class, invoking the readinto method
    //to obtain buff_size many bytes. Exit when less than buff_size many
    //bytes are returned (when the end of the file is reached).
    while length == buff_size {
        vm.call_method(&raw, "readinto", vec![buffer.clone()])
            .map_err(|_| vm.new_value_error("IO Error".to_string()))?;

        //Copy bytes from the buffer vector into the results vector
        if let Some(bytes) = buffer.payload::<PyByteArray>() {
            result.extend_from_slice(&bytes.value.borrow());
        };

        let py_len = vm.call_method(&buffer, "__len__", PyFuncArgs::default())?;
        length = objint::get_value(&py_len).to_usize().unwrap();
    }

    Ok(vm.ctx.new_bytes(result))
}

fn file_io_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (name, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.str_type()))]
    );

    let rust_mode = mode.map_or("r".to_string(), |m| objstr::get_value(m));

    match compute_c_flag(&rust_mode).to_bigint() {
        Some(os_mode) => {
            let args = vec![name.clone(), vm.ctx.new_int(os_mode)];
            let file_no = os::os_open(vm, PyFuncArgs::new(args, vec![]))?;

            vm.set_attr(file_io, "name", name.clone())?;
            vm.set_attr(file_io, "fileno", file_no)?;
            vm.set_attr(file_io, "closefd", vm.new_bool(false))?;
            vm.set_attr(file_io, "closed", vm.new_bool(false))?;

            Ok(vm.get_none())
        }
        None => Err(vm.new_type_error(format!("invalid mode {}", rust_mode))),
    }
}

fn file_io_read(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(file_io, None)]);
    let py_name = vm.get_attribute(file_io.clone(), "name")?;
    let f = match File::open(objstr::get_value(&py_name)) {
        Ok(v) => Ok(v),
        Err(_) => Err(vm.new_type_error("Error opening file".to_string())),
    };

    let buffer = match f {
        Ok(v) => Ok(BufReader::new(v)),
        Err(_) => Err(vm.new_type_error("Error reading from file".to_string())),
    };

    let mut bytes = vec![];
    if let Ok(mut buff) = buffer {
        match buff.read_to_end(&mut bytes) {
            Ok(_) => {}
            Err(_) => return Err(vm.new_value_error("Error reading from Buffer".to_string())),
        }
    }

    Ok(vm.ctx.new_bytes(bytes))
}

fn file_io_readinto(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(file_io, None), (obj, None)]);

    if !obj.readonly() {
        return Ok(vm.new_type_error(
            "readinto() argument must be read-write bytes-like object".to_string(),
        ));
    }

    //extract length of buffer
    let py_length = vm.call_method(obj, "__len__", PyFuncArgs::default())?;
    let length = objint::get_value(&py_length).to_u64().unwrap();

    let file_no = vm.get_attribute(file_io.clone(), "fileno")?;
    let raw_fd = objint::get_value(&file_no).to_i64().unwrap();

    //extract unix file descriptor.
    let handle = os::rust_file(raw_fd);

    let mut f = handle.take(length);
    if let Some(bytes) = obj.payload::<PyByteArray>() {
        //TODO: Implement for MemoryView

        let mut value_mut = bytes.value.borrow_mut();
        value_mut.clear();
        match f.read_to_end(&mut value_mut) {
            Ok(_) => {}
            Err(_) => return Err(vm.new_value_error("Error reading from Take".to_string())),
        }
    };

    let updated = os::raw_file_number(f.into_inner());
    vm.set_attr(file_io, "fileno", vm.ctx.new_int(updated))?;
    Ok(vm.get_none())
}

fn file_io_write(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    let file_no = vm.get_attribute(file_io.clone(), "fileno")?;
    let raw_fd = objint::get_value(&file_no).to_i64().unwrap();

    //unsafe block - creates file handle from the UNIX file descriptor
    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles
    let mut handle = os::rust_file(raw_fd);

    match obj.payload::<PyByteArray>() {
        Some(bytes) => {
            let value_mut = bytes.value.borrow();
            match handle.write(&value_mut[..]) {
                Ok(len) => {
                    //reset raw fd on the FileIO object
                    let updated = os::raw_file_number(handle);
                    vm.set_attr(file_io, "fileno", vm.ctx.new_int(updated))?;

                    //return number of bytes written
                    Ok(vm.ctx.new_int(len))
                }
                Err(_) => Err(vm.new_value_error("Error Writing Bytes to Handle".to_string())),
            }
        }
        None => Err(vm.new_value_error("Expected Bytes Object".to_string())),
    }
}

fn buffered_writer_write(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(buffered, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    let raw = vm.get_attribute(buffered.clone(), "raw").unwrap();

    //This should be replaced with a more appropriate chunking implementation
    vm.call_method(&raw, "write", vec![obj.clone()])
}

fn text_io_wrapper_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(text_io_wrapper, None), (buffer, None)]
    );

    vm.set_attr(text_io_wrapper, "buffer", buffer.clone())?;
    Ok(vm.get_none())
}

fn text_io_base_read(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(text_io_base, None)]);

    let raw = vm.get_attribute(text_io_base.clone(), "buffer").unwrap();

    if let Ok(bytes) = vm.call_method(&raw, "read", PyFuncArgs::default()) {
        let value = objbytes::get_value(&bytes).to_vec();

        //format bytes into string
        let rust_string = String::from_utf8(value).unwrap();
        Ok(vm.ctx.new_str(rust_string))
    } else {
        Err(vm.new_value_error("Error unpacking Bytes".to_string()))
    }
}

pub fn io_open(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.str_type()))]
    );

    let module = import::import_module(vm, PathBuf::default(), "io").unwrap();

    //mode is optional: 'rt' is the default mode (open from reading text)
    let rust_mode = mode.map_or("rt".to_string(), |m| objstr::get_value(m));

    let mut raw_modes = HashSet::new();

    //add raw modes
    raw_modes.insert("a".to_string());
    raw_modes.insert("r".to_string());
    raw_modes.insert("x".to_string());
    raw_modes.insert("w".to_string());

    //This is not a terribly elegant way to separate the file mode from
    //the "type" flag - this should be improved. The intention here is to
    //match a valid flag for the file_io_init call:
    //https://docs.python.org/3/library/io.html#io.FileIO
    let modes: Vec<char> = rust_mode
        .chars()
        .filter(|a| raw_modes.contains(&a.to_string()))
        .collect();

    if modes.is_empty() || modes.len() > 1 {
        return Err(vm.new_value_error("Invalid Mode".to_string()));
    }

    //Class objects (potentially) consumed by io.open
    //RawIO: FileIO
    //Buffered: BufferedWriter, BufferedReader
    //Text: TextIOWrapper
    let file_io_class = vm.get_attribute(module.clone(), "FileIO").unwrap();
    let buffered_writer_class = vm.get_attribute(module.clone(), "BufferedWriter").unwrap();
    let buffered_reader_class = vm.get_attribute(module.clone(), "BufferedReader").unwrap();
    let text_io_wrapper_class = vm.get_attribute(module, "TextIOWrapper").unwrap();

    //Construct a FileIO (subclass of RawIOBase)
    //This is subsequently consumed by a Buffered Class.
    let file_args = vec![file.clone(), vm.ctx.new_str(modes[0].to_string())];
    let file_io = vm.invoke(file_io_class, file_args)?;

    //Create Buffered class to consume FileIO. The type of buffered class depends on
    //the operation in the mode.
    //There are 3 possible classes here, each inheriting from the RawBaseIO
    // creating || writing || appending => BufferedWriter
    let buffered = if rust_mode.contains('w') {
        vm.invoke(buffered_writer_class, vec![file_io.clone()])
    // reading => BufferedReader
    } else {
        vm.invoke(buffered_reader_class, vec![file_io.clone()])
        //TODO: updating => PyBufferedRandom
    };

    if rust_mode.contains('t') {
        //If the mode is text this buffer type is consumed on construction of
        //a TextIOWrapper which is subsequently returned.
        vm.invoke(text_io_wrapper_class, vec![buffered.unwrap()])
    } else {
        // If the mode is binary this Buffered class is returned directly at
        // this point.
        //For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field
        buffered
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    //IOBase the abstract base class of the IO Module
    let io_base = py_class!(ctx, "IOBase", ctx.object(), {
        "__enter__" => ctx.new_rustfunc(io_base_cm_enter),
        "__exit__" => ctx.new_rustfunc(io_base_cm_exit),
        "flush" => ctx.new_rustfunc(io_base_flush)
    });

    // IOBase Subclasses
    let raw_io_base = py_class!(ctx, "RawIOBase", ctx.object(), {});

    let buffered_io_base = py_class!(ctx, "BufferedIOBase", io_base.clone(), {
        "__init__" => ctx.new_rustfunc(buffered_io_base_init)
    });

    //TextIO Base has no public constructor
    let text_io_base = py_class!(ctx, "TextIOBase", io_base.clone(), {
        "read" => ctx.new_rustfunc(text_io_base_read)
    });

    // RawBaseIO Subclasses
    // TODO Fix name?
    let file_io = py_class!(ctx, "FileIO", raw_io_base.clone(), {
        "__init__" => ctx.new_rustfunc(file_io_init),
        "name" => ctx.str_type(),
        "read" => ctx.new_rustfunc(file_io_read),
        "readinto" => ctx.new_rustfunc(file_io_readinto),
        "write" => ctx.new_rustfunc(file_io_write)
    });

    // BufferedIOBase Subclasses
    let buffered_reader = py_class!(ctx, "BufferedReader", buffered_io_base.clone(), {
        "read" => ctx.new_rustfunc(buffered_reader_read)
    });

    let buffered_writer = py_class!(ctx, "BufferedWriter", buffered_io_base.clone(), {
        "write" => ctx.new_rustfunc(buffered_writer_write)
    });

    //TextIOBase Subclass
    let text_io_wrapper = py_class!(ctx, "TextIOWrapper", text_io_base.clone(), {
        "__init__" => ctx.new_rustfunc(text_io_wrapper_init)
    });

    //StringIO: in-memory text
    let string_io = py_class!(ctx, "StringIO", text_io_base.clone(), {
        "__new__" => ctx.new_rustfunc(string_io_new),
        "write" => ctx.new_rustfunc(PyStringIORef::write),
        "getvalue" => ctx.new_rustfunc(PyStringIORef::getvalue)
    });

    //BytesIO: in-memory bytes
    let bytes_io = py_class!(ctx, "BytesIO", buffered_io_base.clone(), {
        "__init__" => ctx.new_rustfunc(bytes_io_init),
        "getvalue" => ctx.new_rustfunc(bytes_io_getvalue)
    });

    py_module!(vm, "io", {
        "open" => ctx.new_rustfunc(io_open),
        "IOBase" => io_base,
        "RawIOBase" => raw_io_base,
        "BufferedIOBase" => buffered_io_base,
        "TextIOBase" => text_io_base,
        "FileIO" => file_io,
        "BufferedReader" => buffered_reader,
        "BufferedWriter" => buffered_writer,
        "TextIOWrapper" => text_io_wrapper,
        "StringIO" => string_io,
        "BytesIO" => bytes_io,
    })
}
