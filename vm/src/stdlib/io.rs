/*
 * I/O core tools.
 */
cfg_if::cfg_if! {
    if #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))] {
        use super::os::Offset;
    } else {
        type Offset = i64;
    }
}

#[cfg(unix)]
use crate::stdlib::os::{errno_err, PathOrFd};
use crate::VirtualMachine;
use crate::{PyObjectRef, PyResult, TryFromObject};
pub(crate) use _io::io_open as open;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let module = _io::make_module(vm);

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    fileio::extend_module(vm, &module);

    let unsupported_operation = _io::UNSUPPORTED_OPERATION
        .get_or_init(|| _io::make_unsupportedop(ctx))
        .clone();
    extend_module!(vm, module, {
        "UnsupportedOperation" => unsupported_operation,
        "BlockingIOError" => ctx.exceptions.blocking_io_error.clone(),
    });

    module
}

// not used on all platforms
#[allow(unused)]
#[derive(Copy, Clone)]
#[repr(transparent)]
pub(crate) struct Fildes(pub i32);

impl TryFromObject for Fildes {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        use crate::builtins::int;
        let int = match obj.downcast::<int::PyInt>() {
            Ok(i) => i,
            Err(obj) => {
                let fileno_meth = vm.get_attribute_opt(obj, "fileno")?.ok_or_else(|| {
                    vm.new_type_error(
                        "argument must be an int, or have a fileno() method.".to_owned(),
                    )
                })?;
                vm.invoke(&fileno_meth, ())?
                    .downcast()
                    .map_err(|_| vm.new_type_error("fileno() returned a non-integer".to_owned()))?
            }
        };
        let fd = int::try_to_primitive(int.as_bigint(), vm)?;
        if fd < 0 {
            return Err(vm.new_value_error(format!(
                "file descriptor cannot be a negative integer ({})",
                fd
            )));
        }
        Ok(Fildes(fd))
    }
}

#[pymodule]
mod _io {
    use super::*;

    use bstr::ByteSlice;
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::ToPrimitive;
    use std::io::{self, prelude::*, Cursor, SeekFrom};
    use std::ops::Range;

    use crate::buffer::{BufferOptions, PyBuffer, PyBufferRef, ResizeGuard};
    use crate::builtins::memory::PyMemoryView;
    use crate::builtins::{
        bytes::{PyBytes, PyBytesRef},
        pybool, pytype, PyByteArray, PyStr, PyStrRef, PyTypeRef,
    };
    use crate::byteslike::{ArgBytesLike, ArgMemoryBuffer};
    use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
    use crate::common::lock::{
        PyMappedThreadMutexGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
        PyThreadMutex, PyThreadMutexGuard,
    };
    use crate::common::rc::PyRc;
    use crate::exceptions::{self, IntoPyException, PyBaseExceptionRef};
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::utils::Either;
    use crate::vm::{ReprGuard, VirtualMachine};
    use crate::{
        IdProtocol, IntoPyObject, PyContext, PyIterable, PyObjectRef, PyRef, PyResult, PyValue,
        StaticType, TryFromObject, TypeProtocol,
    };

