/*
 * I/O core tools.
 */
use std::cell::RefCell;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::io::Cursor;
use std::io::SeekFrom;

use std::path::PathBuf;

use num_bigint::ToBigInt;
use num_traits::ToPrimitive;

use super::os;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::import;
use crate::obj::objbytearray::PyByteArray;
use crate::obj::objbytes;
use crate::obj::objint;
use crate::obj::objstr;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{BufferProtocol, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

fn byte_count(bytes: OptionalArg<Option<PyObjectRef>>) -> i64 {
    match bytes {
        OptionalArg::Present(Some(ref int)) => objint::get_value(int).to_i64().unwrap(),
        _ => (-1 as i64),
    }
}

#[derive(Debug)]
struct BufferedIO {
    cursor: Cursor<Vec<u8>>,
}

impl BufferedIO {
    fn new(cursor: Cursor<Vec<u8>>) -> BufferedIO {
        BufferedIO { cursor: cursor }
    }

    fn write(&mut self, data: Vec<u8>) -> Option<u64> {
        let length = data.len();

        match self.cursor.write_all(&data) {
            Ok(_) => Some(length as u64),
            Err(_) => None,
        }
    }

    //return the entire contents of the underlying
    fn getvalue(&self) -> Vec<u8> {
        self.cursor.clone().into_inner()
    }

    //skip to the jth position
    fn seek(&mut self, offset: u64) -> Option<u64> {
        match self.cursor.seek(SeekFrom::Start(offset.clone())) {
            Ok(_) => Some(offset),
            Err(_) => None,
        }
    }

    //Read k bytes from the object and return.
    fn read(&mut self, bytes: i64) -> Option<Vec<u8>> {
        let mut buffer = Vec::new();

        //for a defined number of bytes, i.e. bytes != -1
        if bytes > 0 {
            let mut handle = self.cursor.clone().take(bytes as u64);
            //read handle into buffer
            if let Err(_) = handle.read_to_end(&mut buffer) {
                return None;
            }
            //the take above consumes the struct value
            //we add this back in with the takes into_inner method
            self.cursor = handle.into_inner();
        } else {
            //read handle into buffer
            if let Err(_) = self.cursor.read_to_end(&mut buffer) {
                return None;
            }
        };

        Some(buffer)
    }
}

#[derive(Debug)]
struct PyStringIO {
    buffer: RefCell<BufferedIO>,
}

type PyStringIORef = PyRef<PyStringIO>;

impl PyValue for PyStringIO {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("io", "StringIO")
    }
}

impl PyStringIORef {
    //write string to underlying vector
    fn write(self, data: objstr::PyStringRef, vm: &VirtualMachine) -> PyResult {
        let bytes = &data.value.clone().into_bytes();

        match self.buffer.borrow_mut().write(bytes.to_vec()) {
            Some(value) => Ok(vm.ctx.new_int(value)),
            None => Err(vm.new_type_error("Error Writing String".to_string())),
        }
    }

    //return the entire contents of the underlying
    fn getvalue(self, vm: &VirtualMachine) -> PyResult {
        match String::from_utf8(self.buffer.borrow().getvalue()) {
            Ok(result) => Ok(vm.ctx.new_str(result)),
            Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_string())),
        }
    }

    //skip to the jth position
    fn seek(self, offset: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let position = objint::get_value(&offset).to_u64().unwrap();
        match self.buffer.borrow_mut().seek(position) {
            Some(value) => Ok(vm.ctx.new_int(value)),
            None => Err(vm.new_value_error("Error Performing Operation".to_string())),
        }
    }

    //Read k bytes from the object and return.
    //If k is undefined || k == -1, then we read all bytes until the end of the file.
    //This also increments the stream position by the value of k
    fn read(self, bytes: OptionalArg<Option<PyObjectRef>>, vm: &VirtualMachine) -> PyResult {
        let data = match self.buffer.borrow_mut().read(byte_count(bytes)) {
            Some(value) => value,
            None => Vec::new(),
        };

        match String::from_utf8(data) {
            Ok(value) => Ok(vm.ctx.new_str(value)),
            Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_string())),
        }
    }
}

