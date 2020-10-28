/*
 * I/O core tools.
 */
use super::os::Offset;
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
        int, pybool, pytype, PyByteArray, PyStr, PyStrRef, PyTypeRef,
    };
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
    use crate::common::lock::{
        PyMutex, PyRwLock, PyRwLockReadGuard, PyRwLockUpgradableReadGuard, PyRwLockWriteGuard,
        PyThreadMutex, PyThreadMutexGuard,
    };
    use crate::common::rc::PyRc;
    use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::pyobject::{
        BorrowValue, IntoPyObject, PyContext, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
        TryFromObject, TypeProtocol,
    };
    use crate::vm::{ReprGuard, VirtualMachine};

    fn ensure_unclosed(file: &PyObjectRef, msg: &str, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.get_attribute(file.clone(), "closed")?)? {
            Err(vm.new_value_error(msg.to_owned()))
        } else {
            Ok(())
        }
    }

    fn new_unsupported_operation(vm: &VirtualMachine, msg: String) -> PyBaseExceptionRef {
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

        fn truncate(&mut self, pos: Option<usize>) -> PyResult<()> {
            let pos = pos.unwrap_or_else(|| self.tell() as usize);
            self.cursor.get_mut().truncate(pos);
            Ok(())
        }
    }

    fn check_closed(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.get_attribute(file.clone(), "closed")?)? {
            Err(vm.new_value_error("I/O operation on closed file".to_owned()))
        } else {
            Ok(())
        }
    }

    fn check_readable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "readable", ())?)? {
            Ok(())
        } else {
            _unsupported(vm, file, "File or stream is not readable")
        }
    }

    fn check_writable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "writable", ())?)? {
            Ok(())
        } else {
            _unsupported(vm, file, "File or stream is not writable.")
        }
    }

    fn check_seekable(file: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if pybool::boolval(vm, vm.call_method(file, "seekable", ())?)? {
            Ok(())
        } else {
            _unsupported(vm, file, "File or stream is not seekable")
        }
    }

    #[pyattr]
    #[pyclass(name = "_IOBase")]
    struct _IOBase;

    #[pyimpl(flags(BASETYPE))]
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
            vm.call_method(&zelf, "seek", vec![vm.ctx.new_int(0), vm.ctx.new_int(1)])
        }
        #[pymethod]
        fn truncate(zelf: PyObjectRef, _pos: OptionalArg, vm: &VirtualMachine) -> PyResult {
            _unsupported(vm, &zelf, "truncate")
        }
        #[pyattr]

        fn __closed(ctx: &PyContext) -> PyObjectRef {
            ctx.new_bool(false)
        }

        #[pymethod(magic)]
        fn enter(instance: PyObjectRef) -> PyObjectRef {
            instance
        }

        #[pyslot]
        fn tp_del(instance: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            vm.call_method(instance, "close", ())?;
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

        // TODO Check if closed, then if so raise ValueError
        #[pymethod]
        fn flush(_self: PyObjectRef) {}

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
            let closed = pybool::boolval(vm, Self::closed(instance.clone(), vm)?)?;
            if !closed {
                let res = vm.call_method(&instance, "flush", ());
                vm.set_attr(&instance, "__closed", vm.ctx.new_bool(true))?;
                res?;
            }
            Ok(())
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
        fn readlines(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Ok(vm.ctx.new_list(vm.extract_elements(&instance)?))
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

        #[pymethod(magic)]
        fn iter(instance: PyObjectRef) -> PyObjectRef {
            instance
        }
        #[pymethod(magic)]
        fn next(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let line = vm.call_method(&instance, "readline", ())?;
            if !pybool::boolval(vm, line.clone())? {
                Err(vm.new_stop_iteration())
            } else {
                Ok(line)
            }
        }
    }

    #[pyattr]
    #[pyclass(name = "_RawIOBase", base = "_IOBase")]
    pub(super) struct _RawIOBase;

    #[pyimpl(flags(BASETYPE))]
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
            b: PyObjectRef,
            method: &str,
            vm: &VirtualMachine,
        ) -> PyResult<usize> {
            let b = PyRwBytesLike::try_from_object(vm, b)?;
            let mut buf = b.borrow_value();
            let data = vm.call_method(&zelf, method, vec![vm.ctx.new_int(buf.len())])?;
            let data = PyBytesLike::try_from_object(vm, data)?;
            let data = data.borrow_value();
            match buf.get_mut(..data.len()) {
                Some(slice) => {
                    slice.copy_from_slice(&data);
                    Ok(b.len())
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
                let n = self.raw_write(self.write_pos as usize..self.write_end as usize, vm)?;
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
                let res = self.raw_seek(self.raw_offset(), 1, vm);
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
            let target = if whence == 1 { -target } else { target };
            let n = self.raw_seek(target, whence, vm)?;
            self.raw_pos = -1;
            if self.readable() {
                self.reset_read();
            }
            Ok(n)
        }

        fn raw_tell(&mut self, vm: &VirtualMachine) -> PyResult<Offset> {
            let ret = vm.call_method(self.check_init(vm)?, "seek", ())?;
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
            buf_range: Range<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let opts = BufferOptions {
                len: buf_range.len(),
                ..Default::default()
            };
            // TODO: see if we can encapsulate this pattern in a function in memory.rs like
            // fn slice_as_memory<R>(s: &[u8], f: impl FnOnce(PyMemoryViewRef) -> R) -> R
            let writebuf = PyRc::new(BufferedRawBuffer {
                data: std::mem::take(&mut self.buffer).into(),
                range: buf_range,
                opts,
            });
            let memobj =
                PyMemoryView::from_buffer(vm.ctx.none(), BufferRef::new(writebuf.clone()), vm)?
                    .into_ref(vm);

            // TODO: loop if write() raises an interrupt
            let res = vm.call_method(self.raw.as_ref().unwrap(), "write", (memobj.clone(),));

            memobj.released.store(true);
            self.buffer = std::mem::take(&mut writebuf.data.lock());

            let res = res?;

            if vm.is_none(&res) {
                return Ok(None);
            }
            let n = isize::try_from_object(vm, res)?;
            if n.to_usize().map_or(true, |n| n >= self.buffer.len()) {
                return Err(vm.new_os_error(format!(
                    "raw write returned invalid length {} (should have been between 0 and {})",
                    n,
                    self.buffer.len(),
                )));
            }
            if self.abs_pos != -1 {
                self.abs_pos += n as Offset
            }
            Ok(Some(n as usize))
        }

        fn active_read_slice(&self) -> &[u8] {
            &self.buffer[self.pos as usize..][..self.readahead() as usize]
        }

        fn read_fast(&mut self, n: usize) -> Option<Vec<u8>> {
            let ret = self.active_read_slice().get(..n).map(ToOwned::to_owned);
            if ret.is_some() {
                self.pos += n as Offset;
            }
            ret
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
            let raw = self.raw.as_ref().unwrap();
            while remaining > 0 {
                // MINUS_LAST_BLOCK() in CPython
                let r = self.buffer.len() * (remaining / self.buffer.len());
                if r == 0 {
                    break;
                }
                let r = Self::raw_read(raw, &mut self.abs_pos, &mut out, written..r, vm)?;
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
            let len = self.buffer.len() - start;
            let res = Self::raw_read(
                self.raw.as_ref().unwrap(),
                &mut self.abs_pos,
                &mut self.buffer,
                start..len,
                vm,
            );
            if let Ok(Some(n)) = &res {
                let new_start = (start + *n) as Offset;
                self.read_end = new_start;
                self.raw_pos = new_start;
            }
            res
        }

        fn raw_read(
            raw: &PyObjectRef,
            abs_pos: &mut Offset,
            v: &mut Vec<u8>,
            buf_range: Range<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<Option<usize>> {
            let opts = BufferOptions {
                len: buf_range.len(),
                readonly: false,
                ..Default::default()
            };
            // TODO: see if we can encapsulate this pattern in a function in memory.rs like
            // fn slice_as_memory<R>(s: &[u8], f: impl FnOnce(PyMemoryViewRef) -> R) -> R
            let writebuf = PyRc::new(BufferedRawBuffer {
                data: std::mem::take(v).into(),
                range: buf_range,
                opts,
            });
            let memobj =
                PyMemoryView::from_buffer(vm.ctx.none(), BufferRef::new(writebuf.clone()), vm)?
                    .into_ref(vm);

            // TODO: loop if readinto() raises an interrupt
            let res = vm.call_method(raw, "readinto", (memobj.clone(),));

            memobj.released.store(true);
            std::mem::swap(v, &mut writebuf.data.lock());

            let res = res?;

            if vm.is_none(&res) {
                return Ok(None);
            }
            let n = isize::try_from_object(vm, res)?;
            if n.to_usize().map_or(true, |n| n >= v.len()) {
                return Err(vm.new_os_error(format!(
                    "raw write returned invalid length {} (should have been between 0 and {})",
                    n,
                    v.len(),
                )));
            }
            if *abs_pos != -1 {
                *abs_pos += n as Offset
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
    }

    // this is a bit fancier than what CPython does, but in CPython if you store
    // the memoryobj for the buffer until after the BufferedIO is destroyed, you
    // can get a use-after-free, so this is a bit safe
    #[derive(Debug)]
    struct BufferedRawBuffer {
        data: PyMutex<Vec<u8>>,
        range: Range<usize>,
        opts: BufferOptions,
    }
    impl Buffer for PyRc<BufferedRawBuffer> {
        fn get_options(&self) -> BorrowedValue<BufferOptions> {
            (&self.opts).into()
        }

        fn obj_bytes(&self) -> BorrowedValue<[u8]> {
            let data = BorrowedValue::from(self.data.lock());
            BorrowedValue::map(data, |data| &data[self.range.clone()])
        }

        fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
            let data = BorrowedValueMut::from(self.data.lock());
            BorrowedValueMut::map(data, |data| &mut data[self.range.clone()])
        }

        fn release(&self) {}
    }

    fn get_offset(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Offset> {
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
            buffer_size: OptionalArg<isize>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            let buffer_size = match buffer_size {
                OptionalArg::Present(i) => i.to_usize().ok_or_else(|| {
                    vm.new_value_error("buffer size must be strictly positive".to_owned())
                })?,
                OptionalArg::Missing => DEFAULT_BUFFER_SIZE,
            };

            let mut data = self.lock(vm)?;
            data.raw = None;
            data.flags.remove(BufferedFlags::DETACHED);

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
            let mut data = self.lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "seek of closed file", vm)?;
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
            let res = vm.call_method(data.raw.as_ref().unwrap(), "truncate", vec![pos])?;
            let _ = data.raw_tell(vm);
            Ok(res)
        }
        #[pymethod]
        fn flush(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let mut data = zelf.lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "flush of closed file", vm)?;
            data.flush_rewind(vm)
        }
        #[pymethod]
        fn detach(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            vm.call_method(zelf.as_object(), "flush", vec![])?;
            let mut data = zelf.lock(vm)?;
            data.flags.insert(BufferedFlags::DETACHED);
            data.raw
                .take()
                .ok_or_else(|| vm.new_value_error("raw stream has been detached".to_owned()))
        }
        #[pymethod]
        fn seekable(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "seekable", vec![])
        }
        #[pyproperty]
        fn raw(&self, vm: &VirtualMachine) -> PyResult<Option<PyObjectRef>> {
            Ok(self.lock(vm)?.raw.clone())
        }
        fn closed(&self, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(self.lock(vm)?.check_init(vm)?.clone(), "closed")
        }
        #[pyproperty]
        fn name(&self, vm: &VirtualMachine) -> PyResult {
            vm.get_attribute(self.lock(vm)?.check_init(vm)?.clone(), "name")
        }
        #[pymethod]
        fn fileno(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "fileno", vec![])
        }
        #[pymethod]
        fn isatty(&self, vm: &VirtualMachine) -> PyResult {
            vm.call_method(self.lock(vm)?.check_init(vm)?, "isatty", vec![])
        }

        #[pymethod(magic)]
        fn repr(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<String> {
            let name = match vm.get_attribute(zelf.clone(), "name") {
                Ok(name) => Some(name),
                Err(e) if e.isinstance(&vm.ctx.exceptions.value_error) => None,
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
    }

    #[pyattr]
    #[pyclass(name = "BufferedReader", base = "_BufferedIOBase")]
    #[derive(Debug)]
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

    #[pyimpl(with(BufferedMixin), flags(BASETYPE, HAS_DICT))]
    impl BufferedReader {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self {
                data: Default::default(),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn read(&self, size: OptionalSize, vm: &VirtualMachine) -> PyResult<Option<PyBytesRef>> {
            let mut data = self.lock(vm)?;
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
    }

    #[pyattr]
    #[pyclass(name = "BufferedWriter", base = "_BufferedIOBase")]
    #[derive(Debug)]
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

    #[pyimpl(with(BufferedMixin), flags(BASETYPE, HAS_DICT))]
    impl BufferedWriter {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
            Self {
                data: Default::default(),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod]
        fn write(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let data = self.lock(vm)?;
            let raw = data.check_init(vm)?;
            ensure_unclosed(raw, "write to closed file", vm)?;

            //This should be replaced with a more appropriate chunking implementation
            vm.call_method(&raw, "write", (obj,))
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

    #[pyattr]
    #[pyclass(name = "TextIOWrapper", base = "_TextIOBase")]
    struct TextIOWrapper;

    #[pyimpl]
    impl TextIOWrapper {
        #[pymethod(magic)]
        fn init(
            instance: PyObjectRef,
            args: TextIOWrapperArgs,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            args.validate_newline(vm)?;

            let mut encoding: Option<PyStrRef> = args.encoding.clone();
            let mut self_encoding = None; // TODO: Try os.device_encoding(fileno)
            if let (None, None) = (&encoding, &self_encoding) {
                // TODO: locale module
                self_encoding = Some("utf-8");
            }
            if let Some(self_encoding) = self_encoding {
                encoding = Some(PyStr::from(self_encoding).into_ref(vm));
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

            vm.set_attr(&instance, "encoding", self_encoding.into_pyobject(vm))?;
            vm.set_attr(&instance, "errors", errors)?;
            vm.set_attr(&instance, "buffer", args.buffer)?;

            Ok(())
        }

        #[pymethod]
        fn seekable(_self: PyObjectRef) -> bool {
            true
        }

        #[pymethod]
        fn seek(
            instance: PyObjectRef,
            offset: PyObjectRef,
            how: OptionalArg,
            vm: &VirtualMachine,
        ) -> PyResult {
            let raw = vm.get_attribute(instance, "buffer")?;
            let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
            vm.invoke(&vm.get_attribute(raw, "seek")?, args)
        }

        #[pymethod]
        fn tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "buffer")?;
            vm.invoke(&vm.get_attribute(raw, "tell")?, ())
        }

        #[pyproperty]
        fn mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "buffer")?;
            vm.get_attribute(raw, "mode")
        }

        #[pyproperty]
        fn name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "buffer")?;
            vm.get_attribute(raw, "name")
        }

        #[pymethod]
        fn fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "buffer")?;
            vm.call_method(&raw, "fileno", ())
        }

        #[pymethod]
        fn read(
            instance: PyObjectRef,
            size: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let buffered_reader_class = BufferedReader::static_type();
            let raw = vm.get_attribute(instance, "buffer").unwrap();

            if !raw.isinstance(&buffered_reader_class) {
                // TODO: this should be io.UnsupportedOperation error which derives both from ValueError *and* OSError
                return Err(vm.new_value_error("not readable".to_owned()));
            }

            let bytes = vm.call_method(&raw, "read", (size.flatten(),))?;
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
        fn write(instance: PyObjectRef, obj: PyStrRef, vm: &VirtualMachine) -> PyResult<usize> {
            use std::str::from_utf8;

            let buffered_writer_class = BufferedWriter::static_type();
            let raw = vm.get_attribute(instance, "buffer").unwrap();

            if !raw.isinstance(&buffered_writer_class) {
                // TODO: this should be io.UnsupportedOperation error which derives from ValueError and OSError
                return Err(vm.new_value_error("not writable".to_owned()));
            }

            let bytes = obj.borrow_value().to_owned().into_bytes();

            let len = vm.call_method(&raw, "write", (vm.ctx.new_bytes(bytes.clone()),))?;
            let len = int::try_to_primitive(int::get_value(&len), vm)?;

            // returns the count of unicode code points written
            let len = from_utf8(&bytes[..len])
                .unwrap_or_else(|e| from_utf8(&bytes[..e.valid_up_to()]).unwrap())
                .chars()
                .count();
            Ok(len)
        }

        #[pymethod]
        fn readline(
            instance: PyObjectRef,
            size: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<String> {
            let buffered_reader_class = BufferedReader::static_type();
            let raw = vm.get_attribute(instance, "buffer").unwrap();

            if !raw.isinstance(&buffered_reader_class) {
                // TODO: this should be io.UnsupportedOperation error which derives both from ValueError *and* OSError
                return Err(vm.new_value_error("not readable".to_owned()));
            }

            let bytes = vm.call_method(&raw, "readline", (size.flatten(),))?;
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

    #[pyimpl(flags(BASETYPE), with(PyRef))]
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
        fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<()> {
            let mut buffer = self.buffer(vm)?;
            buffer.truncate(pos.try_usize(vm)?)?;
            Ok(())
        }
    }

    #[pyattr]
    #[pyclass(name = "BytesIO", base = "_BufferedIOBase")]
    #[derive(Debug)]
    struct BytesIO {
        buffer: PyRwLock<BufferedIO>,
        closed: AtomicCell<bool>,
        exports: AtomicCell<usize>,
        buffer_options: PyRwLock<Option<Box<BufferOptions>>>,
    }

    type BytesIORef = PyRef<BytesIO>;

    impl PyValue for BytesIO {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl(flags(BASETYPE), with(PyRef))]
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
                buffer_options: PyRwLock::new(None),
            }
            .into_ref_with_type(vm, cls)
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
        fn truncate(self, pos: OptionalSize, vm: &VirtualMachine) -> PyResult<()> {
            let mut buffer = self.try_resizable(vm)?;
            buffer.truncate(pos.try_usize(vm)?)?;
            Ok(())
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
            let buffer = BufferRef::new(self.clone());
            let view = PyMemoryView::from_buffer(self.clone().into_object(), buffer, vm)?;
            self.exports.fetch_add(1);
            Ok(view)
        }
    }

    impl Buffer for BytesIORef {
        fn get_options(&self) -> BorrowedValue<BufferOptions> {
            let guard = self.buffer_options.upgradable_read();
            let guard = if guard.is_none() {
                let mut w = PyRwLockUpgradableReadGuard::upgrade(guard);
                *w = Some(Box::new(BufferOptions {
                    readonly: false,
                    len: self.buffer.read().cursor.get_ref().len(),
                    ..Default::default()
                }));
                PyRwLockWriteGuard::downgrade(w)
            } else {
                PyRwLockUpgradableReadGuard::downgrade(guard)
            };
            PyRwLockReadGuard::map(guard, |x| x.as_ref().unwrap().as_ref()).into()
        }

        fn obj_bytes(&self) -> BorrowedValue<[u8]> {
            PyRwLockReadGuard::map(self.buffer.read(), |x| x.cursor.get_ref().as_slice()).into()
        }

        fn obj_bytes_mut(&self) -> BorrowedValueMut<[u8]> {
            PyRwLockWriteGuard::map(self.buffer.write(), |x| x.cursor.get_mut().as_mut_slice())
                .into()
        }

        fn release(&self) {
            let mut w = self.buffer_options.write();
            if self.exports.fetch_sub(1) == 1 {
                *w = None;
            }
        }
    }

    impl<'a> ResizeGuard<'a> for BytesIO {
        type Resizable = PyRwLockWriteGuard<'a, BufferedIO>;

        fn try_resizable(&'a self, vm: &VirtualMachine) -> PyResult<Self::Resizable> {
            let buffer = self.buffer(vm)?;
            if self.exports.load() == 0 {
                Ok(buffer)
            } else {
                Err(vm.new_buffer_error(
                    "Existing exports of data: object cannot be re-sized".to_owned(),
                ))
            }
        }
    }

    fn split_mode_string(mode_string: &str) -> Result<(String, String), String> {
        let mut mode: char = '\0';
        let mut typ: char = '\0';
        let mut plus_is_set = false;

        let invalid_mode = || Err(format!("invalid mode: '{}'", mode_string));
        for ch in mode_string.chars() {
            match ch {
                '+' => {
                    if plus_is_set {
                        return invalid_mode();
                    }
                    plus_is_set = true;
                }
                't' | 'b' => {
                    if typ != '\0' {
                        return if typ == ch {
                            // no duplicates allowed
                            invalid_mode()
                        } else {
                            Err("can't have text and binary mode at once".to_owned())
                        };
                    }
                    typ = ch;
                }
                'a' | 'r' | 'w' => {
                    if mode != '\0' {
                        return if mode == ch {
                            // no duplicates allowed
                            invalid_mode()
                        } else {
                            Err("must have exactly one of create/read/write/append mode".to_owned())
                        };
                    }
                    mode = ch;
                }
                _ => return invalid_mode(),
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

    #[pyfunction]
    fn open(
        file: PyObjectRef,
        mode: OptionalArg<PyStrRef>,
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

    #[pyfunction]
    fn open_code(file: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // TODO: lifecycle hooks or something?
        io_open(file, Some("rb"), OpenArgs::default(), vm)
    }

    #[derive(FromArgs)]
    #[allow(unused)]
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
        let mode_string = mode.unwrap_or("rt");
        let (mode, typ) = split_mode_string(mode_string).map_err(|e| vm.new_value_error(e))?;

        let io_module = vm.import("_io", &[], 0)?;

        // Construct a FileIO (subclass of RawIOBase)
        // This is subsequently consumed by a Buffered Class.
        let file_io_class = vm.get_attribute(io_module.clone(), "FileIO").map_err(|_| {
            new_unsupported_operation(
                vm,
                "Couldn't get FileIO, io.open likely isn't supported on your platform".to_owned(),
            )
        })?;
        let file_io_obj = vm.invoke(
            &file_io_class,
            FuncArgs::new(
                vec![file, vm.ctx.new_str(mode.clone())],
                maplit::hashmap! {
                    "closefd".to_owned() => vm.ctx.new_bool(opts.closefd),
                    "opener".to_owned() => vm.unwrap_or_none(opts.opener),
                },
            ),
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
                vm.invoke(&buffered_writer_class, (file_io_obj,))
            }
            'r' => {
                let buffered_reader_class = vm
                    .get_attribute(io_module.clone(), "BufferedReader")
                    .unwrap();
                vm.invoke(&buffered_reader_class, (file_io_obj,))
            }
            //TODO: updating => PyBufferedRandom
            _ => unimplemented!("'+' modes is not yet implemented"),
        };

        match typ.chars().next().unwrap() {
            // If the mode is text this buffer type is consumed on construction of
            // a TextIOWrapper which is subsequently returned.
            't' => {
                let text_io_wrapper_class = vm.get_attribute(io_module, "TextIOWrapper").unwrap();
                vm.invoke(&text_io_wrapper_class, (buffered.unwrap(),))
            }
            // If the mode is binary this Buffered class is returned directly at
            // this point.
            // For Buffered class construct "raw" IO class e.g. FileIO and pass this into corresponding field
            'b' => buffered,
            _ => unreachable!(),
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
}

// disable FileIO on WASM
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[pymodule]
mod fileio {
    use super::_io::*;
    use crate::builtins::{PyStrRef, PyTypeRef};
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::exceptions::IntoPyException;
    use crate::function::{FuncArgs, OptionalArg};
    use crate::pyobject::{
        BorrowValue, Either, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
    };
    use crate::stdlib::os;
    use crate::vm::VirtualMachine;
    use crossbeam_utils::atomic::AtomicCell;
    use std::io::{Read, Seek, SeekFrom, Write};

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

    #[pyattr]
    #[pyclass(module = "io", name, base = "_RawIOBase")]
    #[derive(Debug)]
    pub(super) struct FileIO {
        fd: AtomicCell<i64>,
        closefd: AtomicCell<bool>,
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

    #[pyimpl(flags(HAS_DICT))]
    impl FileIO {
        #[pyslot]
        fn tp_new(cls: PyTypeRef, _args: FuncArgs, vm: &VirtualMachine) -> PyResult<FileIORef> {
            FileIO {
                fd: AtomicCell::new(-1),
                closefd: AtomicCell::new(false),
            }
            .into_ref_with_type(vm, cls)
        }

        #[pymethod(magic)]
        fn init(zelf: PyRef<Self>, args: FileIOArgs, vm: &VirtualMachine) -> PyResult<()> {
            let mode = args
                .mode
                .map(|mode| mode.borrow_value().to_owned())
                .unwrap_or_else(|| "r".to_owned());
            let name = args.name.clone();
            let fd = if let Some(opener) = args.opener {
                let mode = compute_c_flag(&mode);
                let fd = vm.invoke(&opener, (name.clone(), mode))?;
                if !vm.isinstance(&fd, &vm.ctx.types.int_type)? {
                    return Err(vm.new_type_error("expected integer from opener".to_owned()));
                }
                let fd = i64::try_from_object(vm, fd)?;
                if fd < 0 {
                    return Err(vm.new_os_error("Negative file descriptor".to_owned()));
                }
                fd
            } else {
                match Either::<i64, os::PyPathLike>::try_from_object(vm, args.name)? {
                    Either::A(fno) => fno,
                    Either::B(path) => {
                        if !args.closefd {
                            return Err(vm.new_value_error(
                                "Cannot use closefd=False with file name".to_owned(),
                            ));
                        }
                        let mode = compute_c_flag(&mode);
                        os::open(
                            path,
                            mode as _,
                            OptionalArg::Missing,
                            OptionalArg::Missing,
                            vm,
                        )?
                    }
                }
            };

            zelf.fd.store(fd);
            zelf.closefd.store(args.closefd);
            vm.set_attr(zelf.as_object(), "name", name)?;
            vm.set_attr(zelf.as_object(), "mode", vm.ctx.new_str(mode))?;
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
            true
        }

        #[pymethod]
        fn read(&self, read_byte: OptionalSize, vm: &VirtualMachine) -> PyResult<Vec<u8>> {
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
            let length = obj.len() as u64;

            let handle = self.get_file(vm)?;

            let mut f = handle.take(length);
            let ret = f
                .read(&mut *obj.borrow_value())
                .map_err(|_| vm.new_value_error("Error reading from Take".to_owned()))?;

            self.set_file(f.into_inner())?;

            Ok(ret)
        }

        #[pymethod]
        fn writable(&self) -> bool {
            true
        }

        #[pymethod]
        fn write(&self, obj: PyBytesLike, vm: &VirtualMachine) -> PyResult<usize> {
            let mut handle = self.get_file(vm)?;

            let len = obj
                .with_ref(|b| handle.write(b))
                .map_err(|err| err.into_pyexception(vm))?;

            self.set_file(handle)?;

            //return number of bytes written
            Ok(len)
        }

        #[pymethod]
        fn close(&self) {
            let fd = self.fd.swap(-1);
            if fd >= 0 && self.closefd.load() {
                let _ = os::rust_file(fd);
            }
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
        ) -> PyResult<u64> {
            let mut handle = self.get_file(vm)?;

            let new_pos = handle
                .seek(seekfrom(vm, offset, how)?)
                .map_err(|err| err.into_pyexception(vm))?;

            self.set_file(handle)?;

            Ok(new_pos)
        }

        #[pymethod]
        fn tell(&self, vm: &VirtualMachine) -> PyResult<u64> {
            let mut handle = self.get_file(vm)?;

            let pos = handle
                .seek(SeekFrom::Current(0))
                .map_err(|err| err.into_pyexception(vm))?;

            self.set_file(handle)?;

            Ok(pos)
        }
    }
}
