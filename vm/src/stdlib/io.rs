/*
 * I/O core tools.
 */
use std::fs;
use std::io::{self, prelude::*, Cursor, SeekFrom};

use bstr::ByteSlice;
use crossbeam_utils::atomic::AtomicCell;
use num_traits::ToPrimitive;

use crate::byteslike::PyBytesLike;
use crate::common::cell::{PyRwLock, PyRwLockWriteGuard};
use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
use crate::function::{Args, KwArgs, OptionalArg, OptionalOption, PyFuncArgs};
use crate::obj::objbool;
use crate::obj::objbytearray::PyByteArray;
use crate::obj::objbytes::PyBytesRef;
use crate::obj::objint;
use crate::obj::objiter;
use crate::obj::objstr::{self, PyString, PyStringRef};
use crate::obj::objtype::{self, PyClassRef};
use crate::pyobject::{
    BorrowValue, BufferProtocol, Either, IntoPyObject, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject,
};
use crate::vm::VirtualMachine;

#[derive(FromArgs)]
struct OptionalSize {
    // In a few functions, the default value is -1 rather than None.
    // Make sure the default value doesn't affect compatibility.
    #[pyarg(positional_only, default = "None")]
    size: Option<isize>,
}

impl OptionalSize {
    fn to_usize(self) -> Option<usize> {
        self.size.and_then(|v| v.to_usize())
    }

    fn try_usize(self, vm: &VirtualMachine) -> PyResult<Option<usize>> {
        self.size
            .map(|v| {
                if v >= 0 {
                    Ok(v as usize)
                } else {
                    Err(vm.new_value_error(format!("Negative size value {}", v)))
                }
            })
            .transpose()
    }
}

fn os_err(vm: &VirtualMachine, err: io::Error) -> PyBaseExceptionRef {
    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    {
        err.into_pyexception(vm)
    }
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    {
        vm.new_os_error(err.to_string())
    }
}

const DEFAULT_BUFFER_SIZE: usize = 8 * 1024;

fn seekfrom(vm: &VirtualMachine, offset: PyObjectRef, how: OptionalArg<i32>) -> PyResult<SeekFrom> {
    let seek = match how {
        OptionalArg::Present(0) | OptionalArg::Missing => {
            SeekFrom::Start(u64::try_from_object(vm, offset)?)
        }
        OptionalArg::Present(1) => SeekFrom::Current(i64::try_from_object(vm, offset)?),
        OptionalArg::Present(2) => SeekFrom::End(i64::try_from_object(vm, offset)?),
        _ => return Err(vm.new_value_error("invalid value for how".to_owned())),
    };
    Ok(seek)
}

#[derive(Debug)]
struct BufferedIO {
    cursor: Cursor<Vec<u8>>,
}

impl BufferedIO {
    fn new(cursor: Cursor<Vec<u8>>) -> BufferedIO {
        BufferedIO { cursor }
    }

    fn write(&mut self, data: &[u8]) -> Option<u64> {
        let length = data.len();

        match self.cursor.write_all(data) {
            Ok(_) => Some(length as u64),
            Err(_) => None,
        }
    }

    //return the entire contents of the underlying
    fn getvalue(&self) -> Vec<u8> {
        self.cursor.clone().into_inner()
    }

    //skip to the jth position
    fn seek(&mut self, seek: SeekFrom) -> io::Result<u64> {
        self.cursor.seek(seek)
    }

    //Read k bytes from the object and return.
    fn read(&mut self, bytes: Option<usize>) -> Option<Vec<u8>> {
        //for a defined number of bytes, i.e. bytes != -1
        match bytes {
            Some(bytes) => {
                let mut buffer = unsafe {
                    // Do not move or edit any part of this block without a safety validation.
                    // `set_len` is guaranteed to be safe only when the new length is less than or equal to the capacity
                    let mut buffer = Vec::with_capacity(bytes);
                    buffer.set_len(bytes);
                    buffer
                };
                //read handle into buffer
                self.cursor
                    .read_exact(&mut buffer)
                    .map_or(None, |_| Some(buffer))
            }
            None => {
                let mut buffer = Vec::new();
                //read handle into buffer
                if self.cursor.read_to_end(&mut buffer).is_err() {
                    None
                } else {
                    Some(buffer)
                }
            }
        }
    }

    fn tell(&self) -> u64 {
        self.cursor.position()
    }

    fn readline(&mut self, size: Option<usize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        let size = match size {
            None => {
                let mut buf = String::new();
                self.cursor
                    .read_line(&mut buf)
                    .map_err(|err| os_err(vm, err))?;
                return Ok(buf.into_bytes());
            }
            Some(0) => {
                return Ok(Vec::new());
            }
            Some(size) => size,
        };

        let available = {
            // For Cursor, fill_buf returns all of the remaining data unlike other BufReads which have outer reading source.
            // Unless we add other data by write, there will be no more data.
            let buf = self.cursor.fill_buf().map_err(|err| os_err(vm, err))?;
            if size < buf.len() {
                &buf[..size]
            } else {
                buf
            }
        };
        let buf = match available.find_byte(b'\n') {
            Some(i) => (available[..=i].to_vec()),
            _ => (available.to_vec()),
        };
        self.cursor.consume(buf.len());
        Ok(buf)
    }

    fn truncate(&mut self, pos: Option<usize>) -> PyResult<()> {
        let pos = pos.unwrap_or_else(|| self.tell() as usize);
        self.cursor.get_mut().truncate(pos);
        Ok(())
    }
}

#[derive(Debug)]
struct PyStringIO {
    buffer: PyRwLock<BufferedIO>,
    closed: AtomicCell<bool>,
}

