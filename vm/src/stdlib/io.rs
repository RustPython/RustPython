/*
 * I/O core tools.
 */


use std::fs::File;
use std::io::BufReader;

use std::os::unix::io::{FromRawFd,IntoRawFd};

use std::io::prelude::*;

use super::super::obj::objstr;
use super::super::obj::objint;
use num_bigint::{ToBigInt};

use super::super::obj::objtype;
use super::os::os_open;

use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObjectKind, PyObjectRef, PyResult, TypeProtocol, AttributeProtocol
};

use super::super::vm::VirtualMachine;

use num_traits::ToPrimitive;

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
    arg_check!(
        vm,
        args,
        required = [(buffered, None), (raw, None)]
    );
    vm.ctx.set_attr(&buffered, "raw", raw.clone());
    Ok(vm.get_none())
}

fn buffered_io_base_read(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(buffered, None)]
    );
    let buff_size = 8;
    let mut buffer = vm.ctx.new_bytes(vec![0; buff_size]);

    //buffer method
    let mut result = vec![];
    let mut length = buff_size;

    let raw = vm.ctx.get_attr(&buffered, "raw").unwrap();

    while length == buff_size {
        let raw_read = vm.get_method(raw.clone(), &"readinto".to_string()).unwrap();
        vm.invoke(raw_read, PyFuncArgs::new(vec![buffer.clone()], vec![]));


        match buffer.borrow_mut().kind {
            PyObjectKind::Bytes { ref mut value } => {
                result.extend(value.iter().cloned());
            },
            _ => {}
        };
        
        let len = vm.get_method(buffer.clone(), &"__len__".to_string());
        let py_len  = vm.invoke(len.unwrap(), PyFuncArgs::default());
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

    let mode = if let Some(m) = mode {
        objstr::get_value(m)
    } else {
        "r".to_string()
    };

    let os_mode = match mode.as_ref() {
        "r" => 0.to_bigint(),
        _ => 1.to_bigint()
    };
    let args = vec![name.clone(), vm.ctx.new_int(os_mode.unwrap())];
    let fileno = os_open(vm, PyFuncArgs::new(args, vec![]));


    vm.ctx.set_attr(&file_io, "name", name.clone());
    vm.ctx.set_attr(&file_io, "fileno", fileno.unwrap());
    Ok(vm.get_none())
}

fn file_io_read(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None)]
    );
    let py_name = file_io.get_attr("name").unwrap();
    let f = match File::open(objstr::get_value(& py_name)) {
        Ok(v) => Ok(v),
        Err(_) => Err(vm.new_type_error("Error opening file".to_string())),
    }; 

    let buffer = match f {
        Ok(v) =>  Ok(BufReader::new(v)),
        Err(_) => Err(vm.new_type_error("Error reading from file".to_string()))
    };

    let mut bytes = vec![];
    if let Ok(mut buff) = buffer {
        buff.read_to_end(&mut bytes);
    }    

    Ok(vm.ctx.new_bytes(bytes))
}


fn file_io_readinto(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    //extract length of buffer
    let len_method = vm.get_method(obj.clone(), &"__len__".to_string());
    let py_length = vm.invoke(len_method.unwrap(), PyFuncArgs::default());
    let length = objint::get_value(&py_length.unwrap()).to_u64().unwrap();

    let fileno = file_io.get_attr("fileno").unwrap();
    let raw_fd = objint::get_value(&fileno).to_i32().unwrap();

    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles 
   let handle = unsafe {
        File::from_raw_fd(raw_fd)
   };

   let mut f = handle.take(length);
    match obj.borrow_mut().kind {
        PyObjectKind::Bytes { ref mut value } => {
            value.clear();
            f.read_to_end(&mut *value);

        },
        _ => {}
    };


    let new_handle = f.into_inner().into_raw_fd().to_bigint();
    vm.ctx.set_attr(&file_io, "fileno", vm.ctx.new_int(new_handle.unwrap()));
    Ok(vm.get_none())
}

fn file_io_write(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(file_io, None), (obj, Some(vm.ctx.bytes_type()))]
    );

    let fileno = file_io.get_attr("fileno").unwrap();
    let raw_fd = objint::get_value(&fileno).to_i32().unwrap();

    //raw_fd is supported on UNIX only. This will need to be extended
    //to support windows - i.e. raw file_handles 
   let mut handle = unsafe {
        File::from_raw_fd(raw_fd)
   };

    match obj.borrow_mut().kind {
        PyObjectKind::Bytes { ref mut value } => {
            match handle.write(&value[..]) {
                Ok(k) => { println!("{}", k); },
                Err(_) => {}
            }
        },
        _ => {}
    };

    let len_method = vm.get_method(obj.clone(), &"__len__".to_string());
    vm.invoke(len_method.unwrap(), PyFuncArgs::default())

    //TODO: reset fileno

}