fn string_io_new(
    cls: PyClassRef,
    object: OptionalArg<Option<PyObjectRef>>,
    vm: &VirtualMachine,
) -> PyResult<PyStringIORef> {
    let raw_string = match object {
        OptionalArg::Present(Some(ref input)) => objstr::get_value(input),
        _ => String::new(),
    };

    PyStringIO {
        buffer: RefCell::new(BufferedIO::new(Cursor::new(raw_string.into_bytes()))),
    }
    .into_ref_with_type(vm, cls)
}

#[derive(Debug)]
struct PyBytesIO {
    buffer: RefCell<BufferedIO>,
}

type PyBytesIORef = PyRef<PyBytesIO>;

impl PyValue for PyBytesIO {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("io", "BytesIO")
    }
}

impl PyBytesIORef {
    fn write(self, data: objbytes::PyBytesRef, vm: &VirtualMachine) -> PyResult {
        let bytes = data.get_value();

        match self.buffer.borrow_mut().write(bytes.to_vec()) {
            Some(value) => Ok(vm.ctx.new_int(value)),
            None => Err(vm.new_type_error("Error Writing Bytes".to_string())),
        }
    }
    //Retrieves the entire bytes object value from the underlying buffer
    fn getvalue(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.buffer.borrow().getvalue()))
    }

    //Takes an integer k (bytes) and returns them from the underlying buffer
    //If k is undefined || k == -1, then we read all bytes until the end of the file.
    //This also increments the stream position by the value of k
    fn read(self, bytes: OptionalArg<Option<PyObjectRef>>, vm: &VirtualMachine) -> PyResult {
        match self.buffer.borrow_mut().read(byte_count(bytes)) {
            Some(value) => Ok(vm.ctx.new_bytes(value)),
            None => Err(vm.new_value_error("Error Retrieving Value".to_string())),
        }
    }

    //skip to the jth position
    fn seek(self, offset: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let position = objint::get_value(&offset).to_u64().unwrap();
        match self.buffer.borrow_mut().seek(position) {
            Some(value) => Ok(vm.ctx.new_int(value)),
            None => Err(vm.new_value_error("Error Performing Operation".to_string())),
        }
    }
}

fn bytes_io_new(
    cls: PyClassRef,
    object: OptionalArg<Option<PyObjectRef>>,
    vm: &VirtualMachine,
) -> PyResult<PyBytesIORef> {
    let raw_bytes = match object {
        OptionalArg::Present(Some(ref input)) => objbytes::get_value(input).to_vec(),
        _ => vec![],
    };

    PyBytesIO {
        buffer: RefCell::new(BufferedIO::new(Cursor::new(raw_bytes))),
    }
    .into_ref_with_type(vm, cls)
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
            result.extend_from_slice(&bytes.inner.borrow().elements);
        };

        let py_len = vm.call_method(&buffer, "__len__", PyFuncArgs::default())?;
        length = objint::get_value(&py_len).to_usize().unwrap();
    }

    Ok(vm.ctx.new_bytes(result))
}

fn compute_c_flag(mode: &str) -> u32 {
    let flags = match mode.chars().next() {
        Some(mode) => match mode {
            'w' => os::FileCreationFlags::O_WRONLY | os::FileCreationFlags::O_CREAT,
            'x' => {
                os::FileCreationFlags::O_WRONLY
                    | os::FileCreationFlags::O_CREAT
                    | os::FileCreationFlags::O_EXCL
            }
            'a' => os::FileCreationFlags::O_APPEND,
            '+' => os::FileCreationFlags::O_RDWR,
            _ => os::FileCreationFlags::O_RDONLY,
        },
        None => os::FileCreationFlags::O_RDONLY,
    };
    flags.bits()
}