type PyStringIORef = PyRef<PyStringIO>;

impl PyValue for PyStringIO {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("io", "StringIO")
    }
}

impl PyStringIORef {
    fn buffer(&self, vm: &VirtualMachine) -> PyResult<PyRwLockWriteGuard<'_, BufferedIO>> {
        if !self.closed.load() {
            Ok(self.buffer.write())
        } else {
            Err(vm.new_value_error("I/O operation on closed file.".to_owned()))
        }
    }

    //write string to underlying vector
    fn write(self, data: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let bytes = data.borrow_value().as_bytes();

        match self.buffer(vm)?.write(bytes) {
            Some(value) => Ok(vm.ctx.new_int(value)),
            None => Err(vm.new_type_error("Error Writing String".to_owned())),
        }
    }

    //return the entire contents of the underlying
    fn getvalue(self, vm: &VirtualMachine) -> PyResult {
        match String::from_utf8(self.buffer(vm)?.getvalue()) {
            Ok(result) => Ok(vm.ctx.new_str(result)),
            Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
        }
    }

    //skip to the jth position
    fn seek(
        self,
        offset: PyObjectRef,
        how: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<u64> {
        self.buffer(vm)?
            .seek(seekfrom(vm, offset, how)?)
            .map_err(|err| os_err(vm, err))
    }

    fn seekable(self) -> bool {
        true
    }

    //Read k bytes from the object and return.
    //If k is undefined || k == -1, then we read all bytes until the end of the file.
    //This also increments the stream position by the value of k
    fn read(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult {
        let data = match self.buffer(vm)?.read(size.to_usize()) {
            Some(value) => value,
            None => Vec::new(),
        };

        match String::from_utf8(data) {
            Ok(value) => Ok(vm.ctx.new_str(value)),
            Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
        }
    }

    fn tell(self, vm: &VirtualMachine) -> PyResult<u64> {
        Ok(self.buffer(vm)?.tell())
    }

    fn readline(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<String> {
        // TODO size should correspond to the number of characters, at the moments its the number of
        // bytes.
        match String::from_utf8(self.buffer(vm)?.readline(size.to_usize(), vm)?) {
            Ok(value) => Ok(value),
            Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
        }
    }

    fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<()> {
        let mut buffer = self.buffer(vm)?;
        buffer.truncate(pos.try_usize(vm)?)?;
        Ok(())
    }

    fn closed(self) -> bool {
        self.closed.load()
    }

    fn close(self) {
        self.closed.store(true);
    }
}

#[derive(FromArgs)]
struct StringIOArgs {
    #[pyarg(positional_or_keyword, default = "None")]
    #[allow(dead_code)]
    // TODO: use this
    newline: Option<PyStringRef>,
}

fn string_io_new(
    cls: PyClassRef,
    object: OptionalArg<Option<PyObjectRef>>,
    _args: StringIOArgs,
    vm: &VirtualMachine,
) -> PyResult<PyStringIORef> {
    let raw_bytes = object
        .flatten()
        .map_or_else(Vec::new, |v| objstr::borrow_value(&v).as_bytes().to_vec());

    PyStringIO {
        buffer: PyRwLock::new(BufferedIO::new(Cursor::new(raw_bytes))),
        closed: AtomicCell::new(false),
    }
    .into_ref_with_type(vm, cls)
}

#[derive(Debug)]
struct PyBytesIO {
    buffer: PyRwLock<BufferedIO>,
    closed: AtomicCell<bool>,
}

type PyBytesIORef = PyRef<PyBytesIO>;

impl PyValue for PyBytesIO {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("io", "BytesIO")
    }
}

impl PyBytesIORef {
    fn buffer(&self, vm: &VirtualMachine) -> PyResult<PyRwLockWriteGuard<'_, BufferedIO>> {
        if !self.closed.load() {
            Ok(self.buffer.write())
        } else {
            Err(vm.new_value_error("I/O operation on closed file.".to_owned()))
        }
    }

    fn write(self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<u64> {
        let mut buffer = self.buffer(vm)?;
        match data.with_ref(|b| buffer.write(b)) {
            Some(value) => Ok(value),
            None => Err(vm.new_type_error("Error Writing Bytes".to_owned())),
        }
    }
    //Retrieves the entire bytes object value from the underlying buffer
    fn getvalue(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_bytes(self.buffer(vm)?.getvalue()))
    }

    //Takes an integer k (bytes) and returns them from the underlying buffer
    //If k is undefined || k == -1, then we read all bytes until the end of the file.
    //This also increments the stream position by the value of k
    fn read(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult {
        match self.buffer(vm)?.read(size.to_usize()) {
            Some(value) => Ok(vm.ctx.new_bytes(value)),
            None => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
        }
    }

    //skip to the jth position
    fn seek(
        self,
        offset: PyObjectRef,
        how: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<u64> {
        self.buffer(vm)?
            .seek(seekfrom(vm, offset, how)?)
            .map_err(|err| os_err(vm, err))
    }

    fn seekable(self) -> bool {
        true
    }

    fn tell(self, vm: &VirtualMachine) -> PyResult<u64> {
        Ok(self.buffer(vm)?.tell())
    }

    fn readline(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
        self.buffer(vm)?.readline(size.to_usize(), vm)
    }

    fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<()> {
        let mut buffer = self.buffer(vm)?;
        buffer.truncate(pos.try_usize(vm)?)?;
        Ok(())
    }

    fn closed(self) -> bool {
        self.closed.load()
    }

    fn close(self) {
        self.closed.store(true)
    }
}

fn bytes_io_new(
    cls: PyClassRef,
    object: OptionalArg<Option<PyBytesRef>>,
    vm: &VirtualMachine,
) -> PyResult<PyBytesIORef> {
    let raw_bytes = object
        .flatten()
        .map_or_else(Vec::new, |input| input.borrow_value().to_vec());

    PyBytesIO {
        buffer: PyRwLock::new(BufferedIO::new(Cursor::new(raw_bytes))),
        closed: AtomicCell::new(false),
    }
    .into_ref_with_type(vm, cls)
}

fn io_base_cm_enter(instance: PyObjectRef) -> PyObjectRef {
    instance.clone()
}

fn io_base_cm_exit(instance: PyObjectRef, _args: PyFuncArgs, vm: &VirtualMachine) -> PyResult<()> {
    vm.call_method(&instance, "close", vec![])?;
    Ok(())
}

// TODO Check if closed, then if so raise ValueError
fn io_base_flush(_self: PyObjectRef) {}

fn io_base_seekable(_self: PyObjectRef) -> bool {
    false
}
fn io_base_readable(_self: PyObjectRef) -> bool {
    false
}
fn io_base_writable(_self: PyObjectRef) -> bool {
    false
}

fn io_base_closed(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    vm.get_attribute(instance, "__closed")
}

fn io_base_close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let closed = objbool::boolval(vm, io_base_closed(instance.clone(), vm)?)?;
    if !closed {
        let res = vm.call_method(&instance, "flush", vec![]);
        vm.set_attr(&instance, "__closed", vm.ctx.new_bool(true))?;
        res?;
    }
    Ok(())
}

fn io_base_readline(
    instance: PyObjectRef,
    size: OptionalSize,
    vm: &VirtualMachine,
) -> PyResult<Vec<u8>> {
    let size = size.to_usize();
    let read = vm.get_attribute(instance, "read")?;
    let mut res = Vec::new();
    while size.map_or(true, |s| res.len() < s) {
        let read_res =
            PyBytesLike::try_from_object(vm, vm.invoke(&read, vec![vm.ctx.new_int(1)])?)?;
        if read_res.with_ref(|b| b.is_empty()) {
            break;
        }
        read_res.with_ref(|b| res.extend_from_slice(b));
        if res.ends_with(b"\n") {
            break;
        }
    }
    Ok(res)
}

fn io_base_checkclosed(
    instance: PyObjectRef,
    msg: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if objbool::boolval(vm, vm.get_attribute(instance, "closed")?)? {
        let msg = msg
            .flatten()
            .unwrap_or_else(|| vm.ctx.new_str("I/O operation on closed file."));
        Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
    } else {
        Ok(())
    }
}

fn io_base_checkreadable(
    instance: PyObjectRef,
    msg: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if !objbool::boolval(vm, vm.call_method(&instance, "readable", vec![])?)? {
        let msg = msg
            .flatten()
            .unwrap_or_else(|| vm.ctx.new_str("File or stream is not readable."));
        Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
    } else {
        Ok(())
    }
}

fn io_base_checkwritable(
    instance: PyObjectRef,
    msg: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if !objbool::boolval(vm, vm.call_method(&instance, "writable", vec![])?)? {
        let msg = msg
            .flatten()
            .unwrap_or_else(|| vm.ctx.new_str("File or stream is not writable."));
        Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
    } else {
        Ok(())
    }
}

fn io_base_checkseekable(
    instance: PyObjectRef,
    msg: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if !objbool::boolval(vm, vm.call_method(&instance, "seekable", vec![])?)? {
        let msg = msg
            .flatten()
            .unwrap_or_else(|| vm.ctx.new_str("File or stream is not seekable."));
        Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
    } else {
        Ok(())
    }
}

fn io_base_iter(instance: PyObjectRef) -> PyObjectRef {
    instance
}
fn io_base_next(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let line = vm.call_method(&instance, "readline", vec![])?;
    if !objbool::boolval(vm, line.clone())? {
        Err(objiter::new_stop_iteration(vm))
    } else {
        Ok(line)
    }
}
fn io_base_readlines(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    Ok(vm.ctx.new_list(vm.extract_elements(&instance)?))
}

fn raw_io_base_read(instance: PyObjectRef, size: OptionalSize, vm: &VirtualMachine) -> PyResult {
    if let Some(size) = size.to_usize() {
        // FIXME: unnessessary zero-init
        let b = PyByteArray::from(vec![0; size]).into_ref(vm);
        let n = <Option<usize>>::try_from_object(
            vm,
            vm.call_method(&instance, "readinto", vec![b.as_object().clone()])?,
        )?;
        Ok(n.map(|n| {
            let bytes = &mut b.borrow_value_mut().elements;
            bytes.truncate(n);
            bytes.clone()
        })
        .into_pyobject(vm))
    } else {
        vm.call_method(&instance, "readall", vec![])
    }
}

fn buffered_io_base_init(
    instance: PyObjectRef,
    raw: PyObjectRef,
    buffer_size: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm.set_attr(&instance, "raw", raw.clone())?;
    vm.set_attr(
        &instance,
        "buffer_size",
        vm.ctx.new_int(buffer_size.unwrap_or(DEFAULT_BUFFER_SIZE)),
    )?;
    Ok(())
}

fn buffered_io_base_fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    vm.call_method(&raw, "fileno", vec![])
}

