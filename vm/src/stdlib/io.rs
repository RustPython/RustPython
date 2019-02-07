/*
 * I/O core tools.
 */

//library imports
use std::collections::HashSet;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;

//3rd party imports
use num_bigint::ToBigInt;
use num_traits::ToPrimitive;

//custom imports
use super::super::obj::objbytes;
use super::super::obj::objint;
use super::super::obj::objstr;
use super::super::obj::objtype;
use super::os;

use super::super::pyobject::{
    AttributeProtocol, BufferProtocol, PyContext, PyFuncArgs, PyObjectPayload, PyObjectRef,
    PyResult, TypeProtocol,
};

use super::super::vm::VirtualMachine;

fn compute_c_flag(mode: &str) -> u16 {
    match mode {
        "w" => 512,
        "x" => 512,
        "a" => 8,
        "+" => 2,
        _ => 0,
    }
}

fn string_io_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    // arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    // TODO
    Ok(vm.get_none())
}

fn string_io_getvalue(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO
    Ok(vm.get_none())
}

fn bytes_io_init(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
    // TODO
    Ok(vm.get_none())
}

fn bytes_io_getvalue(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    // TODO
    Ok(vm.get_none())
}

fn buffered_io_base_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(buffered, None), (raw, None)]);
    vm.ctx.set_attr(&buffered, "raw", raw.clone());
    Ok(vm.get_none())
}

fn buffered_reader_read(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(buffered, None)]);
    let buff_size = 8 * 1024;
    let buffer = vm.ctx.new_bytearray(vec![0; buff_size]);

    //buffer method
    let mut result = vec![];
    let mut length = buff_size;

    let raw = vm.ctx.get_attr(&buffered, "raw").unwrap();

    //Iterates through the raw class, invoking the readinto method
    //to obtain buff_size many bytes. Exit when less than buff_size many
    //bytes are returned (when the end of the file is reached).
    while length == buff_size {
        let raw_read = vm.get_method(raw.clone(), &"readinto".to_string()).unwrap();
        vm.invoke(raw_read, PyFuncArgs::new(vec![buffer.clone()], vec![]))
            .map_err(|_| vm.new_value_error("IO Error".to_string()))?;

        //Copy bytes from the buffer vector into the results vector
        if let PyObjectPayload::Bytes { ref mut value } = buffer.borrow_mut().payload {
            result.extend(value.iter().cloned());
        };

        let len = vm.get_method(buffer.clone(), &"__len__".to_string());
        let py_len = vm.invoke(len.unwrap(), PyFuncArgs::default());
        length = objint::get_value(&py_len.unwrap()).to_usize().unwrap();
    }

    Ok(vm.ctx.new_bytes(result))
}

fn file_io_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
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

            vm.ctx.set_attr(&file_io, "name", name.clone());
            vm.ctx.set_attr(&file_io, "fileno", file_no);
            vm.ctx.set_attr(&file_io, "closefd", vm.new_bool(false));
            vm.ctx.set_attr(&file_io, "closed", vm.new_bool(false));

            Ok(vm.get_none())
        }
        None => Err(vm.new_type_error(format!("invalid mode {}", rust_mode))),
    }
}

fn file_io_read(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(file_io, None)]);
    let py_name = file_io.get_attr("name").unwrap();
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

fn file_io_readinto(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(file_io, None), (obj, None)]);

    if !obj.readonly() {
        return Ok(vm.new_type_error(
            "readinto() argument must be read-write bytes-like object".to_string(),
        ));
    }

    //extract length of buffer
    let len_method = vm.get_method(obj.clone(), &"__len__".to_string());
    let py_length = vm.invoke(len_method.unwrap(), PyFuncArgs::default());
    let length = objint::get_value(&py_length.unwrap()).to_u64().unwrap();

    let file_no = file_io.get_attr("fileno").unwrap();
    let raw_fd = objint::get_value(&file_no).to_i64().unwrap();

    //extract unix file descriptor.
    let handle = os::rust_file(raw_fd);

    let mut f = handle.take(length);
    if let PyObjectPayload::Bytes { ref mut value } = obj.borrow_mut().payload {
        //TODO: Implement for MemoryView

        value.clear();
        match f.read_to_end(&mut *value) {
            Ok(_) => {}
            Err(_) => return Err(vm.new_value_error("Error reading from Take".to_string())),
        }
    };

    let updated = os::raw_file_number(f.into_inner());
    vm.ctx.set_attr(&file_io, "fileno", vm.ctx.new_int(updated));
    Ok(vm.get_none())
}