    #[allow(clippy::let_and_return)]
    fn validate_whence(whence: i32) -> bool {
        let x = (0..=2).contains(&whence);
        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "dragonfly", target_os = "freebsd", target_os = "linux"))] {
                x || matches!(whence, libc::SEEK_DATA | libc::SEEK_HOLE)
            } else {
                x
            }
        }
    }

    fn ensure_unclosed(file: &PyObjectRef, msg: &str, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.get_attribute(file.clone(), "closed")?)? {
            Err(vm.new_value_error(msg.to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn new_unsupported_operation(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
        vm.new_exception_msg(UNSUPPORTED_OPERATION.get().unwrap().clone(), msg)
    }

    fn _unsupported<T>(vm: &VirtualMachine, zelf: &PyObjectRef, operation: &str) -> PyResult<T> {
        Err(new_unsupported_operation(
            vm,
            format!("{}.{}() not supported", zelf.class().name, operation),
        ))
    }

    #[derive(FromArgs)]
    pub(super) struct OptionalSize {
        // In a few functions, the default value is -1 rather than None.
        // Make sure the default value doesn't affect compatibility.
        #[pyarg(positional, default)]
        size: Option<isize>,
    }

    impl OptionalSize {
        pub fn to_usize(self) -> Option<usize> {
            self.size.and_then(|v| v.to_usize())
        }

        pub fn try_usize(self, vm: &VirtualMachine) -> PyResult<Option<usize>> {
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

    pub(super) fn io_closed_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_value_error("I/O operation on closed file".to_owned())
    }

    #[pyattr]
    const DEFAULT_BUFFER_SIZE: usize = 8 * 1024;

    pub(super) fn seekfrom(
        vm: &VirtualMachine,
        offset: PyObjectRef,
        how: OptionalArg<i32>,
    ) -> PyResult<SeekFrom> {
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
            let pos = self.cursor.position().to_usize()?;
            let avail_slice = self.cursor.get_ref().get(pos..)?;
            // if we don't specify the number of bytes, or it's too big, give the whole rest of the slice
            let n = bytes.map_or_else(
                || avail_slice.len(),
                |n| std::cmp::min(n, avail_slice.len()),
            );
            let b = avail_slice[..n].to_vec();
            self.cursor.set_position((pos + n) as u64);
            Some(b)
        }

        fn tell(&self) -> u64 {
            self.cursor.position()
        }

        fn readline(&mut self, size: Option<usize>, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.read_until(size, b'\n', vm)
        }

        fn read_until(
            &mut self,
            size: Option<usize>,
            byte: u8,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let size = match size {
                None => {
                    let mut buf: Vec<u8> = Vec::new();
                    self.cursor
                        .read_until(byte, &mut buf)
                        .map_err(|err| os_err(vm, err))?;
                    return Ok(buf);
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
            let buf = match available.find_byte(byte) {
                Some(i) => (available[..=i].to_vec()),
                _ => (available.to_vec()),
            };
            self.cursor.consume(buf.len());
            Ok(buf)
        }

        fn truncate(&mut self, pos: Option<usize>) -> usize {
            let pos = pos.unwrap_or_else(|| self.tell() as usize);
            self.cursor.get_mut().truncate(pos);
            pos
        }
    }

    fn file_closed(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<bool> {
        pybool::boolval(vm, vm.get_attribute(file.clone(), "closed")?)
    }
    fn check_closed(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if file_closed(file, vm)? {
            Err(io_closed_error(vm))
        } else {
            Ok(())
        }
    }

    fn check_readable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "readable", ())?)? {
            Ok(())
        } else {
            Err(new_unsupported_operation(
                vm,
                "File or stream is not readable".to_owned(),
            ))
        }
    }

    fn check_writable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "writable", ())?)? {
            Ok(())
        } else {
            Err(new_unsupported_operation(
                vm,
                "File or stream is not writable.".to_owned(),
            ))
        }
    }

    fn check_seekable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "seekable", ())?)? {
            Ok(())
        } else {
            Err(new_unsupported_operation(
                vm,
                "File or stream is not seekable".to_owned(),
            ))
        }
    }

    fn check_decoded(decoded: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        decoded.downcast().map_err(|obj| {
            vm.new_type_error(format!(
                "decoder should return a string result, not '{}'",
                obj.class().name
            ))
        })
    }

    #[pyattr]
    #[pyclass(name = "_IOBase")]
    struct _IOBase;

    #[pyimpl(flags(BASETYPE, HAS_DICT))]
    impl _IOBase {
        #[pymethod]
        fn seek(
            zelf: PyObjectRef,
            _pos: PyObjectRef,
            _whence: OptionalArg,
            vm: &VirtualMachine,
        ) -> PyResult {
            _unsupported(vm, &zelf, "seek")
        }
        #[pymethod]
        fn tell(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            vm.call_method(&zelf, "seek", (0, 1))
        }
        #[pymethod]
        fn truncate(zelf: PyObjectRef, _pos: OptionalArg, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "truncate")
        }
        #[pymethod]
        fn fileno(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "truncate")
        }

        #[pyattr]
        fn __closed(ctx: &PyContext) -> PyObjectRef {
            ctx.new_bool(false)
        }

        #[pymethod(magic)]
        fn enter(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            check_closed(&instance, vm)?;
            Ok(instance)
        }

        #[pyslot]
        fn tp_del(instance: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let _ = vm.call_method(instance, "close", ());
            Ok(())
        }

        #[pymethod(magic)]
        fn del(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            Self::tp_del(&instance, vm)
        }

        #[pymethod(magic)]
        fn exit(instance: PyObjectRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            vm.call_method(&instance, "close", ())?;
            Ok(())
        }

        #[pymethod]
        fn flush(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            // just check if this is closed; if it isn't, do nothing
            check_closed(&instance, vm)
        }

        #[pymethod]
        fn seekable(_self: PyObjectRef) -> bool {
            false
        }
        #[pymethod]
        fn readable(_self: PyObjectRef) -> bool {
            false
        }
        #[pymethod]
        fn writable(_self: PyObjectRef) -> bool {
            false
        }

        #[pymethod]
        fn isatty(_self: PyObjectRef) -> bool {
            false
        }

        #[pyproperty]
        fn closed(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(instance, "__closed")
        }

        #[pymethod]
        fn close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            iobase_close(&instance, vm)
        }

        #[pymethod]
        fn readline(
            instance: PyObjectRef,
            size: OptionalSize,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<u8>> {
            let size = size.to_usize();
            let read = vm.get_attribute(instance, "read")?;
            let mut res = Vec::new();
            while size.map_or(true, |s| res.len() < s) {
                let read_res = ArgBytesLike::try_from_object(vm, vm.invoke(&read, (1,))?)?;
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

        #[pymethod]
        fn readlines(
            instance: PyObjectRef,
            hint: OptionalOption<isize>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let hint = hint.flatten().unwrap_or(-1);
            if hint <= 0 {
                return Ok(vm.ctx.new_list(vm.extract_elements(&instance)?));
            }
            let hint = hint as usize;
            let mut ret = Vec::new();
            let it = PyIterable::try_from_object(vm, instance)?;
            let mut full_len = 0;
            for line in it.iter(vm)? {
                let line = line?;
                let line_len = vm.obj_len(&line)?;
                ret.push(line.clone());
                full_len += line_len;
                if full_len > hint {
                    break;
                }
            }
            Ok(vm.ctx.new_list(ret))
        }

        #[pymethod]
        fn writelines(
            instance: PyObjectRef,
            lines: PyIterable,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            check_closed(&instance, vm)?;
            for line in lines.iter(vm)? {
                vm.call_method(&instance, "write", (line?,))?;
            }
            Ok(())
        }

        #[pymethod(name = "_checkClosed")]
        fn check_closed(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_closed(&instance, vm)
        }

        #[pymethod(name = "_checkReadable")]
        fn check_readable(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_readable(&instance, vm)
        }

        #[pymethod(name = "_checkWritable")]
        fn check_writable(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_writable(&instance, vm)
        }

        #[pymethod(name = "_checkSeekable")]
        fn check_seekable(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            check_seekable(&instance, vm)
        }

        #[pyslot]
        #[pymethod(name = "__iter__")]
        fn tp_iter(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            check_closed(&instance, vm)?;
            Ok(instance)
        }
        #[pyslot]
        fn tp_iternext(instance: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let line = vm.call_method(instance, "readline", ())?;
            if !pybool::boolval(vm, line.clone())? {
                Err(vm.new_stop_iteration())
            } else {
                Ok(line)
            }
        }
        #[pymethod(magic)]
        fn next(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Self::tp_iternext(&instance, vm)
        }
    }

    pub(super) fn iobase_close(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if !file_closed(file, vm)? {
            let res = vm.call_method(file, "flush", ());
            vm.set_attr(file, "__closed", vm.ctx.new_bool(true))?;
            res?;
        }
        Ok(())
    }

    #[pyattr]
    #[pyclass(name = "_RawIOBase", base = "_IOBase")]
    pub(super) struct _RawIOBase;

    #[pyimpl(flags(BASETYPE, HAS_DICT))]
    impl _RawIOBase {
        #[pymethod]
        fn read(instance: PyObjectRef, size: OptionalSize, vm: &VirtualMachine) -> PyResult {
            if let Some(size) = size.to_usize() {
                // FIXME: unnessessary zero-init
                let b = PyByteArray::from(vec![0; size]).into_ref(vm);
                let n = <Option<usize>>::try_from_object(
                    vm,
                    vm.call_method(&instance, "readinto", (b.clone(),))?,
                )?;
                Ok(n.map(|n| {
                    let mut bytes = b.borrow_buf_mut();
                    bytes.truncate(n);
                    // FIXME: try to use Arc::unwrap on the bytearray to get at the inner buffer
                    bytes.clone()
                })
                .into_pyobject(vm))
            } else {
                vm.call_method(&instance, "readall", ())
            }
        }

        #[pymethod]
        fn readall(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<Vec<u8>>> {
            let mut chunks = Vec::new();
            let mut total_len = 0;
            loop {
                let data = vm.call_method(&instance, "read", (DEFAULT_BUFFER_SIZE,))?;
                let data = <Option<PyBytesRef>>::try_from_object(vm, data)?;
                match data {
                    None => {
                        if chunks.is_empty() {
                            return Ok(None);
                        }
                        break;
                    }
                    Some(b) => {
                        if b.as_bytes().is_empty() {
                            break;
                        }
                        total_len += b.as_bytes().len();
                        chunks.push(b)
                    }
                }
            }
            let mut ret = Vec::with_capacity(total_len);
            for b in chunks {
                ret.extend_from_slice(b.as_bytes())
            }
            Ok(Some(ret))
        }
    }

    #[pyattr]
    #[pyclass(name = "_BufferedIOBase", base = "_IOBase")]
    struct _BufferedIOBase;

    #[pyimpl(flags(BASETYPE))]
    impl _BufferedIOBase {
        #[pymethod]
        fn read(zelf: PyObjectRef, _size: OptionalArg, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "read")
        }
        #[pymethod]
        fn read1(zelf: PyObjectRef, _size: OptionalArg, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "read1")
        }
        fn _readinto(
            zelf: PyObjectRef,
            bufobj: PyObjectRef,
            method: &str,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let b = ArgMemoryBuffer::new(vm, &bufobj)?;
            let l = b.len();
            let data = vm.call_method(&zelf, method, (l,))?;
            if data.is(&bufobj) {
                return Ok(l);
            }
            let mut buf = b.borrow_buf_mut();
            let data = ArgBytesLike::try_from_object(vm, data)?;
            let data = data.borrow_buf();
            match buf.get_mut(..data.len()) {
                Some(slice) => {
                    slice.copy_from_slice(&data);
                    Ok(data.len())
                }
                None => Err(vm.new_value_error(
                    "readinto: buffer and read data have different lengths".to_owned(),
                )),
            }
        }
        #[pymethod]
        fn readinto(zelf: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            Self::_readinto(zelf, b, "read", vm)
        }
        #[pymethod]
        fn readinto1(zelf: PyObjectRef, b: PyObjectRef, vm: &VirtualMachine) -> PyResult<usize> {
            Self::_readinto(zelf, b, "read1", vm)
        }
        #[pymethod]
        fn write(zelf: PyObjectRef, _b: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "write")
        }
        #[pymethod]
        fn detach(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "detach")
        }
    }

    // TextIO Base has no public constructor
    #[pyattr]
    #[pyclass(name = "_TextIOBase", base = "_IOBase")]
    struct _TextIOBase;

    #[pyimpl(flags(BASETYPE))]
    impl _TextIOBase {}

    #[derive(FromArgs, Clone)]
    struct BufferSize {
        #[pyarg(any, optional)]
        buffer_size: OptionalArg<isize>,
    }

    bitflags::bitflags! {
        #[derive(Default)]
        struct BufferedFlags: u8 {
            const DETACHED = 1 << 0;
            const WRITABLE = 1 << 1;
            const READABLE = 1 << 2;
        }
    }

    #[derive(Debug, Default)]
    struct BufferedData {
        raw: Option<PyObjectRef>,
        flags: BufferedFlags,
        abs_pos: Offset,
        buffer: Vec<u8>,
        pos: Offset,
        raw_pos: Offset,
        read_end: Offset,
        write_pos: Offset,
        write_end: Offset,
    }

    impl BufferedData {
        fn check_init(&self, vm: &VirtualMachine) -> PyResult<&PyObjectRef> {
            if let Some(raw) = &self.raw {
                Ok(raw)
            } else {
                let msg = if self.flags.contains(BufferedFlags::DETACHED) {
                    "raw stream has been detached"
                } else {
                    "I/O operation on uninitialized object"
                };
                Err(vm.new_value_error(msg.to_owned()))
            }
        }

        #[inline]
        fn writable(&self) -> bool {
            self.flags.contains(BufferedFlags::WRITABLE)
        }
        #[inline]
        fn readable(&self) -> bool {
            self.flags.contains(BufferedFlags::READABLE)
        }

        #[inline]
        fn valid_read(&self) -> bool {
            self.readable() && self.read_end != -1
        }
        #[inline]
        fn valid_write(&self) -> bool {
            self.writable() && self.write_end != -1
        }

        #[inline]
        fn raw_offset(&self) -> Offset {
            if (self.valid_read() || self.valid_write()) && self.raw_pos >= 0 {
                self.raw_pos - self.pos
            } else {
                0
            }
        }
        #[inline]
        fn readahead(&self) -> Offset {
            if self.valid_read() {
                self.read_end - self.pos
            } else {
                0
            }
        }

        fn reset_read(&mut self) {
            self.read_end = -1;
        }
        fn reset_write(&mut self) {
            self.write_pos = 0;
            self.write_end = -1;
        }

        fn flush(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            if !self.valid_write() || self.write_pos == self.write_end {
                self.reset_write();
                return Ok(());
            }

            let rewind = self.raw_offset() + (self.pos - self.write_pos);
            if rewind != 0 {
                self.raw_seek(-rewind, 1, vm)?;
                self.raw_pos = -rewind;
            }

            while self.write_pos < self.write_end {
                let n =
                    self.raw_write(None, self.write_pos as usize..self.write_end as usize, vm)?;
                let n = n.ok_or_else(|| {
                    vm.new_exception_msg(
                        vm.ctx.exceptions.blocking_io_error.clone(),
                        "write could not complete without blocking".to_owned(),
                    )
                })?;
                self.write_pos += n as Offset;
                self.raw_pos = self.write_pos;
                vm.check_signals()?;
            }

            self.reset_write();

            Ok(())
        }

        fn flush_rewind(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            self.flush(vm)?;
            if self.readable() {
                let res = self.raw_seek(-self.raw_offset(), 1, vm);
                self.reset_read();
                res?;
            }
            Ok(())
        }

        fn raw_seek(&mut self, pos: Offset, whence: i32, vm: &VirtualMachine) -> PyResult<Offset> {
            let ret = vm.call_method(self.check_init(vm)?, "seek", (pos, whence))?;
            let offset = get_offset(ret, vm)?;
            if offset < 0 {
                return Err(
                    vm.new_os_error(format!("Raw stream returned invalid position {}", offset))
                );
            }
            self.abs_pos = offset;
            Ok(offset)
        }

        fn seek(&mut self, target: Offset, whence: i32, vm: &VirtualMachine) -> PyResult<Offset> {
            if matches!(whence, 0 | 1) && self.readable() {
                let current = self.raw_tell_cache(vm)?;
                let available = self.readahead();
                if available > 0 {
                    let offset = if whence == 0 {
                        target - (current - self.raw_offset())
                    } else {
                        target
                    };
                    if offset >= -self.pos && offset <= available {
                        self.pos += offset;
                        return Ok(current - available + offset);
                    }
                }
            }
            // vm.invoke(&vm.get_attribute(raw, "seek")?, args)
            if self.writable() {
                self.flush(vm)?;
            }
            let target = if whence == 1 {
                target - self.raw_offset()
            } else {
                target
            };
            let res = self.raw_seek(target, whence, vm);
            self.raw_pos = -1;
            if res.is_ok() && self.readable() {
                self.reset_read();
            }
            res
        }

        fn raw_tell(&mut self, vm: &VirtualMachine) -> PyResult<Offset> {
            let ret = vm.call_method(self.check_init(vm)?, "tell", ())?;
            let offset = get_offset(ret, vm)?;
            if offset < 0 {
                return Err(
                    vm.new_os_error(format!("Raw stream returned invalid position {}", offset))
                );
            }
            self.abs_pos = offset;
            Ok(offset)
        }

        fn raw_tell_cache(&mut self, vm: &VirtualMachine) -> PyResult<Offset> {
            if self.abs_pos == -1 {
                self.raw_tell(vm)
            } else {
                Ok(self.abs_pos)
            }
        }

        /// None means non-blocking failed
        fn raw_write(
            &mut self,
            buf: Option<PyBufferRef>,
            buf_range: Range<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let len = buf_range.len();
            let res = if let Some(buf) = buf {
                let memobj = PyMemoryView::from_buffer_range(vm.ctx.none(), buf, buf_range, vm)?
                    .into_pyobject(vm);

                // TODO: loop if write() raises an interrupt
                vm.call_method(self.raw.as_ref().unwrap(), "write", (memobj,))?
            } else {
                let options = BufferOptions {
                    len,
                    ..Default::default()
                };
                // TODO: see if we can encapsulate this pattern in a function in memory.rs like
                // fn slice_as_memory<R>(s: &[u8], f: impl FnOnce(PyMemoryViewRef) -> R) -> R
                let writebuf = PyRc::new(BufferedRawBuffer {
                    data: std::mem::take(&mut self.buffer).into(),
                    range: buf_range,
                    options,
                });
                let memobj = PyMemoryView::from_buffer(
                    vm.ctx.none(),
                    PyBufferRef::new(writebuf.clone()),
                    vm,
                )?
                .into_ref(vm);

                // TODO: loop if write() raises an interrupt
                let res = vm.call_method(self.raw.as_ref().unwrap(), "write", (memobj.clone(),));

                memobj.released.store(true);
                self.buffer = std::mem::take(&mut writebuf.data.lock());

                res?
            };

            if vm.is_none(&res) {
                return Ok(None);
            }
            let n = isize::try_from_object(vm, res)?;
            if n < 0 || n as usize > len {
                return Err(vm.new_os_error(format!(
                    "raw write() returned invalid length {} (should have been between 0 and {})",
                    n, len
                )));
            }
            if self.abs_pos != -1 {
                self.abs_pos += n as Offset
            }
            Ok(Some(n as usize))
        }

        fn write(&mut self, obj: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.valid_read() && !self.valid_write() {
                self.pos = 0;
                self.raw_pos = 0;
            }
            let avail = self.buffer.len() - self.pos as usize;
            let buf_len;
            {
                let buf = obj.borrow_buf();
                buf_len = buf.len();
                if buf.len() <= avail {
                    self.buffer[self.pos as usize..][..buf.len()].copy_from_slice(&buf);
                    if !self.valid_write() || self.write_pos > self.pos {
                        self.write_pos = self.pos
                    }
                    self.adjust_position(self.pos + buf.len() as i64);
                    if self.pos > self.write_end {
                        self.write_end = self.pos
                    }
                    return Ok(buf.len());
                }
            }

            // TODO: something something check if error is BlockingIOError?
            let _ = self.flush(vm);

            let offset = self.raw_offset();
            if offset != 0 {
                self.raw_seek(-offset, 1, vm)?;
                self.raw_pos -= offset;
            }

            let mut remaining = buf_len;
            let mut written = 0;
            let rcbuf = obj.into_buffer().into_rcbuf();
            while remaining > self.buffer.len() {
                let res =
                    self.raw_write(Some(PyBufferRef::new(rcbuf.clone())), written..buf_len, vm)?;
                match res {
                    Some(n) => {
                        written += n;
                        if let Some(r) = remaining.checked_sub(n) {
                            remaining = r
                        } else {
                            break;
                        }
                        vm.check_signals()?;
                    }
                    None => {
                        // raw file is non-blocking
                        if remaining > self.buffer.len() {
                            // can't buffer everything, buffer what we can and error
                            let buf = rcbuf.as_contiguous().unwrap();
                            let buffer_len = self.buffer.len();
                            self.buffer.copy_from_slice(&buf[written..][..buffer_len]);
                            self.raw_pos = 0;
                            let buffer_size = self.buffer.len() as _;
                            self.adjust_position(buffer_size);
                            self.write_end = buffer_size;
                            // TODO: BlockingIOError(errno, msg, written)
                            // written += self.buffer.len();
                            return Err(vm.new_exception_msg(
                                vm.ctx.exceptions.blocking_io_error.clone(),
                                "write could not complete without blocking".to_owned(),
                            ));
                        } else {
                            break;
                        }
                    }
                }
            }
            if self.readable() {
                self.reset_read();
            }
            if remaining > 0 {
                let buf = rcbuf.as_contiguous().unwrap();
                self.buffer[..remaining].copy_from_slice(&buf[written..][..remaining]);
                written += remaining;
            }
            self.write_pos = 0;
            self.write_end = remaining as _;
            self.adjust_position(remaining as _);
            self.raw_pos = 0;

            Ok(written)
        }

        fn active_read_slice(&self) -> &[u8] {
            &self.buffer[self.pos as usize..][..self.readahead() as usize]
        }

        fn read_fast(&mut self, n: usize) -> Option<Vec<u8>> {
            let ret = self.active_read_slice().get(..n)?.to_vec();
            self.pos += n as Offset;
            Some(ret)
        }

        fn read_generic(&mut self, n: usize, vm: &VirtualMachine) -> PyResult<Option<Vec<u8>>> {
            if let Some(fast) = self.read_fast(n) {
                return Ok(Some(fast));
            }

            let current_size = self.readahead() as usize;

            let mut out = vec![0u8; n];
            let mut remaining = n;
            let mut written = 0;
            if current_size > 0 {
                let slice = self.active_read_slice();
                out[..slice.len()].copy_from_slice(slice);
                remaining -= current_size;
                written += current_size;
                self.pos += current_size as Offset;
            }
            if self.writable() {
                self.flush_rewind(vm)?;
            }
            self.reset_read();
            macro_rules! handle_opt_read {
                ($x:expr) => {
                    match ($x, written > 0) {
                        (Some(0), _) | (None, true) => {
                            out.truncate(written);
                            return Ok(Some(out));
                        }
                        (Some(r), _) => r,
                        (None, _) => return Ok(None),
                    }
                };
            }
            while remaining > 0 {
                // MINUS_LAST_BLOCK() in CPython
                let r = self.buffer.len() * (remaining / self.buffer.len());
                if r == 0 {
                    break;
                }
                let r = self.raw_read(Either::A(Some(&mut out)), written..written + r, vm)?;
                let r = handle_opt_read!(r);
                remaining -= r;
                written += r;
            }
            self.pos = 0;
            self.raw_pos = 0;
            self.read_end = 0;

            while remaining > 0 && (self.read_end as usize) < self.buffer.len() {
                let r = handle_opt_read!(self.fill_buffer(vm)?);
                if remaining > r {
                    out[written..][..r].copy_from_slice(&self.buffer[self.pos as usize..][..r]);
                    written += r;
                    self.pos += r as Offset;
                    remaining -= r;
                } else if remaining > 0 {
                    out[written..][..remaining]
                        .copy_from_slice(&self.buffer[self.pos as usize..][..remaining]);
                    written += remaining;
                    self.pos += remaining as Offset;
                    remaining = 0;
                }
                if remaining == 0 {
                    break;
                }
            }

            Ok(Some(out))
        }

        fn fill_buffer(&mut self, vm: &VirtualMachine) -> PyResult<Option<usize>> {
            let start = if self.valid_read() {
                self.read_end as usize
            } else {
                0
            };
            let buf_end = self.buffer.len();
            let res = self.raw_read(Either::A(None), start..buf_end, vm)?;
            if let Some(n) = res.filter(|n| *n > 0) {
                let new_start = (start + n) as Offset;
                self.read_end = new_start;
                self.raw_pos = new_start;
            }
            Ok(res)
        }

        fn raw_read(
            &mut self,
            v: Either<Option<&mut Vec<u8>>, PyBufferRef>,
            buf_range: Range<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let len = buf_range.len();
            let res = match v {
                Either::A(v) => {
                    let v = v.unwrap_or(&mut self.buffer);
                    let options = BufferOptions {
                        len,
                        readonly: false,
                        ..Default::default()
                    };
                    // TODO: see if we can encapsulate this pattern in a function in memory.rs like
                    // fn slice_as_memory<R>(s: &[u8], f: impl FnOnce(PyMemoryViewRef) -> R) -> R
                    let readbuf = PyRc::new(BufferedRawBuffer {
                        data: std::mem::take(v).into(),
                        range: buf_range,
                        options,
                    });
                    let memobj = PyMemoryView::from_buffer(
                        vm.ctx.none(),
                        PyBufferRef::new(readbuf.clone()),
                        vm,
                    )?
                    .into_ref(vm);

                    // TODO: loop if readinto() raises an interrupt
                    let res =
                        vm.call_method(self.raw.as_ref().unwrap(), "readinto", (memobj.clone(),));

                    memobj.released.store(true);
                    std::mem::swap(v, &mut readbuf.data.lock());

                    res?
                }
                Either::B(buf) => {
                    let memobj =
                        PyMemoryView::from_buffer_range(vm.ctx.none(), buf, buf_range, vm)?;
                    // TODO: loop if readinto() raises an interrupt
                    vm.call_method(self.raw.as_ref().unwrap(), "readinto", (memobj,))?
                }
            };

            if vm.is_none(&res) {
                return Ok(None);
            }
            let n = isize::try_from_object(vm, res)?;
            if n < 0 || n as usize > len {
                return Err(vm.new_os_error(format!(
                    "raw readinto() returned invalid length {} (should have been between 0 and {})",
                    n, len
                )));
            }
            if n > 0 && self.abs_pos != -1 {
                self.abs_pos += n as Offset
            }
            Ok(Some(n as usize))
        }

        fn read_all(&mut self, vm: &VirtualMachine) -> PyResult<Option<PyBytesRef>> {
            let buf = self.active_read_slice();
            let data = if buf.is_empty() {
                None
            } else {
                let b = buf.to_vec();
                self.pos += buf.len() as Offset;
                Some(b)
            };

            if self.writable() {
                self.flush_rewind(vm)?;
            }

            let readall = vm
                .get_method(self.raw.clone().unwrap(), "readall")
                .transpose()?;
            if let Some(readall) = readall {
                let res = vm.invoke(&readall, ())?;
                let res = <Option<PyBytesRef>>::try_from_object(vm, res)?;
                let ret = if let Some(mut data) = data {
                    if let Some(bytes) = res {
                        data.extend_from_slice(bytes.as_bytes());
                    }
                    Some(PyBytes::from(data).into_ref(vm))
                } else {
                    res
                };
                return Ok(ret);
            }

            let mut chunks = Vec::new();

            let mut read_size = 0;
            loop {
                let read_data = vm.call_method(self.raw.as_ref().unwrap(), "read", ())?;
                let read_data = <Option<PyBytesRef>>::try_from_object(vm, read_data)?;

                match read_data {
                    Some(b) if !b.as_bytes().is_empty() => {
                        let l = b.as_bytes().len();
                        read_size += l;
                        if self.abs_pos != -1 {
                            self.abs_pos += l as Offset;
                        }
                        chunks.push(b);
                    }
                    read_data => {
                        let ret = if data.is_none() && read_size == 0 {
                            read_data
                        } else {
                            let mut data = data.unwrap_or_default();
                            data.reserve(read_size);
                            for bytes in &chunks {
                                data.extend_from_slice(bytes.as_bytes())
                            }
                            Some(PyBytes::from(data).into_ref(vm))
                        };
                        break Ok(ret);
                    }
                }
            }
        }

        fn adjust_position(&mut self, new_pos: Offset) {
            self.pos = new_pos;
            if self.valid_read() && self.read_end < self.pos {
                self.read_end = self.pos
            }
        }

        fn peek(&mut self, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let have = self.readahead();
            let slice = if have > 0 {
                &self.buffer[self.pos as usize..][..have as usize]
            } else {
                self.reset_read();
                let r = self.fill_buffer(vm)?.unwrap_or(0);
                self.pos = 0;
                &self.buffer[..r]
            };
            Ok(slice.to_vec())
        }

        fn readinto_generic(
            &mut self,
            buf: PyBufferRef,
            readinto1: bool,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let mut written = 0;
            let n = self.readahead();
            let buf_len;
            {
                let mut b = buf.as_contiguous_mut().unwrap();
                buf_len = b.len();
                if n > 0 {
                    if n as usize >= b.len() {
                        b.copy_from_slice(&self.buffer[self.pos as usize..][..buf_len]);
                        self.pos += buf_len as Offset;
                        return Ok(Some(buf_len));
                    }
                    b[..n as usize]
                        .copy_from_slice(&self.buffer[self.pos as usize..][..n as usize]);
                    self.pos += n;
                    written = n as usize;
                }
            }
            if self.writable() {
                let _ = self.flush_rewind(vm)?;
            }
            self.reset_read();
            self.pos = 0;

            let rcbuf = buf.into_rcbuf();
            let mut remaining = buf_len - written;
            while remaining > 0 {
                let n = if remaining as usize > self.buffer.len() {
                    let buf = PyBufferRef::new(rcbuf.clone());
                    self.raw_read(Either::B(buf), written..written + remaining, vm)?
                } else if !(readinto1 && written != 0) {
                    let n = self.fill_buffer(vm)?;
                    if let Some(n) = n.filter(|&n| n > 0) {
                        let n = std::cmp::min(n, remaining);
                        rcbuf.as_contiguous_mut().unwrap()[written..][..n]
                            .copy_from_slice(&self.buffer[self.pos as usize..][..n]);
                        self.pos += n as Offset;
                        written += n;
                        remaining -= n;
                        continue;
                    }
                    n
                } else {
                    break;
                };
                let n = match n {
                    Some(0) => break,
                    None if written > 0 => break,
                    None => return Ok(None),
                    Some(n) => n,
                };

                if readinto1 {
                    written += n;
                    break;
                }
                written += n;
                remaining -= n;
            }

            Ok(Some(written))
        }
    }

    // this is a bit fancier than what CPython does, but in CPython if you store
    // the memoryobj for the buffer until after the BufferedIO is destroyed, you
    // can get a use-after-free, so this is a bit safe
    #[derive(Debug)]
    struct BufferedRawBuffer {
        data: PyMutex<Vec<u8>>,
        range: Range<usize>,
        options: BufferOptions,
    }
    impl PyBuffer for PyRc<BufferedRawBuffer> {
        fn get_options(&self) -> &BufferOptions {
            &self.options
        }

        fn obj_bytes(&self) -> BorrowedValue<[u8]> {
            BorrowedValue::map(self.data.lock().into(), |data| &data[self.range.clone()])
        }

        fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
            BorrowedValueMut::map(self.data.lock().into(), |data| {
                &mut data[self.range.clone()]
            })
        }

        fn release(&self) {}
    }

    pub fn get_offset(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Offset> {
        use std::convert::TryInto;
        let int = vm.to_index(&obj)?;
        int.as_bigint().try_into().map_err(|_| {
            vm.new_value_error(format!(
                "cannot fit '{}' into an offset-sized integer",
                obj.class().name
            ))
        })
    }

    pub fn repr_fileobj_name(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<PyStrRef>> {
        let name = match vm.get_attribute(obj.clone(), "name") {
            Ok(name) => Some(name),
            Err(e)
                if e.isinstance(&vm.ctx.exceptions.attribute_error)
                    || e.isinstance(&vm.ctx.exceptions.value_error) =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        match name {
            Some(name) => {
                if let Some(_guard) = ReprGuard::enter(vm, obj) {
                    vm.to_repr(&name).map(Some)
                } else {
                    Err(vm.new_runtime_error(format!(
                        "reentrant call inside {}.__repr__",
                        obj.class().tp_name()
                    )))
                }
            }
            None => Ok(None),
        }
    }

    #[pyimpl]
    trait BufferedMixin: PyValue {
        const READABLE: bool;
        const WRITABLE: bool;
        const SEEKABLE: bool = false;
        fn data(&self) -> &PyThreadMutex<BufferedData>;
        fn lock(&self, vm: &VirtualMachine) -> PyResult<PyThreadMutexGuard<BufferedData>> {
            self.data()
                .lock()
                .ok_or_else(|| vm.new_runtime_error("reentrant call inside buffered io".to_owned()))
        }

        #[pymethod(magic)]
        fn init(
            &self,
            raw: PyObjectRef,
            BufferSize { buffer_size }: BufferSize,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let mut data = self.lock(vm)?;
            data.raw = None;
            data.flags.remove(BufferedFlags::DETACHED);

            let buffer_size = match buffer_size {
                OptionalArg::Present(i) if i <= 0 => {
                    return Err(
                        vm.new_value_error("buffer size must be strictly positive".to_owned())
                    );
                }
                OptionalArg::Present(i) => i as usize,
                OptionalArg::Missing => DEFAULT_BUFFER_SIZE,
            };

            if Self::SEEKABLE {
                check_seekable(&raw, vm)?;
            }
            if Self::READABLE {
                data.flags.insert(BufferedFlags::READABLE);
                check_readable(&raw, vm)?;
            }
            if Self::WRITABLE {
                data.flags.insert(BufferedFlags::WRITABLE);
                check_writable(&raw, vm)?;
            }

            data.buffer = vec![0; buffer_size];

            if Self::READABLE {
                data.reset_read();
            }
            if Self::WRITABLE {
                data.reset_write();
            }
            if Self::SEEKABLE {
                data.pos = 0;
            }

            data.raw = Some(raw);

            Ok(())
        }
        #[pymethod]
        fn seek(
            &self,
            target: PyObjectRef,
            whence: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<Offset> {
            let whence = whence.unwrap_or(0);
            if !validate_whence(whence) {
                return Err(vm.new_value_error(format!("whence value {} unsupported", whence)));
            }
            let mut data = self.lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "seek of closed file", vm)?;
            check_seekable(raw, vm)?;
            let target = get_offset(target, vm)?;
            data.seek(target, whence, vm)
        }
        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult<Offset> {
            let mut data = self.lock(vm)?;
            Ok(data.raw_tell(vm)? - data.raw_offset())
        }
        #[pymethod]
        fn truncate(
            zelf: PyRef<Self>,
            pos: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let pos = pos.flatten().into_pyobject(vm);
            let mut data = zelf.lock(vm)?;
            data.check_init(vm)?;
            if data.writable() {
                data.flush_rewind(vm)?;
            }
            let res = vm.call_method(data.raw.as_ref().unwrap(), "truncate", (pos,))?;
            let _ = data.raw_tell(vm);
            Ok(res)
        }
        #[pymethod]
        fn detach(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            vm.call_method(zelf.as_object(), "flush", ())?;
            let mut data = zelf.lock(vm)?;
            data.flags.insert(BufferedFlags::DETACHED);
            data.raw
                .take()
                .ok_or_else(|| vm.new_value_error("raw stream has been detached".to_owned()))
        }
        #[pymethod]
        fn seekable(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "seekable", ())
        }
        #[pyproperty]
        fn raw(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            Ok(self.lock(vm)?.raw.clone())
        }
        #[pyproperty]
        fn closed(&self, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(self.lock(vm)?.check_init(vm)?.clone(), "closed")
        }
        #[pyproperty]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(self.lock(vm)?.check_init(vm)?.clone(), "name")
        }
        #[pyproperty]
        fn mode(&self, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(self.lock(vm)?.check_init(vm)?.clone(), "mode")
        }
        #[pymethod]
        fn fileno(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "fileno", ())
        }
        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "isatty", ())
        }

        #[pymethod(magic)]
        fn repr(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let name_repr = repr_fileobj_name(&zelf, vm)?;
            let cls = zelf.class();
            let tp_name = cls.tp_name();
            let repr = if let Some(name_repr) = name_repr {
                format!("<{} name={}>", tp_name, name_repr)
            } else {
                format!("<{}>", tp_name)
            };
            Ok(repr)
        }

        fn close_strict(&self, vm: &VirtualMachine) -> PyResult {
            let mut data = self.lock(vm)?;
            let raw = data.check_init(vm)?;
            if file_closed(raw, vm)? {
                return Ok(vm.ctx.none());
            }
            let flush_res = data.flush(vm);
            let close_res = vm.call_method(data.raw.as_ref().unwrap(), "close", ());
            exceptions::chain(flush_res, close_res)
        }

        #[pymethod]
        fn close(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            {
                let data = zelf.lock(vm)?;
                let raw = data.check_init(vm)?;
                if file_closed(raw, vm)? {
                    return Ok(vm.ctx.none());
                }
            }
            let flush_res = vm.call_method(zelf.as_object(), "flush", ()).map(drop);
            let data = zelf.lock(vm)?;
            let raw = data.raw.as_ref().unwrap();
            let close_res = vm.call_method(raw, "close", ());
            exceptions::chain(flush_res, close_res)
        }

        #[pymethod]
        fn readable(&self) -> bool {
            Self::READABLE
        }
        #[pymethod]
        fn writable(&self) -> bool {
            Self::WRITABLE
        }

        // TODO: this should be the default for an equivalent of _PyObject_GetState
        #[pymethod(magic)]
        fn reduce(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(format!("cannot pickle '{}' object", zelf.class().name)))
        }
    }

    #[pyimpl]
    trait BufferedReadable: PyValue {
        type Reader: BufferedMixin;
        fn reader(&self) -> &Self::Reader;
        #[pymethod]
        fn read(&self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Option<PyBytesRef>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            let n = size.size.unwrap_or(-1);
            if n < -1 {
                return Err(vm.new_value_error("read length must be non-negative or -1".to_owned()));
            }
            ensure_unclosed(raw, "read of closed file", vm)?;
            match n.to_usize() {
                Some(n) => data
                    .read_generic(n, vm)
                    .map(|x| x.map(|b| PyBytes::from(b).into_ref(vm))),
                None => data.read_all(vm),
            }
        }
        #[pymethod]
        fn peek(&self, _size: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "peek of closed file", vm)?;

            if data.writable() {
                let _ = data.flush_rewind(vm);
            }
            data.peek(vm)
        }
        #[pymethod]
        fn read1(&self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "read of closed file", vm)?;
            let n = size.to_usize().unwrap_or_else(|| data.buffer.len());
            if n == 0 {
                return Ok(Vec::new());
            }
            let have = data.readahead();
            if have > 0 {
                let n = std::cmp::min(have as usize, n);
                return Ok(data.read_fast(n).unwrap());
            }
            let mut v = vec![0; n];
            data.reset_read();
            let r = data
                .raw_read(Either::A(Some(&mut v)), 0..n, vm)?
                .unwrap_or(0);
            v.truncate(r);
            v.shrink_to_fit();
            Ok(v)
        }
        #[pymethod]
        fn readinto(&self, buf: ArgMemoryBuffer, vm: &VirtualMachine) -> PyResult<Option<usize>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "readinto of closed file", vm)?;
            data.readinto_generic(buf.into_buffer(), false, vm)
        }
        #[pymethod]
        fn readinto1(&self, buf: ArgMemoryBuffer, vm: &VirtualMachine) -> PyResult<Option<usize>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "readinto of closed file", vm)?;
            data.readinto_generic(buf.into_buffer(), true, vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "BufferedReader", base = "_BufferedIOBase")]
    #[derive(Debug, Default)]
    struct BufferedReader {
        data: PyThreadMutex<BufferedData>,
    }
    impl PyValue for BufferedReader {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }
    impl BufferedMixin for BufferedReader {
        const READABLE: bool = true;
        const WRITABLE: bool = false;
        fn data(&self) -> &PyThreadMutex<BufferedData> {
            &self.data
        }
    }
    impl BufferedReadable for BufferedReader {
        type Reader = Self;
        fn reader(&self) -> &Self::Reader {
            self
        }
    }

    #[pyimpl(with(BufferedMixin, BufferedReadable), flags(BASETYPE, HAS_DICT))]
    impl BufferedReader {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::default().into_ref_with_type(vm, cls)
        }
    }

    #[pyimpl]
    trait BufferedWritable: PyValue {
        type Writer: BufferedMixin;
        fn writer(&self) -> &Self::Writer;
        #[pymethod]
        fn write(&self, obj: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let mut data = self.writer().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "write to closed file", vm)?;

            data.write(obj, vm)
        }
        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut data = self.writer().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "flush of closed file", vm)?;
            data.flush_rewind(vm)
        }
    }

    #[pyattr]
    #[pyclass(name = "BufferedWriter", base = "_BufferedIOBase")]
    #[derive(Debug, Default)]
    struct BufferedWriter {
        data: PyThreadMutex<BufferedData>,
    }
    impl PyValue for BufferedWriter {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }
    impl BufferedMixin for BufferedWriter {
        const READABLE: bool = false;
        const WRITABLE: bool = true;
        fn data(&self) -> &PyThreadMutex<BufferedData> {
            &self.data
        }
    }
    impl BufferedWritable for BufferedWriter {
        type Writer = Self;
        fn writer(&self) -> &Self::Writer {
            self
        }
    }

    #[pyimpl(with(BufferedMixin, BufferedWritable), flags(BASETYPE, HAS_DICT))]
    impl BufferedWriter {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::default().into_ref_with_type(vm, cls)
        }
    }

    #[pyattr]
    #[pyclass(name = "BufferedRandom", base = "_BufferedIOBase")]
    #[derive(Debug, Default)]
    struct BufferedRandom {
        data: PyThreadMutex<BufferedData>,
    }
    impl PyValue for BufferedRandom {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }
    impl BufferedMixin for BufferedRandom {
        const READABLE: bool = true;
        const WRITABLE: bool = true;
        const SEEKABLE: bool = true;
        fn data(&self) -> &PyThreadMutex<BufferedData> {
            &self.data
        }
    }
    impl BufferedReadable for BufferedRandom {
        type Reader = Self;
        fn reader(&self) -> &Self::Reader {
            self
        }
    }
    impl BufferedWritable for BufferedRandom {
        type Writer = Self;
        fn writer(&self) -> &Self::Writer {
            self
        }
    }

    #[pyimpl(
        with(BufferedMixin, BufferedReadable, BufferedWritable),
        flags(BASETYPE, HAS_DICT)
    )]
    impl BufferedRandom {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::default().into_ref_with_type(vm, cls)
        }
    }

    #[pyattr]
    #[pyclass(name = "BufferedRWPair", base = "_BufferedIOBase")]
    #[derive(Debug, Default)]
    struct BufferedRWPair {
        read: BufferedReader,
        write: BufferedWriter,
    }
    impl PyValue for BufferedRWPair {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }
    impl BufferedReadable for BufferedRWPair {
        type Reader = BufferedReader;
        fn reader(&self) -> &Self::Reader {
            &self.read
        }
    }
    impl BufferedWritable for BufferedRWPair {
        type Writer = BufferedWriter;
        fn writer(&self) -> &Self::Writer {
            &self.write
        }
    }
    #[pyimpl(with(BufferedReadable, BufferedWritable), flags(BASETYPE, HAS_DICT))]
    impl BufferedRWPair {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::default().into_ref_with_type(vm, cls)
        }
        #[pymethod(magic)]
        fn init(
            &self,
            reader: PyObjectRef,
            writer: PyObjectRef,
            buffer_size: BufferSize,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            self.read.init(reader, buffer_size.clone(), vm)?;
            self.write.init(writer, buffer_size, vm)?;
            Ok(())
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<()> {
            self.write.flush(vm)
        }

        #[pymethod]
        fn readable(&self) -> bool {
            true
        }
        #[pymethod]
        fn writable(&self) -> bool {
            true
        }

        #[pyproperty]
        fn closed(&self, vm: &VirtualMachine) -> PyResult {
            self.write.closed(vm)
        }

        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult {
            // read.isatty() or write.isatty()
            let res = self.read.isatty(vm)?;
            if pybool::boolval(vm, res.clone())? {
                Ok(res)
            } else {
                self.write.isatty(vm)
            }
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult {
            let write_res = self.write.close_strict(vm).map(drop);
            let read_res = self.read.close_strict(vm);
            exceptions::chain(write_res, read_res)
        }
    }

    #[derive(FromArgs)]
    struct TextIOWrapperArgs {
        #[pyarg(any)]
        buffer: PyObjectRef,
        #[pyarg(any, default)]
        encoding: Option<PyStrRef>,
        #[pyarg(any, default)]
        errors: Option<PyStrRef>,
        #[pyarg(any, default)]
        newline: Newlines,
        #[pyarg(any, default = "false")]
        line_buffering: bool,
        #[pyarg(any, default = "false")]
        write_through: bool,
    }

    #[derive(Debug, Copy, Clone)]
    enum Newlines {
        Universal,
        Passthrough,
        Lf,
        Cr,
        Crlf,
    }

    impl Default for Newlines {
        #[inline]
        fn default() -> Self {
            Newlines::Universal
        }
    }

    impl Newlines {
        /// returns position where the new line starts if found, otherwise position at which to
        /// continue the search after more is read into the buffer
        fn find_newline(&self, s: &str) -> Result<usize, usize> {
            let len = s.len();
            match self {
                Newlines::Universal | Newlines::Lf => s.find('\n').map(|p| p + 1).ok_or(len),
                Newlines::Passthrough => {
                    let bytes = s.as_bytes();
                    memchr::memchr2(b'\n', b'\r', bytes)
                        .map(|p| {
                            let nl_len =
                                if bytes[p] == b'\r' && bytes.get(p + 1).copied() == Some(b'\n') {
                                    2
                                } else {
                                    1
                                };
                            p + nl_len
                        })
                        .ok_or(len)
                }
                Newlines::Cr => s.find('\n').map(|p| p + 1).ok_or(len),
                Newlines::Crlf => {
                    // s[searched..] == remaining
                    let mut searched = 0;
                    let mut remaining = s.as_bytes();
                    loop {
                        match memchr::memchr(b'\r', remaining) {
                            Some(p) => match remaining.get(p + 1) {
                                Some(&ch_after_cr) => {
                                    let pos_after = p + 2;
                                    if ch_after_cr == b'\n' {
                                        break Ok(searched + pos_after);
                                    } else {
                                        searched += pos_after;
                                        remaining = &remaining[pos_after..];
                                        continue;
                                    }
                                }
                                None => break Err(searched + p),
                            },
                            None => break Err(len),
                        }
                    }
                }
            }
        }
    }

    impl TryFromObject for Newlines {
        fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
            let nl = if vm.is_none(&obj) {
                Self::Universal
            } else {
                let s = obj.downcast::<PyStr>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "newline argument must be str or None, not {}",
                        obj.class().name
                    ))
                })?;
                match s.as_str() {
                    "" => Self::Passthrough,
                    "\n" => Self::Lf,
                    "\r" => Self::Cr,
                    "\r\n" => Self::Crlf,
                    _ => return Err(vm.new_value_error(format!("illegal newline value: {}", s))),
                }
            };
            Ok(nl)
        }
    }

    /// A length of or index into a UTF-8 string, measured in both chars and bytes
    #[derive(Debug, Default, Copy, Clone)]
    struct Utf8size {
        bytes: usize,
        chars: usize,
    }
    impl Utf8size {
        fn len_pystr(s: &PyStr) -> Self {
            Utf8size {
                bytes: s.byte_len(),
                chars: s.char_len(),
            }
        }

        fn len_str(s: &str) -> Self {
            Utf8size {
                bytes: s.len(),
                chars: s.chars().count(),
            }
        }
    }
    impl std::ops::Add for Utf8size {
        type Output = Self;
        #[inline]
        fn add(mut self, rhs: Self) -> Self {
            self += rhs;
            self
        }
    }
    impl std::ops::AddAssign for Utf8size {
        #[inline]
        fn add_assign(&mut self, rhs: Self) {
            self.bytes += rhs.bytes;
            self.chars += rhs.chars;
        }
    }
    impl std::ops::Sub for Utf8size {
        type Output = Self;
        #[inline]
        fn sub(mut self, rhs: Self) -> Self {
            self -= rhs;
            self
        }
    }
    impl std::ops::SubAssign for Utf8size {
        #[inline]
        fn sub_assign(&mut self, rhs: Self) {
            self.bytes -= rhs.bytes;
            self.chars -= rhs.chars;
        }
    }

    // TODO: implement legit fast-paths for other encodings
    type EncodeFunc = fn(PyStrRef) -> PendingWrite;
    fn textio_encode_utf8(s: PyStrRef) -> PendingWrite {
        PendingWrite::Utf8(s)
    }

    #[derive(Debug)]
    struct TextIOData {
        buffer: PyObjectRef,
        encoder: Option<(PyObjectRef, Option<EncodeFunc>)>,
        decoder: Option<PyObjectRef>,
        encoding: PyStrRef,
        errors: PyStrRef,
        newline: Newlines,
        line_buffering: bool,
        write_through: bool,
        chunk_size: usize,
        seekable: bool,
        has_read1: bool,
        // these are more state than configuration
        pending: PendingWrites,
        telling: bool,
        snapshot: Option<(i32, PyBytesRef)>,
        decoded_chars: Option<PyStrRef>,
        // number of characters we've consumed from decoded_chars
        decoded_chars_used: Utf8size,
        b2cratio: f64,
    }

    #[derive(Debug, Default)]
    struct PendingWrites {
        num_bytes: usize,
        data: PendingWritesData,
    }

    #[derive(Debug)]
    enum PendingWritesData {
        None,
        One(PendingWrite),
        Many(Vec<PendingWrite>),
    }

    #[derive(Debug)]
    enum PendingWrite {
        Utf8(PyStrRef),
        Bytes(PyBytesRef),
    }

    impl PendingWrite {
        fn as_bytes(&self) -> &[u8] {
            match self {
                Self::Utf8(s) => s.as_str().as_bytes(),
                Self::Bytes(b) => b.as_bytes(),
            }
        }
    }

    impl Default for PendingWritesData {
        fn default() -> Self {
            PendingWritesData::None
        }
    }

    impl PendingWrites {
        fn push(&mut self, write: PendingWrite) {
            self.num_bytes += write.as_bytes().len();
            self.data = match std::mem::take(&mut self.data) {
                PendingWritesData::None => PendingWritesData::One(write),
                PendingWritesData::One(write1) => PendingWritesData::Many(vec![write1, write]),
                PendingWritesData::Many(mut v) => {
                    v.push(write);
                    PendingWritesData::Many(v)
                }
            }
        }
        fn take(&mut self, vm: &VirtualMachine) -> PyBytesRef {
            let PendingWrites { num_bytes, data } = std::mem::take(self);
            if let PendingWritesData::One(PendingWrite::Bytes(b)) = data {
                return b;
            }
            let writes_iter = match data {
                PendingWritesData::None => itertools::Either::Left(vec![].into_iter()),
                PendingWritesData::One(write) => itertools::Either::Right(std::iter::once(write)),
                PendingWritesData::Many(writes) => itertools::Either::Left(writes.into_iter()),
            };
            let mut buf = Vec::with_capacity(num_bytes);
            writes_iter.for_each(|chunk| buf.extend_from_slice(chunk.as_bytes()));
            PyBytes::from(buf).into_ref(vm)
        }
    }

    #[derive(Default, Debug)]
    struct TextIOCookie {
        start_pos: Offset,
        dec_flags: i32,
        bytes_to_feed: i32,
        chars_to_skip: i32,
        need_eof: bool,
        // chars_to_skip but utf8 bytes
        bytes_to_skip: i32,
    }

    impl TextIOCookie {
        const START_POS_OFF: usize = 0;
        const DEC_FLAGS_OFF: usize = Self::START_POS_OFF + std::mem::size_of::<Offset>();
        const BYTES_TO_FEED_OFF: usize = Self::DEC_FLAGS_OFF + 4;
        const CHARS_TO_SKIP_OFF: usize = Self::BYTES_TO_FEED_OFF + 4;
        const NEED_EOF_OFF: usize = Self::CHARS_TO_SKIP_OFF + 4;
        const BYTES_TO_SKIP_OFF: usize = Self::NEED_EOF_OFF + 1;
        const BYTE_LEN: usize = Self::BYTES_TO_SKIP_OFF + 4;
        fn parse(cookie: &num_bigint::BigInt) -> Option<Self> {
            use std::convert::TryInto;
            let (_, mut buf) = cookie.to_bytes_le();
            if buf.len() > Self::BYTE_LEN {
                return None;
            }
            buf.resize(Self::BYTE_LEN, 0);
            let buf: &[u8; Self::BYTE_LEN] = buf.as_slice().try_into().unwrap();
            macro_rules! get_field {
                ($t:ty, $off:ident) => {{
                    <$t>::from_ne_bytes(
                        buf[Self::$off..][..std::mem::size_of::<$t>()]
                            .try_into()
                            .unwrap(),
                    )
                }};
            }
            Some(TextIOCookie {
                start_pos: get_field!(Offset, START_POS_OFF),
                dec_flags: get_field!(i32, DEC_FLAGS_OFF),
                bytes_to_feed: get_field!(i32, BYTES_TO_FEED_OFF),
                chars_to_skip: get_field!(i32, CHARS_TO_SKIP_OFF),
                need_eof: get_field!(u8, NEED_EOF_OFF) != 0,
                bytes_to_skip: get_field!(i32, BYTES_TO_SKIP_OFF),
            })
        }
        fn build(&self) -> num_bigint::BigInt {
            let mut buf = [0; Self::BYTE_LEN];
            macro_rules! set_field {
                ($field:expr, $off:ident) => {{
                    let field = $field;
                    buf[Self::$off..][..std::mem::size_of_val(&field)]
                        .copy_from_slice(&field.to_ne_bytes())
                }};
            }
            set_field!(self.start_pos, START_POS_OFF);
            set_field!(self.dec_flags, DEC_FLAGS_OFF);
            set_field!(self.bytes_to_feed, BYTES_TO_FEED_OFF);
            set_field!(self.chars_to_skip, CHARS_TO_SKIP_OFF);
            set_field!(self.need_eof as u8, NEED_EOF_OFF);
            set_field!(self.bytes_to_skip, BYTES_TO_SKIP_OFF);
            num_bigint::BigUint::from_bytes_le(&buf).into()
        }
        fn set_decoder_state(&self, decoder: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            if self.start_pos == 0 && self.dec_flags == 0 {
                vm.call_method(decoder, "reset", ())?;
            } else {
                vm.call_method(
                    decoder,
                    "setstate",
                    ((vm.ctx.new_bytes(vec![]), self.dec_flags),),
                )?;
            }
            Ok(())
        }
        fn num_to_skip(&self) -> Utf8size {
            Utf8size {
                bytes: self.bytes_to_skip as usize,
                chars: self.chars_to_skip as usize,
            }
        }
        fn set_num_to_skip(&mut self, num: Utf8size) {
            self.bytes_to_skip = num.bytes as i32;
            self.chars_to_skip = num.chars as i32;
        }
    }

    #[pyattr]
    #[pyclass(name = "TextIOWrapper", base = "_TextIOBase")]
    #[derive(Debug, Default)]
    struct TextIOWrapper {
        data: PyThreadMutex<Option<TextIOData>>,
    }
    impl PyValue for TextIOWrapper {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(flags(BASETYPE))]
    impl TextIOWrapper {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self::default().into_ref_with_type(vm, cls)
        }

        fn lock_opt(
            &self,
            vm: &VirtualMachine,
        ) -> PyResult<PyThreadMutexGuard<Option<TextIOData>>> {
            self.data
                .lock()
                .ok_or_else(|| vm.new_runtime_error("reentrant call inside textio".to_owned()))
        }
        fn lock(&self, vm: &VirtualMachine) -> PyResult<PyMappedThreadMutexGuard<TextIOData>> {
            let lock = self.lock_opt(vm)?;
            PyThreadMutexGuard::try_map(lock, |x| x.as_mut())
                .map_err(|_| vm.new_value_error("I/O operation on uninitialized object".to_owned()))
        }

        #[pymethod(magic)]
        fn init(&self, args: TextIOWrapperArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mut data = self.lock_opt(vm)?;
            *data = None;

            let encoding = match args.encoding {
                Some(enc) => enc,
                None => {
                    // TODO: try os.device_encoding(fileno) and then locale.getpreferredencoding()
                    PyStr::from(crate::codecs::DEFAULT_ENCODING).into_ref(vm)
                }
            };

            let errors = args
                .errors
                .unwrap_or_else(|| PyStr::from("strict").into_ref(vm));

            let buffer = args.buffer;

            let has_read1 = vm.get_attribute_opt(buffer.clone(), "read1")?.is_some();
            let seekable = pybool::boolval(vm, vm.call_method(&buffer, "seekable", ())?)?;

            let codec = vm.state.codec_registry.lookup(encoding.as_str(), vm)?;

            let encoder = if pybool::boolval(vm, vm.call_method(&buffer, "writable", ())?)? {
                let incremental_encoder =
                    codec.get_incremental_encoder(Some(errors.clone()), vm)?;
                let encoding_name = vm.get_attribute_opt(incremental_encoder.clone(), "name")?;
                let encodefunc = encoding_name.and_then(|name| {
                    name.payload::<PyStr>()
                        .and_then(|name| match name.as_str() {
                            "utf-8" => Some(textio_encode_utf8 as EncodeFunc),
                            _ => None,
                        })
                });
                Some((incremental_encoder, encodefunc))
            } else {
                None
            };

            let decoder = if pybool::boolval(vm, vm.call_method(&buffer, "readable", ())?)? {
                let incremental_decoder =
                    codec.get_incremental_decoder(Some(errors.clone()), vm)?;
                // TODO: wrap in IncrementalNewlineDecoder if newlines == Universal | Passthrough
                Some(incremental_decoder)
            } else {
                None
            };

            *data = Some(TextIOData {
                buffer,
                encoder,
                decoder,
                encoding,
                errors,
                newline: args.newline,
                line_buffering: args.line_buffering,
                write_through: args.write_through,
                chunk_size: 8192,
                seekable,
                has_read1,

                pending: PendingWrites::default(),
                telling: seekable,
                snapshot: None,
                decoded_chars: None,
                decoded_chars_used: Utf8size::default(),
                b2cratio: 0.0,
            });

            Ok(())
        }

        #[pymethod]
        fn seekable(&self, vm: &VirtualMachine) -> PyResult {
            let textio = self.lock(vm)?;
            vm.call_method(&textio.buffer, "seekable", ())
        }
        #[pymethod]
        fn readable(&self, vm: &VirtualMachine) -> PyResult {
            let textio = self.lock(vm)?;
            vm.call_method(&textio.buffer, "readable", ())
        }
        #[pymethod]
        fn writable(&self, vm: &VirtualMachine) -> PyResult {
            let textio = self.lock(vm)?;
            vm.call_method(&textio.buffer, "writable", ())
        }

        #[pyproperty(name = "_CHUNK_SIZE")]
        fn chunksize(&self, vm: &VirtualMachine) -> PyResult<usize> {
            Ok(self.lock(vm)?.chunk_size)
        }

        #[pyproperty(setter, name = "_CHUNK_SIZE")]
        fn set_chunksize(&self, chunk_size: usize, vm: &VirtualMachine) -> PyResult<()> {
            let mut textio = self.lock(vm)?;
            textio.chunk_size = chunk_size;
            Ok(())
        }

        #[pymethod]
        fn seek(
            zelf: PyRef<Self>,
            cookie: PyObjectRef,
            how: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let how = how.unwrap_or(0);

            let reset_encoder = |encoder, start_of_stream| {
                if start_of_stream {
                    vm.call_method(encoder, "reset", ())
                } else {
                    vm.call_method(encoder, "setstate", (0,))
                }
            };

            let textio = zelf.lock(vm)?;

            if !textio.seekable {
                return Err(new_unsupported_operation(
                    vm,
                    "underlying stream is not seekable".to_owned(),
                ));
            }

            let cookie = match how {
                // SEEK_SET
                0 => cookie,
                // SEEK_CUR
                1 => {
                    if vm.bool_eq(&cookie, &vm.ctx.new_int(0))? {
                        vm.call_method(&textio.buffer, "tell", ())?
                    } else {
                        return Err(new_unsupported_operation(
                            vm,
                            "can't do nonzero cur-relative seeks".to_owned(),
                        ));
                    }
                }
                // SEEK_END
                2 => {
                    if vm.bool_eq(&cookie, &vm.ctx.new_int(0))? {
                        drop(textio);
                        vm.call_method(zelf.as_object(), "flush", ())?;
                        let mut textio = zelf.lock(vm)?;
                        textio.set_decoded_chars(None);
                        textio.snapshot = None;
                        if let Some(decoder) = &textio.decoder {
                            vm.call_method(decoder, "reset", ())?;
                        }
                        let res = vm.call_method(&textio.buffer, "seek", (0, 2))?;
                        if let Some((encoder, _)) = &textio.encoder {
                            let start_of_stream = vm.bool_eq(&res, &vm.ctx.new_int(0))?;
                            reset_encoder(encoder, start_of_stream)?;
                        }
                        return Ok(res);
                    } else {
                        return Err(new_unsupported_operation(
                            vm,
                            "can't do nonzero end-relative seeks".to_owned(),
                        ));
                    }
                }
                _ => {
                    return Err(vm
                        .new_value_error(format!("invalid whence ({}, should be 0, 1 or 2)", how)))
                }
            };
            use crate::slots::PyComparisonOp;
            if vm.bool_cmp(&cookie, &vm.ctx.new_int(0), PyComparisonOp::Lt)? {
                return Err(
                    vm.new_value_error(format!("negative seek position {}", vm.to_repr(&cookie)?))
                );
            }
            drop(textio);
            vm.call_method(zelf.as_object(), "flush", ())?;
            let cookie_obj = crate::builtins::PyIntRef::try_from_object(vm, cookie)?;
            let cookie = TextIOCookie::parse(cookie_obj.as_bigint())
                .ok_or_else(|| vm.new_value_error("invalid cookie".to_owned()))?;
            let mut textio = zelf.lock(vm)?;
            vm.call_method(&textio.buffer, "seek", (cookie.start_pos,))?;
            textio.set_decoded_chars(None);
            textio.snapshot = None;
            if let Some(decoder) = &textio.decoder {
                cookie.set_decoder_state(decoder, vm)?;
            }
            if cookie.chars_to_skip != 0 {
                let TextIOData {
                    ref decoder,
                    ref buffer,
                    ref mut snapshot,
                    ..
                } = *textio;
                let decoder = decoder
                    .as_ref()
                    .ok_or_else(|| vm.new_value_error("invalid cookie".to_owned()))?;
                let input_chunk = vm.call_method(buffer, "read", (cookie.bytes_to_feed,))?;
                let input_chunk: PyBytesRef = input_chunk.downcast().map_err(|obj| {
                    vm.new_type_error(format!(
                        "underlying read() should have returned a bytes object, not '{}'",
                        obj.class().name
                    ))
                })?;
                *snapshot = Some((cookie.dec_flags, input_chunk.clone()));
                let decoded = vm.call_method(decoder, "decode", (input_chunk, cookie.need_eof))?;
                let decoded = check_decoded(decoded, vm)?;
                let pos_is_valid = decoded
                    .as_str()
                    .is_char_boundary(cookie.bytes_to_skip as usize);
                textio.set_decoded_chars(Some(decoded));
                if !pos_is_valid {
                    return Err(vm.new_os_error("can't restore logical file position".to_owned()));
                }
                textio.decoded_chars_used = cookie.num_to_skip();
            } else {
                textio.snapshot = Some((cookie.dec_flags, PyBytes::from(vec![]).into_ref(vm)))
            }
            if let Some((encoder, _)) = &textio.encoder {
                let start_of_stream = cookie.start_pos == 0 && cookie.dec_flags == 0;
                reset_encoder(encoder, start_of_stream)?;
            }
            Ok(cookie_obj.into_object())
        }

        #[pymethod]
        fn tell(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            let mut textio = zelf.lock(vm)?;
            if !textio.seekable {
                return Err(new_unsupported_operation(
                    vm,
                    "underlying stream is not seekable".to_owned(),
                ));
            }
            if !textio.telling {
                return Err(vm.new_os_error("telling position disabled by next() call".to_owned()));
            }
            textio.write_pending(vm)?;
            drop(textio);
            vm.call_method(zelf.as_object(), "flush", ())?;
            let textio = zelf.lock(vm)?;
            let pos = vm.call_method(&textio.buffer, "tell", ())?;
            let (decoder, (dec_flags, next_input)) = match (&textio.decoder, &textio.snapshot) {
                (Some(d), Some(s)) => (d, s),
                _ => return Ok(pos),
            };
            let pos = Offset::try_from_object(vm, pos)?;
            let mut cookie = TextIOCookie {
                start_pos: pos - next_input.len() as Offset,
                dec_flags: *dec_flags,
                ..Default::default()
            };
            if textio.decoded_chars_used.bytes == 0 {
                return Ok(cookie.build().into_pyobject(vm));
            }
            let decoder_getstate = || {
                let state = vm.call_method(decoder, "getstate", ())?;
                parse_decoder_state(state, vm)
            };
            let decoder_decode = |b: &[u8]| {
                let decoded = vm.call_method(decoder, "decode", (vm.ctx.new_bytes(b.to_vec()),))?;
                let decoded = check_decoded(decoded, vm)?;
                Ok(Utf8size::len_pystr(&decoded))
            };
            let saved_state = vm.call_method(decoder, "getstate", ())?;
            let mut num_to_skip = textio.decoded_chars_used;
            let mut skip_bytes = (textio.b2cratio * num_to_skip.chars as f64) as isize;
            let mut skip_back = 1;
            while skip_bytes > 0 {
                cookie.set_decoder_state(decoder, vm)?;
                let input = &next_input.as_bytes()[..skip_bytes as usize];
                let ndecoded = decoder_decode(input)?;
                if ndecoded.chars <= num_to_skip.chars {
                    let (dec_buffer, dec_flags) = decoder_getstate()?;
                    if dec_buffer.is_empty() {
                        cookie.dec_flags = dec_flags;
                        num_to_skip -= ndecoded;
                        break;
                    }
                    skip_bytes -= dec_buffer.len() as isize;
                    skip_back = 1;
                } else {
                    skip_bytes -= skip_back;
                    skip_back *= 2;
                }
            }
            if skip_bytes <= 0 {
                skip_bytes = 0;
                cookie.set_decoder_state(decoder, vm)?;
            }
            let skip_bytes = skip_bytes as usize;

            cookie.start_pos += skip_bytes as Offset;
            cookie.set_num_to_skip(num_to_skip);

            if num_to_skip.chars != 0 {
                let mut ndecoded = Utf8size::default();
                let mut input = next_input.as_bytes();
                input = &input[skip_bytes..];
                while !input.is_empty() {
                    let (byte1, rest) = input.split_at(1);
                    let n = decoder_decode(byte1)?;
                    ndecoded += n;
                    cookie.bytes_to_feed += 1;
                    let (dec_buffer, dec_flags) = decoder_getstate()?;
                    if dec_buffer.is_empty() && ndecoded.chars < num_to_skip.chars {
                        cookie.start_pos += cookie.bytes_to_feed as Offset;
                        num_to_skip -= ndecoded;
                        cookie.dec_flags = dec_flags;
                        cookie.bytes_to_feed = 0;
                        ndecoded = Utf8size::default();
                    }
                    if ndecoded.chars >= num_to_skip.chars {
                        break;
                    }
                    input = rest;
                }
                if input.is_empty() {
                    let decoded =
                        vm.call_method(decoder, "decode", (vm.ctx.new_bytes(vec![]), true))?;
                    let decoded = check_decoded(decoded, vm)?;
                    let final_decoded_chars = ndecoded.chars + decoded.char_len();
                    cookie.need_eof = true;
                    if final_decoded_chars < num_to_skip.chars {
                        return Err(
                            vm.new_os_error("can't reconstruct logical file position".to_owned())
                        );
                    }
                }
            }
            vm.call_method(decoder, "setstate", (saved_state,))?;
            cookie.set_num_to_skip(num_to_skip);
            Ok(cookie.build().into_pyobject(vm))
        }

        #[pyproperty]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.get_attribute(buffer, "name")
        }
        #[pyproperty]
        fn encoding(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Ok(self.lock(vm)?.encoding.clone())
        }
        #[pyproperty]
        fn errors(&self, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Ok(self.lock(vm)?.errors.clone())
        }

        #[pymethod]
        fn fileno(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.call_method(&buffer, "fileno", ())
        }

        #[pymethod]
        fn read(&self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let mut textio = self.lock(vm)?;
            textio.check_closed(vm)?;
            let decoder = textio
                .decoder
                .clone()
                .ok_or_else(|| new_unsupported_operation(vm, "not readable".to_owned()))?;

            textio.write_pending(vm)?;

            let s = if let Some(mut remaining) = size.to_usize() {
                let mut chunks = Vec::new();
                let mut chunks_bytes = 0;
                loop {
                    if let Some((s, char_len)) = textio.get_decoded_chars(remaining, vm) {
                        chunks_bytes += s.byte_len();
                        chunks.push(s);
                        remaining = remaining.saturating_sub(char_len);
                    }
                    if remaining == 0 {
                        break;
                    }
                    let eof = textio.read_chunk(remaining, vm)?;
                    if eof {
                        break;
                    }
                }
                if chunks.is_empty() {
                    PyStr::from("").into_ref(vm)
                } else if chunks.len() == 1 {
                    chunks.pop().unwrap()
                } else {
                    let mut ret = String::with_capacity(chunks_bytes);
                    for chunk in chunks {
                        ret.push_str(chunk.as_str())
                    }
                    PyStr::from(ret).into_ref(vm)
                }
            } else {
                let bytes = vm.call_method(&textio.buffer, "read", ())?;
                let decoded = vm.call_method(&decoder, "decode", (bytes, true))?;
                let decoded = check_decoded(decoded, vm)?;
                let ret = textio.take_decoded_chars(Some(decoded), vm);
                textio.snapshot = None;
                ret
            };
            Ok(s)
        }

        #[pymethod]
        fn write(&self, obj: PyStrRef, vm: &VirtualMachine) -> PyResult<usize> {
            let mut textio = self.lock(vm)?;
            textio.check_closed(vm)?;

            let (encoder, encodefunc) = textio
                .encoder
                .as_ref()
                .ok_or_else(|| new_unsupported_operation(vm, "not writable".to_owned()))?;

            let char_len = obj.char_len();

            let data = obj.as_str();

            let replace_nl = match textio.newline {
                Newlines::Cr => Some("\r"),
                Newlines::Crlf => Some("\r\n"),
                _ => None,
            };
            let has_lf = if replace_nl.is_some() || textio.line_buffering {
                data.contains('\n')
            } else {
                false
            };
            let flush = textio.line_buffering && (has_lf || data.contains('\r'));
            let chunk = if let Some(replace_nl) = replace_nl {
                if has_lf {
                    PyStr::from(data.replace('\n', replace_nl)).into_ref(vm)
                } else {
                    obj
                }
            } else {
                obj
            };
            let chunk = if let Some(encodefunc) = *encodefunc {
                encodefunc(chunk)
            } else {
                let b = vm.call_method(encoder, "encode", (chunk.clone(),))?;
                b.downcast::<PyBytes>()
                    .map(PendingWrite::Bytes)
                    .or_else(|obj| {
                        // TODO: not sure if encode() returning the str it was passed is officially
                        // supported or just a quirk of how the CPython code is written
                        if obj.is(&chunk) {
                            Ok(PendingWrite::Utf8(chunk))
                        } else {
                            Err(vm.new_type_error(format!(
                                "encoder should return a bytes object, not '{}'",
                                obj.class().name
                            )))
                        }
                    })?
            };
            if textio.pending.num_bytes + chunk.as_bytes().len() > textio.chunk_size {
                textio.write_pending(vm)?;
            }
            textio.pending.push(chunk);
            if flush || textio.write_through || textio.pending.num_bytes >= textio.chunk_size {
                textio.write_pending(vm)?;
            }
            if flush {
                let _ = vm.call_method(&textio.buffer, "flush", ());
            }

            Ok(char_len)
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult {
            let mut textio = self.lock(vm)?;
            textio.check_closed(vm)?;
            textio.telling = textio.seekable;
            textio.write_pending(vm)?;
            vm.call_method(&textio.buffer, "flush", ())
        }

        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult {
            let textio = self.lock(vm)?;
            textio.check_closed(vm)?;
            vm.call_method(&textio.buffer, "isatty", ())
        }

        #[pymethod]
        fn readline(&self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            let limit = size.to_usize();

            let mut textio = self.lock(vm)?;
            check_closed(&textio.buffer, vm)?;

            textio.write_pending(vm)?;

            #[derive(Clone)]
            struct SlicedStr(PyStrRef, Range<usize>);
            impl SlicedStr {
                #[inline]
                fn byte_len(&self) -> usize {
                    self.1.len()
                }
                #[inline]
                fn char_len(&self) -> usize {
                    if self.is_full_slice() {
                        self.0.char_len()
                    } else {
                        self.slice().chars().count()
                    }
                }
                #[inline]
                fn is_full_slice(&self) -> bool {
                    self.1.len() >= self.0.byte_len()
                }
                #[inline]
                fn slice(&self) -> &str {
                    &self.0.as_str()[self.1.clone()]
                }
                #[inline]
                fn slice_pystr(self, vm: &VirtualMachine) -> PyStrRef {
                    if self.is_full_slice() {
                        self.0
                    } else {
                        // TODO: try to use Arc::get_mut() on the str?
                        PyStr::from(self.slice()).into_ref(vm)
                    }
                }
                fn utf8_len(&self) -> Utf8size {
                    Utf8size {
                        bytes: self.byte_len(),
                        chars: self.char_len(),
                    }
                }
            }

            let mut start;
            let mut endpos;
            let mut offset_to_buffer;
            let mut chunked = Utf8size::default();
            let mut remaining: Option<SlicedStr> = None;
            let mut chunks = Vec::new();

            let cur_line = 'outer: loop {
                let decoded_chars = loop {
                    match textio.decoded_chars.as_ref() {
                        Some(s) if !s.is_empty() => break s,
                        _ => {}
                    }
                    let eof = textio.read_chunk(0, vm)?;
                    if eof {
                        textio.set_decoded_chars(None);
                        textio.snapshot = None;
                        start = Utf8size::default();
                        endpos = Utf8size::default();
                        offset_to_buffer = Utf8size::default();
                        break 'outer None;
                    }
                };
                let line = match remaining.take() {
                    None => {
                        start = textio.decoded_chars_used;
                        offset_to_buffer = Utf8size::default();
                        decoded_chars.clone()
                    }
                    Some(remaining) => {
                        assert_eq!(textio.decoded_chars_used.bytes, 0);
                        offset_to_buffer = remaining.utf8_len();
                        let decoded_chars = decoded_chars.as_str();
                        let line = if remaining.is_full_slice() {
                            let mut line = remaining.0;
                            line.concat_in_place(decoded_chars, vm);
                            line
                        } else {
                            let remaining = remaining.slice();
                            let mut s =
                                String::with_capacity(remaining.len() + decoded_chars.len());
                            s.push_str(remaining);
                            s.push_str(decoded_chars);
                            PyStr::from(s).into_ref(vm)
                        };
                        start = Utf8size::default();
                        line
                    }
                };
                let line_from_start = &line.as_str()[start.bytes..];
                let nl_res = textio.newline.find_newline(line_from_start);
                match nl_res {
                    Ok(p) | Err(p) => {
                        endpos = start + Utf8size::len_str(&line_from_start[..p]);
                        if let Some(limit) = limit {
                            // original CPython logic: endpos = start + limit - chunked
                            if chunked.chars + endpos.chars >= limit {
                                endpos = start
                                    + Utf8size {
                                        chars: limit - chunked.chars,
                                        bytes: crate::common::str::char_range_end(
                                            line_from_start,
                                            limit - chunked.chars,
                                        )
                                        .unwrap(),
                                    };
                                break Some(line);
                            }
                        }
                    }
                }
                if nl_res.is_ok() {
                    break Some(line);
                }
                if endpos.bytes > start.bytes {
                    let chunk = SlicedStr(line.clone(), start.bytes..endpos.bytes);
                    chunked += chunk.utf8_len();
                    chunks.push(chunk);
                }
                let line_len = line.byte_len();
                if endpos.bytes < line_len {
                    remaining = Some(SlicedStr(line, endpos.bytes..line_len));
                }
                textio.set_decoded_chars(None);
            };

            let cur_line = cur_line.map(|line| {
                textio.decoded_chars_used = endpos - offset_to_buffer;
                SlicedStr(line, start.bytes..endpos.bytes)
            });
            // don't need to care about chunked.chars anymore
            let mut chunked = chunked.bytes;
            if let Some(remaining) = remaining {
                chunked += remaining.byte_len();
                chunks.push(remaining);
            }
            let line = if !chunks.is_empty() {
                if let Some(cur_line) = cur_line {
                    chunked += cur_line.byte_len();
                    chunks.push(cur_line);
                }
                let mut s = String::with_capacity(chunked);
                for chunk in chunks {
                    s.push_str(chunk.slice())
                }
                PyStr::from(s).into_ref(vm)
            } else if let Some(cur_line) = cur_line {
                cur_line.slice_pystr(vm)
            } else {
                PyStr::from("").into_ref(vm)
            };
            Ok(line)
        }

        #[pymethod]
        fn close(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let buffer = zelf.lock(vm)?.buffer.clone();
            if file_closed(&buffer, vm)? {
                return Ok(());
            }
            let flush_res = vm.call_method(zelf.as_object(), "flush", ()).map(drop);
            let close_res = vm.call_method(&buffer, "close", ()).map(drop);
            exceptions::chain(flush_res, close_res)
        }
        #[pyproperty]
        fn closed(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.get_attribute(buffer, "closed")
        }
        #[pyproperty]
        fn buffer(&self, vm: &VirtualMachine) -> PyResult {
            Ok(self.lock(vm)?.buffer.clone())
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(format!("cannot pickle '{}' object", zelf.class().name)))
        }
    }

    fn parse_decoder_state(state: PyObjectRef, vm: &VirtualMachine) -> PyResult<(PyBytesRef, i32)> {
        use crate::builtins::{int, PyTuple};
        let state_err = || vm.new_type_error("illegal decoder state".to_owned());
        let state = state.downcast::<PyTuple>().map_err(|_| state_err())?;
        match state.as_slice() {
            [buf, flags] => {
                let buf = buf.clone().downcast::<PyBytes>().map_err(|obj| {
                    vm.new_type_error(format!(
                        "illegal decoder state: the first item should be a bytes object, not '{}'",
                        obj.class().name
                    ))
                })?;
                let flags = flags.payload::<int::PyInt>().ok_or_else(state_err)?;
                let flags = int::try_to_primitive(flags.as_bigint(), vm)?;
                Ok((buf, flags))
            }
            _ => Err(state_err()),
        }
    }

    impl TextIOData {
        fn write_pending(&mut self, vm: &VirtualMachine) -> PyResult<()> {
            if self.pending.num_bytes == 0 {
                return Ok(());
            }
            let data = self.pending.take(vm);
            vm.call_method(&self.buffer, "write", (data,))?;
            Ok(())
        }
        /// returns true on EOF
        fn read_chunk(&mut self, size_hint: usize, vm: &VirtualMachine) -> PyResult<bool> {
            let decoder = self
                .decoder
                .as_ref()
                .ok_or_else(|| new_unsupported_operation(vm, "not readable".to_owned()))?;

            let dec_state = if self.telling {
                let state = vm.call_method(decoder, "getstate", ())?;
                Some(parse_decoder_state(state, vm)?)
            } else {
                None
            };

            let method = if self.has_read1 { "read1" } else { "read" };
            let size_hint = if size_hint > 0 {
                (self.b2cratio.max(1.0) * size_hint as f64) as usize
            } else {
                size_hint
            };
            let chunk_size = std::cmp::max(self.chunk_size, size_hint);
            let input_chunk = vm.call_method(&self.buffer, method, (chunk_size,))?;

            let buf = ArgBytesLike::new(vm, &input_chunk).map_err(|_| {
                vm.new_type_error(format!(
                    "underlying {}() should have returned a bytes-like object, not '{}'",
                    method,
                    input_chunk.class().name
                ))
            })?;
            let nbytes = buf.borrow_buf().len();
            let eof = nbytes == 0;
            let decoded = vm.call_method(decoder, "decode", (input_chunk, eof))?;
            let decoded = check_decoded(decoded, vm)?;

            let char_len = decoded.char_len();
            self.b2cratio = if char_len > 0 {
                nbytes as f64 / char_len as f64
            } else {
                0.0
            };
            let eof = if char_len > 0 { false } else { eof };
            self.set_decoded_chars(Some(decoded));

            if let Some((dec_buffer, dec_flags)) = dec_state {
                // TODO: inplace append to bytes when refcount == 1
                let mut next_input = dec_buffer.as_bytes().to_vec();
                next_input.extend_from_slice(&*buf.borrow_buf());
                self.snapshot = Some((dec_flags, PyBytes::from(next_input).into_ref(vm)));
            }

            Ok(eof)
        }

        fn check_closed(&self, vm: &VirtualMachine) -> PyResult<()> {
            check_closed(&self.buffer, vm)
        }

        /// returns str, str.char_len() (it might not be cached in the str yet but we calculate it
        /// anyway in this method)
        fn get_decoded_chars(
            &mut self,
            n: usize,
            vm: &VirtualMachine,
        ) -> Option<(PyStrRef, usize)> {
            if n == 0 {
                return None;
            }
            let decoded_chars = self.decoded_chars.as_ref()?;
            let avail = &decoded_chars.as_str()[self.decoded_chars_used.bytes..];
            if avail.is_empty() {
                return None;
            }
            let avail_chars = decoded_chars.char_len() - self.decoded_chars_used.chars;
            let (chars, chars_used) = if n >= avail_chars {
                if self.decoded_chars_used.bytes == 0 {
                    (decoded_chars.clone(), avail_chars)
                } else {
                    (PyStr::from(avail).into_ref(vm), avail_chars)
                }
            } else {
                let s = crate::common::str::get_chars(avail, 0..n);
                (PyStr::from(s).into_ref(vm), n)
            };
            self.decoded_chars_used += Utf8size {
                bytes: chars.byte_len(),
                chars: chars_used,
            };
            Some((chars, chars_used))
        }
        fn set_decoded_chars(&mut self, s: Option<PyStrRef>) {
            self.decoded_chars = s;
            self.decoded_chars_used = Utf8size::default();
        }
        fn take_decoded_chars(
            &mut self,
            append: Option<PyStrRef>,
            vm: &VirtualMachine,
        ) -> PyStrRef {
            let empty_str = || PyStr::from("").into_ref(vm);
            let chars_pos = std::mem::take(&mut self.decoded_chars_used).bytes;
            let decoded_chars = match std::mem::take(&mut self.decoded_chars) {
                None => return append.unwrap_or_else(empty_str),
                Some(s) if s.is_empty() => return append.unwrap_or_else(empty_str),
                Some(s) => s,
            };
            let append_len = append.as_ref().map_or(0, |s| s.byte_len());
            if append_len == 0 && chars_pos == 0 {
                return decoded_chars;
            }
            // TODO: in-place editing of `str` when refcount == 1
            let decoded_chars_unused = &decoded_chars.as_str()[chars_pos..];
            let mut s = String::with_capacity(decoded_chars_unused.len() + append_len);
            s.push_str(decoded_chars_unused);
            if let Some(append) = append {
                s.push_str(append.as_str())
            }
            PyStr::from(s).into_ref(vm)
        }
    }

    #[derive(FromArgs)]
    struct StringIOArgs {
        #[pyarg(any, default)]
        #[allow(dead_code)]
        // TODO: use this
        newline: Newlines,
    }

    #[pyattr]
    #[pyclass(name = "StringIO", base = "_TextIOBase")]
    #[derive(Debug)]
    struct StringIO {
        buffer: PyRwLock<BufferedIO>,
        closed: AtomicCell<bool>,
    }

    type StringIORef = PyRef<StringIO>;

    impl PyValue for StringIO {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(flags(BASETYPE, HAS_DICT), with(PyRef))]
    impl StringIO {
        fn buffer(&self, vm: &VirtualMachine) -> PyResult<PyRwLockWriteGuard<'_, BufferedIO>> {
            if !self.closed.load() {
                Ok(self.buffer.write())
            } else {
                Err(io_closed_error(vm))
            }
        }

        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            object: OptionalOption<PyStrRef>,
            _args: StringIOArgs,
            vm: &VirtualMachine,
        ) -> PyResult<StringIORef> {
            let raw_bytes = object
                .flatten()
                .map_or_else(Vec::new, |v| v.as_str().as_bytes().to_vec());

            StringIO {
                buffer: PyRwLock::new(BufferedIO::new(Cursor::new(raw_bytes))),
                closed: AtomicCell::new(false),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn readable(&self) -> bool {
            true
        }
        #[pymethod]
        fn writable(&self) -> bool {
            true
        }
        #[pymethod]
        fn seekable(&self) -> bool {
            true
        }

        #[pyproperty]
        fn closed(&self) -> bool {
            self.closed.load()
        }

        #[pymethod]
        fn close(&self) {
            self.closed.store(true);
        }
    }

    #[pyimpl]
    impl StringIORef {
        //write string to underlying vector
        #[pymethod]
        fn write(self, data: PyStrRef, vm: &VirtualMachine) -> PyResult {
            let bytes = data.as_str().as_bytes();

            match self.buffer(vm)?.write(bytes) {
                Some(value) => Ok(vm.ctx.new_int(value)),
                None => Err(vm.new_type_error("Error Writing String".to_owned())),
            }
        }

        //return the entire contents of the underlying
        #[pymethod]
        fn getvalue(self, vm: &VirtualMachine) -> PyResult {
            match String::from_utf8(self.buffer(vm)?.getvalue()) {
                Ok(result) => Ok(vm.ctx.new_str(result)),
                Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
            }
        }

        //skip to the jth position
        #[pymethod]
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

        //Read k bytes from the object and return.
        //If k is undefined || k == -1, then we read all bytes until the end of the file.
        //This also increments the stream position by the value of k
        #[pymethod]
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

        #[pymethod]
        fn tell(self, vm: &VirtualMachine) -> PyResult<u64> {
            Ok(self.buffer(vm)?.tell())
        }

        #[pymethod]
        fn readline(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<String> {
            // TODO size should correspond to the number of characters, at the moments its the number of
            // bytes.
            match String::from_utf8(self.buffer(vm)?.readline(size.to_usize(), vm)?) {
                Ok(value) => Ok(value),
                Err(_) => Err(vm.new_value_error("Error Retrieving Value".to_owned())),
            }
        }

        #[pymethod]
        fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<usize> {
            let mut buffer = self.buffer(vm)?;
            let pos = pos.try_usize(vm)?;
            Ok(buffer.truncate(pos))
        }
    }

    #[pyattr]
    #[pyclass(name = "BytesIO", base = "_BufferedIOBase")]
    #[derive(Debug)]
    struct BytesIO {
        buffer: PyRwLock<BufferedIO>,
        closed: AtomicCell<bool>,
        exports: AtomicCell<usize>,
    }

    type BytesIORef = PyRef<BytesIO>;

    impl PyValue for BytesIO {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(flags(BASETYPE, HAS_DICT), with(PyRef))]
    impl BytesIO {
        fn buffer(&self, vm: &VirtualMachine) -> PyResult<PyRwLockWriteGuard<'_, BufferedIO>> {
            if !self.closed.load() {
                Ok(self.buffer.write())
            } else {
                Err(io_closed_error(vm))
            }
        }

        #[pyslot]
        fn tp_new(
            cls: PyTypeRef,
            object: OptionalArg<Option<PyBytesRef>>,
            vm: &VirtualMachine,
        ) -> PyResult<BytesIORef> {
            let raw_bytes = object
                .flatten()
                .map_or_else(Vec::new, |input| input.as_bytes().to_vec());

            BytesIO {
                buffer: PyRwLock::new(BufferedIO::new(Cursor::new(raw_bytes))),
                closed: AtomicCell::new(false),
                exports: AtomicCell::new(0),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn readable(&self) -> bool {
            true
        }
        #[pymethod]
        fn writable(&self) -> bool {
            true
        }
        #[pymethod]
        fn seekable(&self) -> bool {
            true
        }
    }

    #[pyimpl]
    impl BytesIORef {
        #[pymethod]
        fn write(self, data: ArgBytesLike, vm: &VirtualMachine) -> PyResult<u64> {
            let mut buffer = self.try_resizable(vm)?;
            match data.with_ref(|b| buffer.write(b)) {
                Some(value) => Ok(value),
                None => Err(vm.new_type_error("Error Writing Bytes".to_owned())),
            }
        }

        //Retrieves the entire bytes object value from the underlying buffer
        #[pymethod]
        fn getvalue(self, vm: &VirtualMachine) -> PyResult {
            Ok(vm.ctx.new_bytes(self.buffer(vm)?.getvalue()))
        }

        //Takes an integer k (bytes) and returns them from the underlying buffer
        //If k is undefined || k == -1, then we read all bytes until the end of the file.
        //This also increments the stream position by the value of k
        #[pymethod]
        #[pymethod(name = "read1")]
        fn read(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            let buf = self
                .buffer(vm)?
                .read(size.to_usize())
                .unwrap_or_else(Vec::new);
            Ok(buf)
        }

        #[pymethod]
        fn readinto(self, obj: ArgMemoryBuffer, vm: &VirtualMachine) -> PyResult<usize> {
            let mut buf = self.buffer(vm)?;
            let ret = buf
                .cursor
                .read(&mut *obj.borrow_buf_mut())
                .map_err(|_| vm.new_value_error("Error readinto from Take".to_owned()))?;

            Ok(ret)
        }

        //skip to the jth position
        #[pymethod]
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

        #[pymethod]
        fn seekable(self) -> bool {
            true
        }

        #[pymethod]
        fn tell(self, vm: &VirtualMachine) -> PyResult<u64> {
            Ok(self.buffer(vm)?.tell())
        }

        #[pymethod]
        fn readline(self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            self.buffer(vm)?.readline(size.to_usize(), vm)
        }

        #[pymethod]
        fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<usize> {
            if self.closed.load() {
                return Err(io_closed_error(vm));
            }
            let mut buffer = self.try_resizable(vm)?;
            let pos = pos.try_usize(vm)?;
            Ok(buffer.truncate(pos))
        }

        #[pyproperty]
        fn closed(self) -> bool {
            self.closed.load()
        }

        #[pymethod]
        fn close(self, vm: &VirtualMachine) -> PyResult<()> {
            let _ = self.try_resizable(vm)?;
            self.closed.store(true);
            Ok(())
        }

        #[pymethod]
        fn getbuffer(self, vm: &VirtualMachine) -> PyResult<PyMemoryView> {
            self.exports.fetch_add(1);
            let buffer = PyBufferRef::new(BytesIOBuffer {
                bytesio: self.clone(),
                options: BufferOptions {
                    readonly: false,
                    len: self.buffer.read().cursor.get_ref().len(),
                    ..Default::default()
                },
            });
            let view = PyMemoryView::from_buffer(self.into_object(), buffer, vm)?;
            Ok(view)
        }
    }

    #[derive(Debug)]
    struct BytesIOBuffer {
        bytesio: BytesIORef,
        options: BufferOptions,
    }

    impl PyBuffer for BytesIOBuffer {
        fn get_options(&self) -> &BufferOptions {
            &self.options
        }

        fn obj_bytes(&self) -> BorrowedValue<[u8]> {
            PyRwLockReadGuard::map(self.bytesio.buffer.read(), |x| {
                x.cursor.get_ref().as_slice()
            })
            .into()
        }

        fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
            PyRwLockWriteGuard::map(self.bytesio.buffer.write(), |x| {
                x.cursor.get_mut().as_mut_slice()
            })
            .into()
        }

        fn release(&self) {
            self.bytesio.exports.fetch_sub(1);
        }
    }

    impl<'a> ResizeGuard<'a> for BytesIO {
        type Resizable = PyRwLockWriteGuard<'a, BufferedIO>;

        fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
            if self.exports.load() == 0 {
                Ok(self.buffer.write())
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            }
        }
    }

    #[repr(u8)]
    enum FileMode {
        Read = b'r',
        Write = b'w',
        Exclusive = b'x',
        Append = b'a',
    }
    #[repr(u8)]
    enum EncodeMode {
        Text = b't',
        Bytes = b'b',
    }
    struct Mode {
        file: FileMode,
        encode: EncodeMode,
        plus: bool,
    }
    impl std::str::FromStr for Mode {
        type Err = ParseModeError;
        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let mut file = None;
            let mut encode = None;
            let mut plus = false;
            macro_rules! set_mode {
                ($var:ident, $mode:path, $err:ident) => {{
                    match $var {
                        Some($mode) => return Err(ParseModeError::InvalidMode),
                        Some(_) => return Err(ParseModeError::$err),
                        None => $var = Some($mode),
                    }
                }};
            }

            for ch in s.chars() {
                match ch {
                    '+' => {
                        if plus {
                            return Err(ParseModeError::InvalidMode);
                        }
                        plus = true
                    }
                    't' => set_mode!(encode, EncodeMode::Text, MultipleEncode),
                    'b' => set_mode!(encode, EncodeMode::Bytes, MultipleEncode),
                    'r' => set_mode!(file, FileMode::Read, MultipleFile),
                    'a' => set_mode!(file, FileMode::Append, MultipleFile),
                    'w' => set_mode!(file, FileMode::Write, MultipleFile),
                    'x' => set_mode!(file, FileMode::Exclusive, MultipleFile),
                    _ => return Err(ParseModeError::InvalidMode),
                }
            }

            let file = file.ok_or(ParseModeError::NoFile)?;
            let encode = encode.unwrap_or(EncodeMode::Text);

            Ok(Mode { file, encode, plus })
        }
    }
    impl Mode {
        fn rawmode(&self) -> &'static str {
            match (&self.file, self.plus) {
                (FileMode::Read, true) => "rb+",
                (FileMode::Read, false) => "rb",
                (FileMode::Write, true) => "wb+",
                (FileMode::Write, false) => "wb",
                (FileMode::Exclusive, true) => "xb+",
                (FileMode::Exclusive, false) => "xb",
                (FileMode::Append, true) => "ab+",
                (FileMode::Append, false) => "ab",
            }
        }
    }
    enum ParseModeError {
        InvalidMode,
        MultipleFile,
        MultipleEncode,
        NoFile,
    }
    impl ParseModeError {
        fn error_msg(&self, mode_string: &str) -> String {
            match self {
                ParseModeError::InvalidMode => format!("invalid mode: '{}'", mode_string),
                ParseModeError::MultipleFile => {
                    "must have exactly one of create/read/write/append mode".to_owned()
                }
                ParseModeError::MultipleEncode => {
                    "can't have text and binary mode at once".to_owned()
                }
                ParseModeError::NoFile => {
                    "Must have exactly one of create/read/write/append mode and at most one plus"
                        .to_owned()
                }
            }
        }
    }

    #[derive(FromArgs)]
    struct IoOpenArgs {
        #[pyarg(any)]
        file: PyObjectRef,
        #[pyarg(any, optional)]
        mode: OptionalArg<PyStrRef>,
        #[pyarg(flatten)]
        opts: OpenArgs,
    }
    #[pyfunction]
    fn open(args: IoOpenArgs, vm: &VirtualMachine) -> PyResult {
        io_open(
            args.file,
            args.mode.as_ref().into_option().map(|s| s.as_str()),
            args.opts,
            vm,
        )
    }

    #[pyfunction]
    fn open_code(file: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // TODO: lifecycle hooks or something?
        io_open(file, Some("rb"), OpenArgs::default(), vm)
    }

    #[derive(FromArgs)]
    pub struct OpenArgs {
        #[pyarg(any, default = "-1")]
        buffering: isize,
        #[pyarg(any, default)]
        encoding: Option<PyStrRef>,
        #[pyarg(any, default)]
        errors: Option<PyStrRef>,
        #[pyarg(any, default)]
        newline: Option<PyStrRef>,
        #[pyarg(any, default = "true")]
        closefd: bool,
        #[pyarg(any, default)]
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
        let mode_string = mode.unwrap_or("r");
        let mode = mode_string
            .parse::<Mode>()
            .map_err(|e| vm.new_value_error(e.error_msg(mode_string)))?;

        if let EncodeMode::Bytes = mode.encode {
            let msg = if opts.encoding.is_some() {
                Some("binary mode doesn't take an encoding argument")
            } else if opts.errors.is_some() {
                Some("binary mode doesn't take an errors argument")
            } else if opts.newline.is_some() {
                Some("binary mode doesn't take a newline argument")
            } else {
                None
            };
            if let Some(msg) = msg {
                return Err(vm.new_value_error(msg.to_owned()));
            }
        }

        // check file descriptor validity
        #[cfg(unix)]
        if let Ok(PathOrFd::Fd(fd)) = PathOrFd::try_from_object(vm, file.clone()) {
            nix::fcntl::fcntl(fd, nix::fcntl::F_GETFD).map_err(|_| errno_err(vm))?;
        }

        // Construct a FileIO (subclass of RawIOBase)
        // This is subsequently consumed by a Buffered Class.
        let file_io_class = {
            cfg_if::cfg_if! {
                if #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))] {
                    Some(super::fileio::FileIO::static_type())
                } else {
                    None
                }
            }
        };
        let file_io_class: &PyTypeRef = file_io_class.ok_or_else(|| {
            new_unsupported_operation(
                vm,
                "Couldn't get FileIO, io.open likely isn't supported on your platform".to_owned(),
            )
        })?;
        let raw = vm.invoke(
            file_io_class.as_object(),
            (file, mode.rawmode(), opts.closefd, opts.opener),
        )?;

        let isatty = opts.buffering < 0 && {
            let atty = vm.call_method(&raw, "isatty", ())?;
            bool::try_from_object(vm, atty)?
        };

        let line_buffering = opts.buffering == 1 || isatty;

        let buffering = if opts.buffering < 0 || opts.buffering == 1 {
            DEFAULT_BUFFER_SIZE
        } else {
            opts.buffering as usize
        };

        if buffering == 0 {
            let ret = match mode.encode {
                EncodeMode::Text => {
                    Err(vm.new_value_error("can't have unbuffered text I/O".to_owned()))
                }
                EncodeMode::Bytes => Ok(raw),
            };
            return ret;
        }

        let cls = if mode.plus {
            BufferedRandom::static_type()
        } else if let FileMode::Read = mode.file {
            BufferedReader::static_type()
        } else {
            BufferedWriter::static_type()
        };
        let buffered = vm.invoke(cls.as_object(), (raw, buffering))?;

        match mode.encode {
            EncodeMode::Text => {
                let tio = TextIOWrapper::static_type();
                let wrapper = vm.invoke(
                    tio.as_object(),
                    (
                        buffered,
                        opts.encoding,
                        opts.errors,
                        opts.newline,
                        line_buffering,
                    ),
                )?;
                vm.set_attr(&wrapper, "mode", vm.ctx.new_str(mode_string))?;
                Ok(wrapper)
            }
            EncodeMode::Bytes => Ok(buffered),
        }
    }

    rustpython_common::static_cell! {
        pub(super) static UNSUPPORTED_OPERATION: PyTypeRef;
    }

    pub(super) fn make_unsupportedop(ctx: &PyContext) -> PyTypeRef {
        pytype::new(
            ctx.types.type_type.clone(),
            "UnsupportedOperation",
            ctx.exceptions.os_error.clone(),
            vec![
                ctx.exceptions.os_error.clone(),
                ctx.exceptions.value_error.clone(),
            ],
            Default::default(),
            Default::default(),
        )
        .unwrap()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

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
}