fn buffered_io_base_mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    vm.get_attribute(raw, "mode")
}

fn buffered_io_base_name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    vm.get_attribute(raw, "name")
}

fn buffered_reader_read(
    instance: PyObjectRef,
    size: OptionalSize,
    vm: &VirtualMachine,
) -> PyResult {
    vm.call_method(
        &vm.get_attribute(instance.clone(), "raw")?,
        "read",
        vec![size.to_usize().into_pyobject(vm)],
    )
}

fn buffered_reader_seekable(_self: PyObjectRef) -> bool {
    true
}

fn buffered_reader_seek(
    instance: PyObjectRef,
    offset: PyObjectRef,
    how: OptionalArg,
    vm: &VirtualMachine,
) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
    vm.invoke(&vm.get_attribute(raw, "seek")?, args)
}

fn buffered_io_base_tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    vm.invoke(&vm.get_attribute(raw, "tell")?, vec![])
}

fn buffered_io_base_close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    let raw = vm.get_attribute(instance, "raw")?;
    vm.invoke(&vm.get_attribute(raw, "close")?, vec![])?;
    Ok(())
}

// disable FileIO on WASM
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
mod fileio {
    use super::super::os;
    use super::*;

    fn compute_c_flag(mode: &str) -> u32 {
        let flag = match mode.chars().next() {
            Some(mode) => match mode {
                'w' => libc::O_WRONLY | libc::O_CREAT,
                'x' => libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                'a' => libc::O_APPEND,
                '+' => libc::O_RDWR,
                _ => libc::O_RDONLY,
            },
            None => libc::O_RDONLY,
        };
        flag as u32
    }