fn file_io_write(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    let file_no = file_io.get_attr("fileno").unwrap();
    let raw_fd = objint::get_value(&file_no).to_i64().unwrap();

    //unsafe block - creates file handle from the UNIX file descriptor
    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles
    let mut handle = os::rust_file(raw_fd);

    match obj.borrow_mut().payload {
        PyObjectPayload::Bytes { ref mut value } => {
            match handle.write(&value[..]) {
                Ok(len) => {
                    //reset raw fd on the FileIO object
                    let updated = os::raw_file_number(handle);
                    vm.ctx.set_attr(&file_io, "fileno", vm.ctx.new_int(updated));

                    //return number of bytes written
                    Ok(vm.ctx.new_int(len))
                }
                Err(_) => Err(vm.new_value_error("Error Writing Bytes to Handle".to_string())),
            }
        }
        _ => Err(vm.new_value_error("Expected Bytes Object".to_string())),
    }
}

fn buffered_writer_write(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(buffered, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    let raw = vm.ctx.get_attr(&buffered, "raw").unwrap();
    let raw_write = vm.get_method(raw.clone(), &"write".to_string()).unwrap();

    //This should be replaced with a more appropriate chunking implementation
    vm.invoke(raw_write, PyFuncArgs::new(vec![obj.clone()], vec![]))
}

fn text_io_wrapper_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(text_io_wrapper, None), (buffer, None)]
    );

    vm.ctx.set_attr(&text_io_wrapper, "buffer", buffer.clone());
    Ok(vm.get_none())
}

fn text_io_base_read(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(text_io_base, None)]);

    let raw = vm.ctx.get_attr(&text_io_base, "buffer").unwrap();
    let read = vm.get_method(raw.clone(), &"read".to_string());

    if let Ok(bytes) = vm.invoke(read.unwrap(), PyFuncArgs::default()) {
        let value = objbytes::get_value(&bytes).to_vec();

        //format bytes into string
        let rust_string = String::from_utf8(value).unwrap();
        Ok(vm.ctx.new_str(rust_string))
    } else {
        Err(vm.new_value_error("Error unpacking Bytes".to_string()))
    }
}