// disable FileIO on WASM
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[pymodule]
mod fileio {
    use super::Offset;
    use super::_io::*;
    use crate::builtins::{PyStr, PyStrRef, PyTypeRef};
    use crate::byteslike::{ArgBytesLike, ArgMemoryBuffer};
    use crate::crt_fd::Fd;
    use crate::exceptions::IntoPyException;
    use crate::function::OptionalOption;
    use crate::function::{FuncArgs, OptionalArg};
    use crate::stdlib::os;
    use crate::vm::VirtualMachine;
    use crate::{PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol};
    use crossbeam_utils::atomic::AtomicCell;
    use std::io::{Read, Write};

    bitflags::bitflags! {
        struct Mode: u8 {
            const CREATED   = 0b0001;
            const READABLE  = 0b0010;
            const WRITABLE  = 0b0100;
            const APPENDING = 0b1000;
        }
    }

    enum ModeError {
        Invalid,
        BadRwa,
    }
    impl ModeError {
        fn error_msg(&self, mode_str: &str) -> String {
            match self {
                ModeError::Invalid => format!("invalid mode: {}", mode_str),
                ModeError::BadRwa => {
                    "Must have exactly one of create/read/write/append mode and at most one plus"
                        .to_owned()
                }
            }
        }
    }

    fn compute_mode(mode_str: &str) -> Result<(Mode, i32), ModeError> {
        let mut flags = 0;
        let mut plus = false;
        let mut rwa = false;
        let mut mode = Mode::empty();
        for c in mode_str.bytes() {
            match c {
                b'x' => {
                    if rwa {
                        return Err(ModeError::BadRwa);
                    }
                    rwa = true;
                    mode.insert(Mode::WRITABLE | Mode::CREATED);
                    flags |= libc::O_EXCL | libc::O_CREAT;
                }
                b'r' => {
                    if rwa {
                        return Err(ModeError::BadRwa);
                    }
                    rwa = true;
                    mode.insert(Mode::READABLE);
                }
                b'w' => {
                    if rwa {
                        return Err(ModeError::BadRwa);
                    }
                    rwa = true;
                    mode.insert(Mode::WRITABLE);
                    flags |= libc::O_CREAT | libc::O_TRUNC;
                }
                b'a' => {
                    if rwa {
                        return Err(ModeError::BadRwa);
                    }
                    rwa = true;
                    mode.insert(Mode::WRITABLE | Mode::APPENDING);
                    flags |= libc::O_APPEND | libc::O_CREAT;
                }
                b'+' => {
                    if plus {
                        return Err(ModeError::BadRwa);
                    }
                    plus = true;
                    mode.insert(Mode::READABLE | Mode::WRITABLE);
                }
                b'b' => {}
                _ => return Err(ModeError::Invalid),
            }
        }

        if !rwa {
            return Err(ModeError::BadRwa);
        }

        if mode.contains(Mode::READABLE | Mode::WRITABLE) {
            flags |= libc::O_RDWR
        } else if mode.contains(Mode::READABLE) {
            flags |= libc::O_RDONLY
        } else {
            flags |= libc::O_WRONLY
        }

        #[cfg(windows)]
        {
            flags |= libc::O_BINARY | libc::O_NOINHERIT;
        }
        #[cfg(unix)]
        {
            flags |= libc::O_CLOEXEC
        }

        Ok((mode, flags as _))
    }