    #[derive(FromArgs)]
    struct FileIOArgs {
        #[pyarg(positional_only)]
        name: Either<PyStringRef, i64>,
        #[pyarg(positional_or_keyword, default = "None")]
        mode: Option<PyStringRef>,
        #[pyarg(positional_or_keyword, default = "true")]
        closefd: bool,
        #[pyarg(positional_or_keyword, default = "None")]
        opener: Option<PyObjectRef>,
    }
    fn file_io_init(file_io: PyObjectRef, args: FileIOArgs, vm: &VirtualMachine) -> PyResult {
        let mode = args
            .mode
            .map(|mode| mode.borrow_value().to_owned())
            .unwrap_or_else(|| "r".to_owned());
        let (name, file_no) = match args.name {
            Either::A(name) => {
                if !args.closefd {
                    return Err(
                        vm.new_value_error("Cannot use closefd=False with file name".to_owned())
                    );
                }
                let mode = compute_c_flag(&mode);
                let fd = if let Some(opener) = args.opener {
                    let fd = vm.invoke(
                        &opener,
                        vec![name.clone().into_object(), vm.ctx.new_int(mode)],
                    )?;
                    if !vm.isinstance(&fd, &vm.ctx.types.int_type)? {
                        return Err(vm.new_type_error("expected integer from opener".to_owned()));
                    }
                    let fd = i64::try_from_object(vm, fd)?;
                    if fd < 0 {
                        return Err(vm.new_os_error("Negative file descriptor".to_owned()));
                    }
                    fd
                } else {
                    os::open(
                        os::PyPathLike::new_str(name.borrow_value().to_owned()),
                        mode as _,
                        OptionalArg::Missing,
                        OptionalArg::Missing,
                        vm,
                    )?
                };
                (name.into_object(), fd)
            }
            Either::B(fno) => (vm.ctx.new_int(fno), fno),
        };

        vm.set_attr(&file_io, "name", name)?;
        vm.set_attr(&file_io, "mode", vm.ctx.new_str(mode))?;
        vm.set_attr(&file_io, "__fileno", vm.ctx.new_int(file_no))?;
        vm.set_attr(&file_io, "closefd", vm.ctx.new_bool(args.closefd))?;
        vm.set_attr(&file_io, "__closed", vm.ctx.new_bool(false))?;
        Ok(vm.get_none())
    }

    fn fio_get_fileno(instance: &PyObjectRef, vm: &VirtualMachine) -> PyResult<fs::File> {
        io_base_checkclosed(instance.clone(), OptionalArg::Missing, vm)?;
        let fileno = i64::try_from_object(vm, vm.get_attribute(instance.clone(), "__fileno")?)?;
        Ok(os::rust_file(fileno))
    }
    fn fio_set_fileno(instance: &PyObjectRef, f: fs::File, vm: &VirtualMachine) -> PyResult<()> {
        let updated = os::raw_file_number(f);
        vm.set_attr(&instance, "__fileno", vm.ctx.new_int(updated))?;
        Ok(())
    }

    fn file_io_read(
        instance: PyObjectRef,
        read_byte: OptionalSize,
        vm: &VirtualMachine,
    ) -> PyResult<Vec<u8>> {
        let mut handle = fio_get_fileno(&instance, vm)?;
        let bytes = if let Some(read_byte) = read_byte.to_usize() {
            let mut bytes = vec![0; read_byte as usize];
            let n = handle
                .read(&mut bytes)
                .map_err(|err| err.into_pyexception(vm))?;
            bytes.truncate(n);
            bytes
        } else {
            let mut bytes = vec![];
            handle
                .read_to_end(&mut bytes)
                .map_err(|err| err.into_pyexception(vm))?;
            bytes
        };
        fio_set_fileno(&instance, handle, vm)?;

        Ok(bytes)
    }

    fn file_io_readinto(
        instance: PyObjectRef,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if !obj.readonly() {
            return Err(vm.new_type_error(
                "readinto() argument must be read-write bytes-like object".to_owned(),
            ));
        }

        //extract length of buffer
        let py_length = vm.call_method(&obj, "__len__", PyFuncArgs::default())?;
        let length = objint::get_value(&py_length).to_u64().unwrap();

        let handle = fio_get_fileno(&instance, vm)?;

        let mut f = handle.take(length);
        if let Some(bytes) = obj.payload::<PyByteArray>() {
            //TODO: Implement for MemoryView

            let value_mut = &mut bytes.borrow_value_mut().elements;
            value_mut.clear();
            match f.read_to_end(value_mut) {
                Ok(_) => {}
                Err(_) => return Err(vm.new_value_error("Error reading from Take".to_owned())),
            }
        };

        fio_set_fileno(&instance, f.into_inner(), vm)?;

        Ok(())
    }