fn buffered_reader_init(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm, 
        args, 
        required = [(buffed_reader, None), (raw, None)]
    );


    //simple calls read on the read class!
    // TODO
    Ok(vm.get_none())
}

fn io_open(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm, 
        args, 
        required = [(file, Some(vm.ctx.str_type())), (mode, Some(vm.ctx.str_type()))]
    );

    let module = mk_module(&vm.ctx);

    let rust_mode = objstr::get_value(mode);
    if rust_mode.contains("w") {
        vm.new_not_implemented_error("Writes are not yet implemented".to_string());
    }

    let file_io_class = vm.ctx.get_attr(&module, "FileIO").unwrap();
    vm.invoke(file_io_class, PyFuncArgs::new(vec![file.clone()], vec![]))



    // vm.get_method(fi.clone(), &"__new__".to_string());

    // let buffer = vm.ctx.new_bytearray(vec![]);
    // vm.invoke(new_file_io.unwrap(), PyFuncArgs {
    //     args: vec![fi.clone(), file.clone()],
    //     kwargs: vec![]
    // });
    // Ok(fi)

    //mode is optional: 'rt' is the default mode (open from reading text)
    //To start we construct a FileIO (subclass of RawIOBase)
    //This is subsequently consumed by a Buffered_class of type depending
    //operation in the mode. i.e:
    // updating => PyBufferedRandom
    // creating || writing || appending => BufferedWriter
    // reading => BufferedReader
    // If the mode is binary this Buffered class is returned directly at
    // this point.
    //For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field

    //If the mode is text this buffer type is consumed on construction of 
    //a TextIOWrapper which is subsequently returned.
    // Ok(vm.get_none())
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

    let buffered_io_base = ctx.new_class("BufferedIOBase", io_base.clone());
    ctx.set_attr(&buffered_io_base, "__init__", ctx.new_rustfunc(buffered_io_base_init));
    ctx.set_attr(&buffered_io_base, "read", ctx.new_rustfunc(buffered_io_base_read));
    ctx.set_attr(&py_mod, "BufferedIOBase", buffered_io_base.clone());

    let text_io_base = ctx.new_class("TextIOBase", io_base.clone());
    ctx.set_attr(&py_mod, "TextIOBase", text_io_base.clone());

    // RawBaseIO Subclasses
    let file_io = ctx.new_class("FileIO", raw_io_base.clone());
    ctx.set_attr(&file_io, "__init__", ctx.new_rustfunc(file_io_init));
    ctx.set_attr(&file_io, "name", ctx.str_type());
    ctx.set_attr(&file_io, "read", ctx.new_rustfunc(file_io_read));
    ctx.set_attr(&file_io, "readinto", ctx.new_rustfunc(file_io_readinto));
    ctx.set_attr(&file_io, "write", ctx.new_rustfunc(file_io_write));
    ctx.set_attr(&py_mod, "FileIO", file_io.clone());

    // BufferedIOBase Subclasses
    let buffered_reader = ctx.new_class("BufferedReader", buffered_io_base.clone());
    ctx.set_attr(&py_mod, "BufferedReader", buffered_reader.clone());

    let buffered_reader = ctx.new_class("BufferedWriter", buffered_io_base.clone());
    ctx.set_attr(&py_mod, "BufferedWriter", buffered_reader.clone());

    //TextIOBase Subclass
    let text_io_wrapper = ctx.new_class("TextIOWrapper", ctx.object());
    ctx.set_attr(&py_mod, "TextIOWrapper", text_io_wrapper.clone());

    // BytesIO: in-memory bytes
    let string_io = ctx.new_class("StringIO", io_base.clone());
    ctx.set_attr(&string_io, "__init__", ctx.new_rustfunc(string_io_init));
    ctx.set_attr(&string_io, "getvalue", ctx.new_rustfunc(string_io_getvalue));
    ctx.set_attr(&py_mod, "StringIO", string_io);

    // StringIO: in-memory text
    let bytes_io = ctx.new_class("BytesIO", io_base.clone());
    ctx.set_attr(&bytes_io, "__init__", ctx.new_rustfunc(bytes_io_init));
    ctx.set_attr(&bytes_io, "getvalue", ctx.new_rustfunc(bytes_io_getvalue));
    ctx.set_attr(&py_mod, "BytesIO", bytes_io);

    py_mod
}