pub fn io_open(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file, Some(vm.ctx.str_type()))],
        optional = [(mode, Some(vm.ctx.str_type()))]
    );

    let module = mk_module(&vm.ctx);

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
    let file_io_class = vm.ctx.get_attr(&module, "FileIO").unwrap();
    let buffered_writer_class = vm.ctx.get_attr(&module, "BufferedWriter").unwrap();
    let buffered_reader_class = vm.ctx.get_attr(&module, "BufferedReader").unwrap();
    let text_io_wrapper_class = vm.ctx.get_attr(&module, "TextIOWrapper").unwrap();

    //Construct a FileIO (subclass of RawIOBase)
    //This is subsequently consumed by a Buffered Class.
    let file_args = PyFuncArgs::new(
        vec![file.clone(), vm.ctx.new_str(modes[0].to_string())],
        vec![],
    );
    let file_io = vm.invoke(file_io_class, file_args)?;

    //Create Buffered class to consume FileIO. The type of buffered class depends on
    //the operation in the mode.
    //There are 3 possible classes here, each inheriting from the RawBaseIO
    // creating || writing || appending => BufferedWriter
    let buffered = if rust_mode.contains('w') {
        vm.invoke(
            buffered_writer_class,
            PyFuncArgs::new(vec![file_io.clone()], vec![]),
        )
    // reading => BufferedReader
    } else {
        vm.invoke(
            buffered_reader_class,
            PyFuncArgs::new(vec![file_io.clone()], vec![]),
        )
        //TODO: updating => PyBufferedRandom
    };

    if rust_mode.contains('t') {
        //If the mode is text this buffer type is consumed on construction of
        //a TextIOWrapper which is subsequently returned.
        vm.invoke(
            text_io_wrapper_class,
            PyFuncArgs::new(vec![buffered.unwrap()], vec![]),
        )
    } else {
        // If the mode is binary this Buffered class is returned directly at
        // this point.
        //For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field
        buffered
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module(&"io".to_string(), ctx.new_scope(None));

    ctx.set_attr(&py_mod, "open", ctx.new_rustfunc(io_open));

    //IOBase the abstract base class of the IO Module
    let io_base = ctx.new_class("IOBase", ctx.object());
    ctx.set_attr(&py_mod, "IOBase", io_base.clone());

    // IOBase Subclasses
    let raw_io_base = ctx.new_class("RawIOBase", ctx.object());
    ctx.set_attr(&py_mod, "RawIOBase", raw_io_base.clone());

    let buffered_io_base = {
        let buffered_io_base = ctx.new_class("BufferedIOBase", io_base.clone());
        ctx.set_attr(
            &buffered_io_base,
            "__init__",
            ctx.new_rustfunc(buffered_io_base_init),
        );
        buffered_io_base
    };
    ctx.set_attr(&py_mod, "BufferedIOBase", buffered_io_base.clone());

    //TextIO Base has no public constructor
    let text_io_base = {
        let text_io_base = ctx.new_class("TextIOBase", io_base.clone());
        ctx.set_attr(&text_io_base, "read", ctx.new_rustfunc(text_io_base_read));
        text_io_base
    };
    ctx.set_attr(&py_mod, "TextIOBase", text_io_base.clone());

    // RawBaseIO Subclasses
    let file_io = {
        let file_io = ctx.new_class("FileIO", raw_io_base.clone());
        ctx.set_attr(&file_io, "__init__", ctx.new_rustfunc(file_io_init));
        ctx.set_attr(&file_io, "name", ctx.str_type());
        ctx.set_attr(&file_io, "read", ctx.new_rustfunc(file_io_read));
        ctx.set_attr(&file_io, "readinto", ctx.new_rustfunc(file_io_readinto));
        ctx.set_attr(&file_io, "write", ctx.new_rustfunc(file_io_write));
        file_io
    };
    ctx.set_attr(&py_mod, "FileIO", file_io.clone());

    // BufferedIOBase Subclasses
    let buffered_reader = {
        let buffered_reader = ctx.new_class("BufferedReader", buffered_io_base.clone());
        ctx.set_attr(
            &buffered_reader,
            "read",
            ctx.new_rustfunc(buffered_reader_read),
        );
        buffered_reader
    };
    ctx.set_attr(&py_mod, "BufferedReader", buffered_reader.clone());

    let buffered_writer = {
        let buffered_writer = ctx.new_class("BufferedWriter", buffered_io_base.clone());
        ctx.set_attr(
            &buffered_writer,
            "write",
            ctx.new_rustfunc(buffered_writer_write),
        );
        buffered_writer
    };
    ctx.set_attr(&py_mod, "BufferedWriter", buffered_writer.clone());

    //TextIOBase Subclass
    let text_io_wrapper = {
        let text_io_wrapper = ctx.new_class("TextIOWrapper", text_io_base.clone());
        ctx.set_attr(
            &text_io_wrapper,
            "__init__",
            ctx.new_rustfunc(text_io_wrapper_init),
        );
        text_io_wrapper
    };
    ctx.set_attr(&py_mod, "TextIOWrapper", text_io_wrapper.clone());

    // BytesIO: in-memory bytes
    let string_io = {
        let string_io = ctx.new_class("StringIO", io_base.clone());
        ctx.set_attr(&string_io, "__init__", ctx.new_rustfunc(string_io_init));
        ctx.set_attr(&string_io, "getvalue", ctx.new_rustfunc(string_io_getvalue));
        string_io
    };
    ctx.set_attr(&py_mod, "StringIO", string_io);

    // StringIO: in-memory text
    let bytes_io = {
        let bytes_io = ctx.new_class("BytesIO", io_base.clone());
        ctx.set_attr(&bytes_io, "__init__", ctx.new_rustfunc(bytes_io_init));
        ctx.set_attr(&bytes_io, "getvalue", ctx.new_rustfunc(bytes_io_getvalue));
        bytes_io
    };
    ctx.set_attr(&py_mod, "BytesIO", bytes_io);

    py_mod
}