    fn file_io_write(
        instance: PyObjectRef,
        obj: PyBytesLike,
        vm: &VirtualMachine,
    ) -> PyResult<usize> {
        let mut handle = fio_get_fileno(&instance, vm)?;

        let len = obj
            .with_ref(|b| handle.write(b))
            .map_err(|err| err.into_pyexception(vm))?;

        fio_set_fileno(&instance, handle, vm)?;

        //return number of bytes written
        Ok(len)
    }

    fn file_io_close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let closefd = objbool::boolval(vm, vm.get_attribute(instance.clone(), "closefd")?)?;
        if closefd {
            let raw_handle =
                i64::try_from_object(vm, vm.get_attribute(instance.clone(), "__fileno")?)?;
            drop(os::rust_file(raw_handle));
        }
        vm.set_attr(&instance, "__closed", vm.ctx.new_bool(true))?;
        Ok(())
    }

    fn file_io_seekable(_self: PyObjectRef) -> bool {
        true
    }

    fn file_io_seek(
        instance: PyObjectRef,
        offset: PyObjectRef,
        how: OptionalArg<i32>,
        vm: &VirtualMachine,
    ) -> PyResult<u64> {
        let mut handle = fio_get_fileno(&instance, vm)?;

        let new_pos = handle
            .seek(seekfrom(vm, offset, how)?)
            .map_err(|err| err.into_pyexception(vm))?;

        fio_set_fileno(&instance, handle, vm)?;

        Ok(new_pos)
    }

    fn file_io_tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<u64> {
        let mut handle = fio_get_fileno(&instance, vm)?;

        let pos = handle
            .seek(SeekFrom::Current(0))
            .map_err(|err| err.into_pyexception(vm))?;

        fio_set_fileno(&instance, handle, vm)?;

        Ok(pos)
    }

    fn file_io_fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.get_attribute(instance, "__fileno")
    }

    pub fn make_fileio(ctx: &crate::pyobject::PyContext, raw_io_base: PyClassRef) -> PyClassRef {
        py_class!(ctx, "FileIO", raw_io_base, {
            "__init__" => ctx.new_method(file_io_init),
            "name" => ctx.str_type(),
            "read" => ctx.new_method(file_io_read),
            "readinto" => ctx.new_method(file_io_readinto),
            "write" => ctx.new_method(file_io_write),
            "close" => ctx.new_method(file_io_close),
            "seekable" => ctx.new_method(file_io_seekable),
            "seek" => ctx.new_method(file_io_seek),
            "tell" => ctx.new_method(file_io_tell),
            "fileno" => ctx.new_method(file_io_fileno),
        })
    }
}

fn buffered_writer_write(instance: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "raw").unwrap();

    //This should be replaced with a more appropriate chunking implementation
    vm.call_method(&raw, "write", vec![obj.clone()])
}

fn buffered_writer_seekable(_self: PyObjectRef) -> bool {
    true
}

fn buffered_writer_seek(
    instance: PyObjectRef,
    offset: PyObjectRef,
    how: OptionalArg,
    vm: &VirtualMachine,
) -> PyResult {
    let raw = vm.get_attribute(instance, "raw")?;
    let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
    vm.invoke(&vm.get_attribute(raw, "seek")?, args)
}

#[derive(FromArgs)]
struct TextIOWrapperArgs {
    #[pyarg(positional_or_keyword, optional = false)]
    buffer: PyObjectRef,
    #[pyarg(positional_or_keyword, default = "None")]
    encoding: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    errors: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    newline: Option<PyStringRef>,
}

impl TextIOWrapperArgs {
    fn validate_newline(&self, vm: &VirtualMachine) -> PyResult<()> {
        if let Some(pystr) = &self.newline {
            match pystr.borrow_value() {
                "" | "\n" | "\r" | "\r\n" => Ok(()),
                _ => {
                    Err(vm.new_value_error(format!("illegal newline value: '{}'", pystr.repr(vm)?)))
                }
            }
        } else {
            Ok(())
        }
    }
}

fn text_io_wrapper_init(
    instance: PyObjectRef,
    args: TextIOWrapperArgs,
    vm: &VirtualMachine,
) -> PyResult<()> {
    args.validate_newline(vm)?;

    let mut encoding: Option<PyStringRef> = args.encoding.clone();
    let mut self_encoding = None; // TODO: Try os.device_encoding(fileno)
    if encoding.is_none() && self_encoding.is_none() {
        // TODO: locale module
        self_encoding = Some("utf-8");
    }
    if let Some(self_encoding) = self_encoding {
        encoding = Some(PyString::from(self_encoding).into_ref(vm));
    } else if let Some(ref encoding) = encoding {
        self_encoding = Some(encoding.borrow_value())
    } else {
        return Err(vm.new_os_error("could not determine default encoding".to_owned()));
    }
    let _ = encoding; // TODO: check codec

    let errors = args
        .errors
        .map_or_else(|| vm.ctx.new_str("strict"), |o| o.into_object());

    // let readuniversal = args.newline.map_or_else(true, |s| s.borrow_value().is_empty());

    vm.set_attr(
        &instance,
        "encoding",
        self_encoding.map_or_else(|| vm.get_none(), |s| vm.ctx.new_str(s)),
    )?;
    vm.set_attr(&instance, "errors", errors)?;
    vm.set_attr(&instance, "buffer", args.buffer.clone())?;

    Ok(())
}