    #[pyattr]
    #[pyclass(module = "io", name, base = "_RawIOBase")]
    #[derive(Debug)]
    pub(super) struct FileIO {
        fd: AtomicCell<i32>,
        closefd: AtomicCell<bool>,
        mode: AtomicCell<Mode>,
        seekable: AtomicCell<Option<bool>>,
    }

    type FileIORef = PyRef<FileIO>;

    impl PyValue for FileIO {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[derive(FromArgs)]
    struct FileIOArgs {
        #[pyarg(positional)]
        name: PyObjectRef,
        #[pyarg(any, default)]
        mode: Option<PyStrRef>,
        #[pyarg(any, default = "true")]
        closefd: bool,
        #[pyarg(any, default)]
        opener: Option<PyObjectRef>,
    }

    #[pyimpl(flags(BASETYPE, HAS_DICT))]
    impl FileIO {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<FileIORef> {
            FileIO {
                fd: AtomicCell::new(-1),
                closefd: AtomicCell::new(false),
                mode: AtomicCell::new(Mode::empty()),
                seekable: AtomicCell::new(None),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(magic)]
        fn init(zelf: PyRef<Self>, args: FileIOArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mode_obj = args.mode.unwrap_or_else(|| PyStr::from("rb").into_ref(vm));
            let mode_str = mode_obj.as_str();
            let name = args.name;
            let (mode, flags) =
                compute_mode(mode_str).map_err(|e| vm.new_value_error(e.error_msg(mode_str)))?;
            zelf.mode.store(mode);
            let fd = if let Some(opener) = args.opener {
                let fd = vm.invoke(&opener, (name.clone(), flags))?;
                if !vm.isinstance(&fd, &vm.ctx.types.int_type)? {
                    return Err(vm.new_type_error("expected integer from opener".to_owned()));
                }
                let fd = i32::try_from_object(vm, fd)?;
                if fd < 0 {
                    return Err(vm.new_os_error("Negative file descriptor".to_owned()));
                }
                fd
            } else if let Some(i) = name.payload::<crate::builtins::PyInt>() {
                crate::builtins::int::try_to_primitive(i.as_bigint(), vm)?
            } else {
                let path = os::PyPathLike::try_from_object(vm, name.clone())?;
                if !args.closefd {
                    return Err(
                        vm.new_value_error("Cannot use closefd=False with file name".to_owned())
                    );
                }
                os::open(path, flags as _, None, Default::default(), vm)?
            };

            if mode.contains(Mode::APPENDING) {
                let _ = os::lseek(fd as _, 0, libc::SEEK_END, vm);
            }

            zelf.fd.store(fd);
            zelf.closefd.store(args.closefd);
            #[cfg(windows)]
            crate::stdlib::msvcrt::setmode_binary(fd);
            vm.set_attr(zelf.as_object(), "name", name)?;
            Ok(())
        }