fn file_io_init(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (name, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.str_type()))]
    );

    let rust_mode = mode.map_or("r".to_string(), objstr::get_value);

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

        let value_mut = &mut bytes.inner.borrow_mut().elements;
        value_mut.clear();
        match f.read_to_end(value_mut) {
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
            let value_mut = &mut bytes.inner.borrow_mut().elements;
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

fn split_mode_string(mode_string: String) -> Result<(String, String), String> {
    let mut mode: char = '\0';
    let mut typ: char = '\0';
    let mut plus_is_set = false;

    for ch in mode_string.chars() {
        match ch {
            '+' => {
                if plus_is_set {
                    return Err(format!("invalid mode: '{}'", mode_string));
                }
                plus_is_set = true;
            }
            't' | 'b' => {
                if typ != '\0' {
                    if typ == ch {
                        // no duplicates allowed
                        return Err(format!("invalid mode: '{}'", mode_string));
                    } else {
                        return Err("can't have text and binary mode at once".to_string());
                    }
                }
                typ = ch;
            }
            'a' | 'r' | 'w' => {
                if mode != '\0' {
                    if mode == ch {
                        // no duplicates allowed
                        return Err(format!("invalid mode: '{}'", mode_string));
                    } else {
                        return Err(
                            "must have exactly one of create/read/write/append mode".to_string()
                        );
                    }
                }
                mode = ch;
            }
            _ => return Err(format!("invalid mode: '{}'", mode_string)),
        }
    }

    if mode == '\0' {
        return Err(
            "Must have exactly one of create/read/write/append mode and at most one plus"
                .to_string(),
        );
    }
    let mut mode = mode.to_string();
    if plus_is_set {
        mode.push('+');
    }
    if typ == '\0' {
        typ = 't';
    }
    Ok((mode, typ.to_string()))
}

pub fn io_open(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.str_type()))]
    );

    // mode is optional: 'rt' is the default mode (open from reading text)
    let mode_string = mode.map_or("rt".to_string(), objstr::get_value);

    let (mode, typ) = match split_mode_string(mode_string) {
        Ok((mode, typ)) => (mode, typ),
        Err(error_message) => {
            return Err(vm.new_value_error(error_message));
        }
    };

    let io_module = import::import_module(vm, PathBuf::default(), "io").unwrap();

    // Construct a FileIO (subclass of RawIOBase)
    // This is subsequently consumed by a Buffered Class.
    let file_io_class = vm.get_attribute(io_module.clone(), "FileIO").unwrap();
    let file_io_obj = vm.invoke(
        file_io_class,
        vec![file.clone(), vm.ctx.new_str(mode.clone())],
    )?;

    // Create Buffered class to consume FileIO. The type of buffered class depends on
    // the operation in the mode.
    // There are 3 possible classes here, each inheriting from the RawBaseIO
    // creating || writing || appending => BufferedWriter
    let buffered = match mode.chars().next().unwrap() {
        'w' => {
            let buffered_writer_class = vm
                .get_attribute(io_module.clone(), "BufferedWriter")
                .unwrap();
            vm.invoke(buffered_writer_class, vec![file_io_obj.clone()])
        }
        'r' => {
            let buffered_reader_class = vm
                .get_attribute(io_module.clone(), "BufferedReader")
                .unwrap();
            vm.invoke(buffered_reader_class, vec![file_io_obj.clone()])
        }
        //TODO: updating => PyBufferedRandom
        _ => unimplemented!("'a' mode is not yet implemented"),
    };

    let io_obj = match typ.chars().next().unwrap() {
        // If the mode is text this buffer type is consumed on construction of
        // a TextIOWrapper which is subsequently returned.
        't' => {
            let text_io_wrapper_class = vm.get_attribute(io_module, "TextIOWrapper").unwrap();
            vm.invoke(text_io_wrapper_class, vec![buffered.unwrap()])
        }
        // If the mode is binary this Buffered class is returned directly at
        // this point.
        // For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field
        'b' => buffered,
        _ => unreachable!(),
    };
    io_obj
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
    let raw_io_base = py_class!(ctx, "RawIOBase", io_base.clone(), {});

    let buffered_io_base = py_class!(ctx, "BufferedIOBase", io_base.clone(), {});

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
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547
        "__init__" => ctx.new_rustfunc(buffered_io_base_init),
        "read" => ctx.new_rustfunc(buffered_reader_read)
    });

    let buffered_writer = py_class!(ctx, "BufferedWriter", buffered_io_base.clone(), {
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547
        "__init__" => ctx.new_rustfunc(buffered_io_base_init),
        "write" => ctx.new_rustfunc(buffered_writer_write)
    });

    //TextIOBase Subclass
    let text_io_wrapper = py_class!(ctx, "TextIOWrapper", text_io_base.clone(), {
        "__init__" => ctx.new_rustfunc(text_io_wrapper_init)
    });

    //StringIO: in-memory text
    let string_io = py_class!(ctx, "StringIO", text_io_base.clone(), {
        "__new__" => ctx.new_rustfunc(string_io_new),
        "seek" => ctx.new_rustfunc(PyStringIORef::seek),
        "read" => ctx.new_rustfunc(PyStringIORef::read),
        "write" => ctx.new_rustfunc(PyStringIORef::write),
        "getvalue" => ctx.new_rustfunc(PyStringIORef::getvalue)
    });

    //BytesIO: in-memory bytes
    let bytes_io = py_class!(ctx, "BytesIO", buffered_io_base.clone(), {
        "__new__" => ctx.new_rustfunc(bytes_io_new),
        "read" => ctx.new_rustfunc(PyBytesIORef::read),
        "read1" => ctx.new_rustfunc(PyBytesIORef::read),
        "seek" => ctx.new_rustfunc(PyBytesIORef::seek),
        "write" => ctx.new_rustfunc(PyBytesIORef::write),
        "getvalue" => ctx.new_rustfunc(PyBytesIORef::getvalue)
    });

    py_module!(vm, "_io", {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_mode_split_into(mode_string: &str, expected_mode: &str, expected_typ: &str) {
        let (mode, typ) = split_mode_string(mode_string.to_string()).unwrap();
        assert_eq!(mode, expected_mode);
        assert_eq!(typ, expected_typ);
    }

    #[test]
    fn test_split_mode_valid_cases() {
        assert_mode_split_into("r", "r", "t");
        assert_mode_split_into("rb", "r", "b");
        assert_mode_split_into("rt", "r", "t");
        assert_mode_split_into("r+t", "r+", "t");
        assert_mode_split_into("w+t", "w+", "t");
        assert_mode_split_into("r+b", "r+", "b");
        assert_mode_split_into("w+b", "w+", "b");
    }

    #[test]
    fn test_invalid_mode() {
        assert_eq!(
            split_mode_string("rbsss".to_string()),
            Err("invalid mode: 'rbsss'".to_string())
        );
        assert_eq!(
            split_mode_string("rrb".to_string()),
            Err("invalid mode: 'rrb'".to_string())
        );
        assert_eq!(
            split_mode_string("rbb".to_string()),
            Err("invalid mode: 'rbb'".to_string())
        );
    }

    #[test]
    fn test_mode_not_specified() {
        assert_eq!(
            split_mode_string("".to_string()),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_string()
            )
        );
        assert_eq!(
            split_mode_string("b".to_string()),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_string()
            )
        );
        assert_eq!(
            split_mode_string("t".to_string()),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_text_and_binary_at_once() {
        assert_eq!(
            split_mode_string("rbt".to_string()),
            Err("can't have text and binary mode at once".to_string())
        );
    }

    #[test]
    fn test_exactly_one_mode() {
        assert_eq!(
            split_mode_string("rwb".to_string()),
            Err("must have exactly one of create/read/write/append mode".to_string())
        );
    }

    #[test]
    fn test_at_most_one_plus() {
        assert_eq!(
            split_mode_string("a++".to_string()),
            Err("invalid mode: 'a++'".to_string())
        );
    }

    #[test]
    fn test_buffered_read() {
        let data = vec![1, 2, 3, 4];
        let bytes: i64 = -1;
        let mut buffered = BufferedIO {
            cursor: Cursor::new(data.clone()),
        };

        assert_eq!(buffered.read(bytes).unwrap(), data);
    }

    #[test]
    fn test_buffered_seek() {
        let data = vec![1, 2, 3, 4];
        let count: u64 = 2;
        let mut buffered = BufferedIO {
            cursor: Cursor::new(data.clone()),
        };

        assert_eq!(buffered.seek(count.clone()).unwrap(), count);
        assert_eq!(buffered.read(count.clone() as i64).unwrap(), vec![3, 4]);
    }

    #[test]
    fn test_buffered_value() {
        let data = vec![1, 2, 3, 4];
        let buffered = BufferedIO {
            cursor: Cursor::new(data.clone()),
        };

        assert_eq!(buffered.getvalue(), data);
    }
}