fn text_io_wrapper_seekable(_self: PyObjectRef) -> bool {
    true
}

fn text_io_wrapper_seek(
    instance: PyObjectRef,
    offset: PyObjectRef,
    how: OptionalArg,
    vm: &VirtualMachine,
) -> PyResult {
    let raw = vm.get_attribute(instance, "buffer")?;
    let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
    vm.invoke(&vm.get_attribute(raw, "seek")?, args)
}

fn text_io_wrapper_tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "buffer")?;
    vm.invoke(&vm.get_attribute(raw, "tell")?, vec![])
}

fn text_io_wrapper_mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "buffer")?;
    vm.get_attribute(raw, "mode")
}

fn text_io_wrapper_name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "buffer")?;
    vm.get_attribute(raw, "name")
}

fn text_io_wrapper_fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let raw = vm.get_attribute(instance, "buffer")?;
    vm.call_method(&raw, "fileno", vec![])
}

fn text_io_wrapper_read(
    instance: PyObjectRef,
    size: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let buffered_reader_class = vm.try_class("_io", "BufferedReader")?;
    let raw = vm.get_attribute(instance.clone(), "buffer").unwrap();

    if !objtype::isinstance(&raw, &buffered_reader_class) {
        // TODO: this should be io.UnsupportedOperation error which derives both from ValueError *and* OSError
        return Err(vm.new_value_error("not readable".to_owned()));
    }

    let bytes = vm.call_method(
        &raw,
        "read",
        vec![size.flatten().unwrap_or_else(|| vm.get_none())],
    )?;
    let bytes = PyBytesLike::try_from_object(vm, bytes)?;
    //format bytes into string
    let rust_string = String::from_utf8(bytes.to_cow().into_owned()).map_err(|e| {
        vm.new_unicode_decode_error(format!(
            "cannot decode byte at index: {}",
            e.utf8_error().valid_up_to()
        ))
    })?;
    Ok(rust_string)
}

fn text_io_wrapper_write(
    instance: PyObjectRef,
    obj: PyStringRef,
    vm: &VirtualMachine,
) -> PyResult<usize> {
    use std::str::from_utf8;

    let buffered_writer_class = vm.try_class("_io", "BufferedWriter")?;
    let raw = vm.get_attribute(instance.clone(), "buffer").unwrap();

    if !objtype::isinstance(&raw, &buffered_writer_class) {
        // TODO: this should be io.UnsupportedOperation error which derives from ValueError and OSError
        return Err(vm.new_value_error("not writable".to_owned()));
    }

    let bytes = obj.borrow_value().to_owned().into_bytes();

    let len = vm.call_method(&raw, "write", vec![vm.ctx.new_bytes(bytes.clone())])?;
    let len = objint::get_value(&len)
        .to_usize()
        .ok_or_else(|| vm.new_overflow_error("int to large to convert to Rust usize".to_owned()))?;

    // returns the count of unicode code points written
    let len = from_utf8(&bytes[..len])
        .unwrap_or_else(|e| from_utf8(&bytes[..e.valid_up_to()]).unwrap())
        .chars()
        .count();
    Ok(len)
}

fn text_io_wrapper_readline(
    instance: PyObjectRef,
    size: OptionalOption<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let buffered_reader_class = vm.try_class("_io", "BufferedReader")?;
    let raw = vm.get_attribute(instance.clone(), "buffer").unwrap();

    if !objtype::isinstance(&raw, &buffered_reader_class) {
        // TODO: this should be io.UnsupportedOperation error which derives both from ValueError *and* OSError
        return Err(vm.new_value_error("not readable".to_owned()));
    }

    let bytes = vm.call_method(
        &raw,
        "readline",
        vec![size.flatten().unwrap_or_else(|| vm.get_none())],
    )?;
    let bytes = PyBytesLike::try_from_object(vm, bytes)?;
    //format bytes into string
    let rust_string = String::from_utf8(bytes.to_cow().into_owned()).map_err(|e| {
        vm.new_unicode_decode_error(format!(
            "cannot decode byte at index: {}",
            e.utf8_error().valid_up_to()
        ))
    })?;
    Ok(rust_string)
}

fn split_mode_string(mode_string: &str) -> Result<(String, String), String> {
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
                        return Err("can't have text and binary mode at once".to_owned());
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
                            "must have exactly one of create/read/write/append mode".to_owned()
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
                .to_owned(),
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

fn io_open_wrapper(
    file: PyObjectRef,
    mode: OptionalArg<PyStringRef>,
    opts: OpenArgs,
    vm: &VirtualMachine,
) -> PyResult {
    io_open(
        file,
        mode.as_ref().into_option().map(|s| s.borrow_value()),
        opts,
        vm,
    )
}
fn io_open_code(file: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    // TODO: lifecycle hooks or something?
    io_open(file, Some("rb"), Default::default(), vm)
}

#[derive(FromArgs)]
#[allow(unused)]
pub struct OpenArgs {
    #[pyarg(positional_or_keyword, default = "-1")]
    buffering: isize,
    #[pyarg(positional_or_keyword, default = "None")]
    encoding: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    errors: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "None")]
    newline: Option<PyStringRef>,
    #[pyarg(positional_or_keyword, default = "true")]
    closefd: bool,
    #[pyarg(positional_or_keyword, default = "None")]
    opener: Option<PyObjectRef>,
}
impl Default for OpenArgs {
    fn default() -> Self {
        OpenArgs {
            buffering: -1,
            encoding: None,
            errors: None,
            newline: None,
            closefd: true,
            opener: None,
        }
    }
}