        #[pyproperty]
        fn closed(&self) -> bool {
            self.fd.load() < 0
        }

        #[pyproperty]
        fn closefd(&self) -> bool {
            self.closefd.load()
        }

        #[pymethod]
        fn fileno(&self, vm: &VirtualMachine) -> PyResult<i32> {
            let fd = self.fd.load();
            if fd >= 0 {
                Ok(fd)
            } else {
                Err(io_closed_error(vm))
            }
        }

        fn get_fd(&self, vm: &VirtualMachine) -> PyResult<Fd> {
            self.fileno(vm).map(Fd)
        }

        #[pymethod]
        fn readable(&self) -> bool {
            self.mode.load().contains(Mode::READABLE)
        }
        #[pymethod]
        fn writable(&self) -> bool {
            self.mode.load().contains(Mode::WRITABLE)
        }
        #[pyproperty]
        fn mode(&self) -> &'static str {
            let mode = self.mode.load();
            if mode.contains(Mode::CREATED) {
                if mode.contains(Mode::READABLE) {
                    "xb+"
                } else {
                    "xb"
                }
            } else if mode.contains(Mode::APPENDING) {
                if mode.contains(Mode::READABLE) {
                    "ab+"
                } else {
                    "ab"
                }
            } else if mode.contains(Mode::READABLE) {
                if mode.contains(Mode::WRITABLE) {
                    "rb+"
                } else {
                    "rb"
                }
            } else {
                "wb"
            }
        }

