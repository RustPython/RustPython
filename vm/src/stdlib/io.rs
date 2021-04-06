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
use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;
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

#[pymodule]
mod _io {
    use super::*;

    use bstr::ByteSlice;
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::ToPrimitive;
    use std::io::{self, prelude::*, Cursor, SeekFrom};
    use std::ops::Range;

    use crate::builtins::memory::{Buffer, BufferOptions, BufferRef, PyMemoryView, ResizeGuard};
    use crate::builtins::{
        bytes::{PyBytes, PyBytesRef},
        pybool, pytype, PyByteArray, PyStr, PyStrRef, PyTypeRef,
    };
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
    use crate::common::lock::{
        PyMappedThreadMutexGuard, PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard,
        PyThreadMutex, PyThreadMutexGuard,
    };
    use crate::common::rc::PyRc;
    use crate::exceptions::{self, IntoPyException, PyBaseExceptionRef};
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::pyobject::{
        BorrowValue, Either, IdProtocol, IntoPyObject, PyContext, PyIterable, PyObjectRef, PyRef,
        PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
    };
    use crate::vm::{ReprGuard, VirtualMachine};

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
                let read_res = PyBytesLike::try_from_object(vm, vm.invoke(&read, (1,))?)?;
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
            let line = vm.call_method(&instance, "readline", ())?;
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
                    let bytes = &mut b.borrow_value_mut().elements;
                    bytes.truncate(n);
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
                        if b.borrow_value().is_empty() {
                            break;
                        }
                        total_len += b.borrow_value().len();
                        chunks.push(b)
                    }
                }
            }
            let mut ret = Vec::with_capacity(total_len);
            for b in chunks {
                ret.extend_from_slice(b.borrow_value())
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
            let b = PyRwBytesLike::new(vm, &bufobj)?;
            let l = b.len();
            let data = vm.call_method(&zelf, method, (l,))?;
            if data.is(&bufobj) {
                return Ok(l);
            }
            let mut buf = b.borrow_value();
            let data = PyBytesLike::try_from_object(vm, data)?;
            let data = data.borrow_value();
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
            buf: Option<BufferRef>,
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
                let memobj =
                    PyMemoryView::from_buffer(vm.ctx.none(), BufferRef::new(writebuf.clone()), vm)?
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

        fn write(&mut self, obj: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.valid_read() && !self.valid_write() {
                self.pos = 0;
                self.raw_pos = 0;
            }
            let avail = self.buffer.len() - self.pos as usize;
            let buf_len;
            {
                let buf = obj.borrow_value();
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
                    self.raw_write(Some(BufferRef::new(rcbuf.clone())), written..buf_len, vm)?;
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
            v: Either<Option<&mut Vec<u8>>, BufferRef>,
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
                        BufferRef::new(readbuf.clone()),
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
                        data.extend_from_slice(bytes.borrow_value());
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
                    Some(b) if !b.borrow_value().is_empty() => {
                        let l = b.borrow_value().len();
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
                                data.extend_from_slice(bytes.borrow_value())
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
            buf: BufferRef,
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
                    let buf = BufferRef::new(rcbuf.clone());
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
    impl Buffer for PyRc<BufferedRawBuffer> {
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
        int.borrow_value().try_into().map_err(|_| {
            vm.new_value_error(format!(
                "cannot fit '{}' into an offset-sized integer",
                obj.class().name
            ))
        })
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
            let name = match vm.get_attribute(zelf.clone(), "name") {
                Ok(name) => Some(name),
                Err(e)
                    if e.isinstance(&vm.ctx.exceptions.attribute_error)
                        || e.isinstance(&vm.ctx.exceptions.value_error) =>
                {
                    None
                }
                Err(e) => return Err(e),
            };
            if let Some(name) = name {
                if let Some(_guard) = ReprGuard::enter(vm, &zelf) {
                    let repr = vm.to_repr(&name)?;
                    Ok(format!("<{} name={}>", zelf.class().tp_name(), repr))
                } else {
                    Err(vm.new_runtime_error(format!(
                        "reentrant call inside {}.__repr__",
                        zelf.class().tp_name()
                    )))
                }
            } else {
                Ok(format!("<{}>", zelf.class().tp_name()))
            }
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
        fn readinto(&self, buf: PyRwBytesLike, vm: &VirtualMachine) -> PyResult<Option<usize>> {
            let mut data = self.reader().lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "readinto of closed file", vm)?;
            data.readinto_generic(buf.into_buffer(), false, vm)
        }
        #[pymethod]
        fn readinto1(&self, buf: PyRwBytesLike, vm: &VirtualMachine) -> PyResult<Option<usize>> {
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
        fn write(&self, obj: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
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
        newline: Option<PyStrRef>,
        #[pyarg(any, default = "false")]
        line_buffering: bool,
    }

    impl TextIOWrapperArgs {
        fn validate_newline(&self, vm: &VirtualMachine) -> PyResult<()> {
            if let Some(pystr) = &self.newline {
                match pystr.borrow_value() {
                    "" | "\n" | "\r" | "\r\n" => Ok(()),
                    _ => Err(
                        vm.new_value_error(format!("illegal newline value: '{}'", pystr.repr(vm)?))
                    ),
                }
            } else {
                Ok(())
            }
        }
    }

    #[derive(Debug)]
    struct TextIOData {
        buffer: PyObjectRef,
        // TODO: respect the encoding
        encoding: PyStrRef,
        // TODO: respect errors setting
        errors: PyStrRef,
        // TODO: respect newline
        newline: Option<PyStrRef>,
        // TODO: respect line_buffering
        line_buffering: bool,
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
            args.validate_newline(vm)?;
            let mut data = self.lock_opt(vm)?;
            *data = None;

            let encoding = match args.encoding {
                Some(enc) => enc,
                None => {
                    // TODO: try os.device_encoding(fileno) and then locale.getpreferredencoding()
                    PyStr::from("utf-8").into_ref(vm)
                }
            };

            let errors = args
                .errors
                .unwrap_or_else(|| PyStr::from("strict").into_ref(vm));

            // let readuniversal = args.newline.map_or_else(true, |s| s.borrow_value().is_empty());

            *data = Some(TextIOData {
                buffer: args.buffer,
                encoding,
                errors,
                newline: args.newline,
                line_buffering: args.line_buffering,
            });

            Ok(())
        }

        #[pymethod]
        fn seekable(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.get_attribute(buffer, "seekable")
        }

        #[pymethod]
        fn seek(
            &self,
            offset: PyObjectRef,
            how: OptionalArg<i32>,
            vm: &VirtualMachine,
        ) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            let offset = get_offset(offset, vm)?;
            let how = how.unwrap_or(0);
            if how == 1 && offset != 0 {
                return Err(new_unsupported_operation(
                    vm,
                    "can't do nonzero cur-relative seeks".to_owned(),
                ));
            } else if how == 2 && offset != 0 {
                return Err(new_unsupported_operation(
                    vm,
                    "can't do nonzero end-relative seeks".to_owned(),
                ));
            }
            vm.call_method(&buffer, "seek", (offset, how))
        }

        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.call_method(&buffer, "tell", ())
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
        fn read(&self, size: OptionalOption<PyObjectRef>, vm: &VirtualMachine) -> PyResult<String> {
            let buffer = self.lock(vm)?.buffer.clone();
            check_readable(&buffer, vm)?;

            let bytes = vm.call_method(&buffer, "read", (size.flatten(),))?;
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

        #[pymethod]
        fn write(&self, obj: PyStrRef, vm: &VirtualMachine) -> PyResult<usize> {
            use std::str::from_utf8;

            let buffer = self.lock(vm)?.buffer.clone();
            check_writable(&buffer, vm)?;

            let bytes = obj.borrow_value().as_bytes();

            let len = vm.call_method(&buffer, "write", (bytes.to_owned(),));
            if obj.borrow_value().contains('\n') {
                let _ = vm.call_method(&buffer, "flush", ());
            }
            let len = usize::try_from_object(vm, len?)?;

            // returns the count of unicode code points written
            let len = from_utf8(&bytes[..len])
                .unwrap_or_else(|e| from_utf8(&bytes[..e.valid_up_to()]).unwrap())
                .chars()
                .count();
            Ok(len)
        }

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            check_closed(&buffer, vm)?;
            vm.call_method(&buffer, "flush", ())
        }

        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            check_closed(&buffer, vm)?;
            vm.call_method(&buffer, "isatty", ())
        }

        #[pymethod]
        fn readline(
            &self,
            size: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let buffer = self.lock(vm)?.buffer.clone();
            check_readable(&buffer, vm)?;

            let bytes = vm.call_method(&buffer, "readline", (size.flatten(),))?;
            let bytes = PyBytesLike::try_from_object(vm, bytes)?;
            //format bytes into string
            let rust_string = String::from_utf8(bytes.borrow_value().to_vec()).map_err(|e| {
                vm.new_unicode_decode_error(format!(
                    "cannot decode byte at index: {}",
                    e.utf8_error().valid_up_to()
                ))
            })?;
            Ok(rust_string)
        }

        #[pymethod]
        fn close(&self, vm: &VirtualMachine) -> PyResult {
            let buffer = self.lock(vm)?.buffer.clone();
            vm.call_method(&buffer, "close", ())
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

    #[derive(FromArgs)]
    struct StringIOArgs {
        #[pyarg(any, default)]
        #[allow(dead_code)]
        // TODO: use this
        newline: Option<PyStrRef>,
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
                .map_or_else(Vec::new, |v| v.borrow_value().as_bytes().to_vec());

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
            let bytes = data.borrow_value().as_bytes();

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
                .map_or_else(Vec::new, |input| input.borrow_value().to_vec());

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
        fn write(self, data: PyBytesLike, vm: &VirtualMachine) -> PyResult<u64> {
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
        fn readinto(self, obj: PyRwBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let mut buf = self.buffer(vm)?;
            let ret = buf
                .cursor
                .read(&mut *obj.borrow_value())
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
            let buffer = BufferRef::new(BytesIOBuffer {
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

    impl Buffer for BytesIOBuffer {
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
            args.mode.as_ref().into_option().map(|s| s.borrow_value()),
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

        let buffering = if opts.buffering < 0 {
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
                    (buffered, opts.encoding, opts.errors, opts.newline),
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
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::exceptions::IntoPyException;
    use crate::function::OptionalOption;
    use crate::function::{FuncArgs, OptionalArg};
    use crate::pyobject::{
        BorrowValue, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
    };
    use crate::stdlib::os;
    use crate::vm::VirtualMachine;
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

    fn compute_mode(mode_str: &str) -> Result<(Mode, os::OpenFlags), ModeError> {
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
        fd: AtomicCell<i64>,
        closefd: AtomicCell<bool>,
        mode: AtomicCell<Mode>,
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
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(magic)]
        fn init(zelf: PyRef<Self>, args: FileIOArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mode_obj = args.mode.unwrap_or_else(|| PyStr::from("rb").into_ref(vm));
            let mode_str = mode_obj.borrow_value();
            let name = args.name;
            let (mode, flags) =
                compute_mode(mode_str).map_err(|e| vm.new_value_error(e.error_msg(mode_str)))?;
            zelf.mode.store(mode);
            let fd = if let Some(opener) = args.opener {
                let fd = vm.invoke(&opener, (name.clone(), flags))?;
                if !vm.isinstance(&fd, &vm.ctx.types.int_type)? {
                    return Err(vm.new_type_error("expected integer from opener".to_owned()));
                }
                let fd = i64::try_from_object(vm, fd)?;
                if fd < 0 {
                    return Err(vm.new_os_error("Negative file descriptor".to_owned()));
                }
                fd
            } else if let Some(i) = name.payload::<crate::builtins::PyInt>() {
                crate::builtins::int::try_to_primitive(i.borrow_value(), vm)?
            } else {
                let path = os::PyPathLike::try_from_object(vm, name.clone())?;
                if !args.closefd {
                    return Err(
                        vm.new_value_error("Cannot use closefd=False with file name".to_owned())
                    );
                }
                os::open(
                    path,
                    flags as _,
                    OptionalArg::Missing,
                    Default::default(),
                    vm,
                )?
            };

            if mode.contains(Mode::APPENDING) {
                let _ = os::lseek(fd as _, 0, libc::SEEK_END, vm);
            }

            zelf.fd.store(fd);
            zelf.closefd.store(args.closefd);
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
        fn fileno(&self, vm: &VirtualMachine) -> PyResult<i64> {
            let fd = self.fd.load();
            if fd >= 0 {
                Ok(fd)
            } else {
                Err(io_closed_error(vm))
            }
        }

        fn get_file(&self, vm: &VirtualMachine) -> PyResult<std::fs::File> {
            let fileno = self.fileno(vm)?;
            Ok(os::rust_file(fileno))
        }

        fn set_file(&self, f: std::fs::File) -> PyResult<()> {
            let updated = os::raw_file_number(f);
            self.fd.store(updated);
            Ok(())
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

        #[pymethod]
        fn flush(&self, vm: &VirtualMachine) -> PyResult<()> {
            let mut handle = self.get_file(vm)?;
            handle.flush().map_err(|e| e.into_pyexception(vm))?;
            self.set_file(handle)?;
            Ok(())
        }

        #[pymethod]
        fn read(&self, read_byte: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
            if !self.mode.load().contains(Mode::READABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not readable".to_owned(),
                ));
            }
            let mut handle = self.get_file(vm)?;
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
            self.set_file(handle)?;

            Ok(bytes)
        }

        #[pymethod]
        fn readinto(&self, obj: PyRwBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.mode.load().contains(Mode::READABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not readable".to_owned(),
                ));
            }

            let handle = self.get_file(vm)?;

            let mut buf = obj.borrow_value();
            let mut f = handle.take(buf.len() as _);
            let ret = f.read(&mut buf).map_err(|e| e.into_pyexception(vm))?;

            self.set_file(f.into_inner())?;

            Ok(ret)
        }

        #[pymethod]
        fn write(&self, obj: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            if !self.mode.load().contains(Mode::WRITABLE) {
                return Err(new_unsupported_operation(
                    vm,
                    "File or stream is not writable".to_owned(),
                ));
            }

            let mut handle = self.get_file(vm)?;

            let len = obj
                .with_ref(|b| handle.write(b))
                .map_err(|err| err.into_pyexception(vm))?;

            self.set_file(handle)?;

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
                // TODO: detect errors from file close
                let _ = os::rust_file(fd);
            }
            res
        }

        #[pymethod]
        fn seekable(&self) -> bool {
            true
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

            os::lseek(fd as _, offset, how, vm)
        }

        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult<Offset> {
            let fd = self.fileno(vm)?;
            os::lseek(fd as _, 0, libc::SEEK_CUR, vm)
        }

        #[pymethod]
        fn truncate(&self, len: OptionalOption, vm: &VirtualMachine) -> PyResult<Offset> {
            let fd = self.fileno(vm)?;
            let len = match len.flatten() {
                Some(l) => get_offset(l, vm)?,
                None => os::lseek(fd as _, 0, libc::SEEK_CUR, vm)?,
            };
            os::ftruncate(fd, len, vm)?;
            Ok(len)
        }

        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult<bool> {
            let fd = self.fileno(vm)?;
            Ok(os::isatty(fd as _))
        }

        #[pymethod(magic)]
        fn reduce(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error(format!("cannot pickle '{}' object", zelf.class().name)))
        }
    }
}