pub fn io_open(
    file: PyObjectRef,
    mode: Option<&str>,
    opts: OpenArgs,
    vm: &VirtualMachine,
) -> PyResult {
    // mode is optional: 'rt' is the default mode (open from reading text)
    let mode_string = mode.unwrap_or("rt");

    let (mode, typ) = match split_mode_string(mode_string) {
        Ok((mode, typ)) => (mode, typ),
        Err(error_message) => {
            return Err(vm.new_value_error(error_message));
        }
    };

    let io_module = vm.import("_io", &[], 0)?;

    // Construct a FileIO (subclass of RawIOBase)
    // This is subsequently consumed by a Buffered Class.
    let file_io_class = vm.get_attribute(io_module.clone(), "FileIO").map_err(|_| {
        // TODO: UnsupportedOperation here
        vm.new_os_error(
            "Couldn't get FileIO, io.open likely isn't supported on your platform".to_owned(),
        )
    })?;
    let file_io_obj = vm.invoke(
        &file_io_class,
        PyFuncArgs::from((
            Args::new(vec![file.clone(), vm.ctx.new_str(mode.clone())]),
            KwArgs::new(maplit::hashmap! {
                "closefd".to_owned() => vm.ctx.new_bool(opts.closefd),
                "opener".to_owned() => opts.opener.unwrap_or_else(|| vm.get_none()),
            }),
        )),
    )?;

    vm.set_attr(&file_io_obj, "mode", vm.ctx.new_str(mode_string))?;

    // Create Buffered class to consume FileIO. The type of buffered class depends on
    // the operation in the mode.
    // There are 3 possible classes here, each inheriting from the RawBaseIO
    // creating || writing || appending => BufferedWriter
    let buffered = match mode.chars().next().unwrap() {
        'w' | 'a' => {
            let buffered_writer_class = vm
                .get_attribute(io_module.clone(), "BufferedWriter")
                .unwrap();
            vm.invoke(&buffered_writer_class, vec![file_io_obj.clone()])
        }
        'r' => {
            let buffered_reader_class = vm
                .get_attribute(io_module.clone(), "BufferedReader")
                .unwrap();
            vm.invoke(&buffered_reader_class, vec![file_io_obj.clone()])
        }
        //TODO: updating => PyBufferedRandom
        _ => unimplemented!("'+' modes is not yet implemented"),
    };

    match typ.chars().next().unwrap() {
        // If the mode is text this buffer type is consumed on construction of
        // a TextIOWrapper which is subsequently returned.
        't' => {
            let text_io_wrapper_class = vm.get_attribute(io_module, "TextIOWrapper").unwrap();
            vm.invoke(&text_io_wrapper_class, vec![buffered.unwrap()])
        }
        // If the mode is binary this Buffered class is returned directly at
        // this point.
        // For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field
        'b' => buffered,
        _ => unreachable!(),
    }
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    // IOBase the abstract base class of the IO Module
    let io_base = py_class!(ctx, "_IOBase", ctx.object(), {
        "__enter__" => ctx.new_method(io_base_cm_enter),
        "__exit__" => ctx.new_method(io_base_cm_exit),
        "seekable" => ctx.new_method(io_base_seekable),
        "readable" => ctx.new_method(io_base_readable),
        "writable" => ctx.new_method(io_base_writable),
        "flush" => ctx.new_method(io_base_flush),
        "closed" => ctx.new_readonly_getset("closed", io_base_closed),
        "__closed" => ctx.new_bool(false),
        "close" => ctx.new_method(io_base_close),
        "readline" => ctx.new_method(io_base_readline),
        "_checkClosed" => ctx.new_method(io_base_checkclosed),
        "_checkReadable" => ctx.new_method(io_base_checkreadable),
        "_checkWritable" => ctx.new_method(io_base_checkwritable),
        "_checkSeekable" => ctx.new_method(io_base_checkseekable),
        "__iter__" => ctx.new_method(io_base_iter),
        "__next__" => ctx.new_method(io_base_next),
        "readlines" => ctx.new_method(io_base_readlines),
    });

    // IOBase Subclasses
    let raw_io_base = py_class!(ctx, "_RawIOBase", io_base.clone(), {
        "read" => ctx.new_method(raw_io_base_read),
    });

    let buffered_io_base = py_class!(ctx, "_BufferedIOBase", io_base.clone(), {});

    //TextIO Base has no public constructor
    let text_io_base = py_class!(ctx, "_TextIOBase", io_base.clone(), {});

    // BufferedIOBase Subclasses
    let buffered_reader = py_class!(ctx, "BufferedReader", buffered_io_base.clone(), {
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547
        "__init__" => ctx.new_method(buffered_io_base_init),
        "read" => ctx.new_method(buffered_reader_read),
        "seekable" => ctx.new_method(buffered_reader_seekable),
        "seek" => ctx.new_method(buffered_reader_seek),
        "tell" => ctx.new_method(buffered_io_base_tell),
        "close" => ctx.new_method(buffered_io_base_close),
        "fileno" => ctx.new_method(buffered_io_base_fileno),
        "name" => ctx.new_readonly_getset("name", buffered_io_base_name),
        "mode" => ctx.new_readonly_getset("mode", buffered_io_base_mode),
    });

    let buffered_writer = py_class!(ctx, "BufferedWriter", buffered_io_base.clone(), {
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547
        "__init__" => ctx.new_method(buffered_io_base_init),
        "write" => ctx.new_method(buffered_writer_write),
        "seekable" => ctx.new_method(buffered_writer_seekable),
        "seek" => ctx.new_method(buffered_writer_seek),
        "fileno" => ctx.new_method(buffered_io_base_fileno),
        "tell" => ctx.new_method(buffered_io_base_tell),
        "close" => ctx.new_method(buffered_io_base_close),
        "name" => ctx.new_readonly_getset("name", buffered_io_base_name),
        "mode" => ctx.new_readonly_getset("mode", buffered_io_base_mode),
    });

    //TextIOBase Subclass
    let text_io_wrapper = py_class!(ctx, "TextIOWrapper", text_io_base.clone(), {
        "__init__" => ctx.new_method(text_io_wrapper_init),
        "seekable" => ctx.new_method(text_io_wrapper_seekable),
        "seek" => ctx.new_method(text_io_wrapper_seek),
        "tell" => ctx.new_method(text_io_wrapper_tell),
        "read" => ctx.new_method(text_io_wrapper_read),
        "write" => ctx.new_method(text_io_wrapper_write),
        "readline" => ctx.new_method(text_io_wrapper_readline),
        "fileno" => ctx.new_method(text_io_wrapper_fileno),
        "name" => ctx.new_readonly_getset("name", text_io_wrapper_name),
        "mode" => ctx.new_readonly_getset("mode", text_io_wrapper_mode),
    });

    //StringIO: in-memory text
    let string_io = py_class!(ctx, "StringIO", text_io_base.clone(), {
        "__module__" => ctx.new_str("_io"),
        (slot new) => string_io_new,
        "seek" => ctx.new_method(PyStringIORef::seek),
        "seekable" => ctx.new_method(PyStringIORef::seekable),
        "read" => ctx.new_method(PyStringIORef::read),
        "write" => ctx.new_method(PyStringIORef::write),
        "getvalue" => ctx.new_method(PyStringIORef::getvalue),
        "tell" => ctx.new_method(PyStringIORef::tell),
        "readline" => ctx.new_method(PyStringIORef::readline),
        "truncate" => ctx.new_method(PyStringIORef::truncate),
        "closed" => ctx.new_readonly_getset("closed", PyStringIORef::closed),
        "close" => ctx.new_method(PyStringIORef::close),
    });

    //BytesIO: in-memory bytes
    let bytes_io = py_class!(ctx, "BytesIO", buffered_io_base.clone(), {
        (slot new) => bytes_io_new,
        "read" => ctx.new_method(PyBytesIORef::read),
        "read1" => ctx.new_method(PyBytesIORef::read),
        "seek" => ctx.new_method(PyBytesIORef::seek),
        "seekable" => ctx.new_method(PyBytesIORef::seekable),
        "write" => ctx.new_method(PyBytesIORef::write),
        "getvalue" => ctx.new_method(PyBytesIORef::getvalue),
        "tell" => ctx.new_method(PyBytesIORef::tell),
        "readline" => ctx.new_method(PyBytesIORef::readline),
        "truncate" => ctx.new_method(PyBytesIORef::truncate),
        "closed" => ctx.new_readonly_getset("closed", PyBytesIORef::closed),
        "close" => ctx.new_method(PyBytesIORef::close),
    });

    let module = py_module!(vm, "_io", {
        "open" => ctx.new_function(io_open_wrapper),
        "open_code" => ctx.new_function(io_open_code),
        "_IOBase" => io_base,
        "_RawIOBase" => raw_io_base.clone(),
        "_BufferedIOBase" => buffered_io_base,
        "_TextIOBase" => text_io_base,
        "BufferedReader" => buffered_reader,
        "BufferedWriter" => buffered_writer,
        "TextIOWrapper" => text_io_wrapper,
        "StringIO" => string_io,
        "BytesIO" => bytes_io,
        "DEFAULT_BUFFER_SIZE" => ctx.new_int(8 * 1024),
    });

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    extend_module!(vm, module, {
        "FileIO" => fileio::make_fileio(ctx, raw_io_base),
    });

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_mode_split_into(mode_string: &str, expected_mode: &str, expected_typ: &str) {
        let (mode, typ) = split_mode_string(mode_string).unwrap();
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
            split_mode_string("rbsss"),
            Err("invalid mode: 'rbsss'".to_owned())
        );
        assert_eq!(
            split_mode_string("rrb"),
            Err("invalid mode: 'rrb'".to_owned())
        );
        assert_eq!(
            split_mode_string("rbb"),
            Err("invalid mode: 'rbb'".to_owned())
        );
    }

    #[test]
    fn test_mode_not_specified() {
        assert_eq!(
            split_mode_string(""),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_owned()
            )
        );
        assert_eq!(
            split_mode_string("b"),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_owned()
            )
        );
        assert_eq!(
            split_mode_string("t"),
            Err(
                "Must have exactly one of create/read/write/append mode and at most one plus"
                    .to_owned()
            )
        );
    }

    #[test]
    fn test_text_and_binary_at_once() {
        assert_eq!(
            split_mode_string("rbt"),
            Err("can't have text and binary mode at once".to_owned())
        );
    }

    #[test]
    fn test_exactly_one_mode() {
        assert_eq!(
            split_mode_string("rwb"),
            Err("must have exactly one of create/read/write/append mode".to_owned())
        );
    }

    #[test]
    fn test_at_most_one_plus() {
        assert_eq!(
            split_mode_string("a++"),
            Err("invalid mode: 'a++'".to_owned())
        );
    }

    #[test]
    fn test_buffered_read() {
        let data = vec![1, 2, 3, 4];
        let bytes = None;
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
            cursor: Cursor::new(data),
        };

        assert_eq!(buffered.seek(SeekFrom::Start(count)).unwrap(), count);
        assert_eq!(buffered.read(Some(count as usize)).unwrap(), vec![3, 4]);
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