        #[pymethod(magic)]
        fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let fd = zelf.fd.load();
            if fd < 0 {
                return Ok("<_io.FileIO [closed]>".to_owned());
            }
            let name_repr = repr_fileobj_name(zelf.as_object(), vm)?;
            let mode = zelf.mode();
            let closefd = if zelf.closefd.load() { "True" } else { "False" };
            let repr = if let Some(name_repr) = name_repr {
                format!(
                    "<_io.FileIO name={} mode='{}' closefd={}>",
                    name_repr, mode, closefd
                )
            } else {
                format!("<_io.FileIO fd={} mode='{}' closefd={}>", fd, mode, closefd)
            };
            Ok(repr)
        }

        #[pymethod]
        fn read(&self, read_byte: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            if !self.mode.load().contains(Mode::READABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not readable".to_owned(),
                ));
            }
            let mut handle = self.get_fd(vm)?;
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

            Ok(bytes)
        }

        #[pymethod]
        fn readinto(&self, obj: ArgMemoryBuffer, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.mode.load().contains(Mode::READABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not readable".to_owned(),
                ));
            }

            let handle = self.get_fd(vm)?;

            let mut buf = obj.borrow_buf_mut();
            let mut f = handle.take(buf.len() as _);
            let ret = f.read(&mut buf).map_err(|e| e.into_pyexception(vm))?;

            Ok(ret)
        }

        #[pymethod]
        fn write(&self, obj: ArgBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.mode.load().contains(Mode::WRITABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not writable".to_owned(),
                ));
            }

            let mut handle = self.get_fd(vm)?;

            let len = obj
                .with_ref(|b| handle.write(b))
                .map_err(|err| err.into_pyexception(vm))?;

            //return number of bytes written
            Ok(len)
        }

        #[pymethod]
        fn close(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let res = iobase_close(zelf.as_object(), vm);
            if !zelf.closefd.load() {
                zelf.fd.store(-1);
                return res;
            }
            let fd = zelf.fd.swap(-1);
            if fd >= 0 {
                Fd(fd).close().map_err(|e| e.into_pyexception(vm))?;
            }
            res
        }

        #[pymethod]
        fn seekable(&self, vm: &VirtualMachine) -> PyResult<bool> {
            let fd = self.fileno(vm)?;
            Ok(self.seekable.load().unwrap_or_else(|| {
                let seekable = os::lseek(fd, 0, libc::SEEK_CUR, vm).is_ok();
                self.seekable.store(Some(seekable));
                seekable
            }))
        }

        #[pymethod]
        fn seek(
            &self,
            offset: PyObjectRef,
            how: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult<Offset> {
            let how = how.unwrap_or(0);
            let fd = self.fileno(vm)?;
            let offset = get_offset(offset, vm)?;

            os::lseek(fd, offset, how, vm)
        }

        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult<Offset> {
            let fd = self.fileno(vm)?;
            os::lseek(fd, 0, libc::SEEK_CUR, vm)
        }

        #[pymethod]
        fn truncate(&self, len: OptionalOption, vm: &VirtualMachine) -> PyResult<Offset> {
            let fd = self.fileno(vm)?;
            let len = match len.flatten() {
                Some(l) => get_offset(l, vm)?,
                None => os::lseek(fd, 0, libc::SEEK_CUR, vm)?,
            };
            os::ftruncate(fd, len, vm)?;
            Ok(len)
        }

        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult<bool> {
            let fd = self.fileno(vm)?;
            Ok(os::isatty(fd))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(format!("cannot pickle '{}' object", zelf.class().name)))
        }
    }
}
