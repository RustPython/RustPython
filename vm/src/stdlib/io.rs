/*
 * I/O core tools.
 */
use crate::pyobject::PyObjectRef;
use crate::VirtualMachine;
pub(crate) use _io::io_open as open;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let module = _io::make_module(vm);
    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    fileio::extend_module(vm, &module);
    module
}

#[pymodule]
mod _io {
    use bstr::ByteSlice;
    use crossbeam_utils::atomic::AtomicCell;
    use num_traits::ToPrimitive;
    use std::io::{self, prelude::*, Cursor, SeekFrom};

    use crate::builtins::bytearray::PyByteArray;
    use crate::builtins::bytes::PyBytesRef;
    use crate::builtins::int;
    use crate::builtins::memory::{Buffer, BufferOptions, BufferRef, PyMemoryView, ResizeGuard};
    use crate::builtins::pybool;
    use crate::builtins::pystr::{self, PyStr, PyStrRef};
    use crate::builtins::pytype::PyTypeRef;
    use crate::byteslike::{PyBytesLike, PyRwBytesLike};
    use crate::common::borrow::{BorrowedValue, BorrowedValueMut};
    use crate::common::lock::{
        PyRwLock, PyRwLockReadGuard, PyRwLockUpgradableReadGuard, PyRwLockWriteGuard,
    };
    use crate::exceptions::{IntoPyException, PyBaseExceptionRef};
    use crate::function::{FuncArgs, OptionalArg, OptionalOption};
    use crate::pyobject::{
        BorrowValue, IntoPyObject, PyContext, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
        TryFromObject, TypeProtocol,
    };
    use crate::vm::VirtualMachine;

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

    #[pyattr]
    #[pyclass(name = "_IOBase")]
    struct _IOBase;

    #[pyimpl(flags(BASETYPE))]
    impl _IOBase {
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
        fn check_closed(
            instance: PyObjectRef,
            msg: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if pybool::boolval(vm, vm.get_attribute(instance, "closed")?)? {
                let msg = msg
                    .flatten()
                    .unwrap_or_else(|| vm.ctx.new_str("I/O operation on closed file"));
                Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
            } else {
                Ok(())
            }
        }

        #[pymethod(name = "_checkReadable")]
        fn check_readable(
            instance: PyObjectRef,
            msg: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if !pybool::boolval(vm, vm.call_method(&instance, "readable", ())?)? {
                let msg = msg
                    .flatten()
                    .unwrap_or_else(|| vm.ctx.new_str("File or stream is not readable."));
                Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
            } else {
                Ok(())
            }
        }

        #[pymethod(name = "_checkWritable")]
        fn check_writable(
            instance: PyObjectRef,
            msg: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if !pybool::boolval(vm, vm.call_method(&instance, "writable", ())?)? {
                let msg = msg
                    .flatten()
                    .unwrap_or_else(|| vm.ctx.new_str("File or stream is not writable."));
                Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
            } else {
                Ok(())
            }
        }

        #[pymethod(name = "_checkSeekable")]
        fn check_seekable(
            instance: PyObjectRef,
            msg: OptionalOption<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            if !pybool::boolval(vm, vm.call_method(&instance, "seekable", ())?)? {
                let msg = msg
                    .flatten()
                    .unwrap_or_else(|| vm.ctx.new_str("File or stream is not seekable."));
                Err(vm.new_exception(vm.ctx.exceptions.value_error.clone(), vec![msg]))
            } else {
                Ok(())
            }
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
        // #[pymethod(magic)]
        fn init(
            instance: PyObjectRef,
            raw: PyObjectRef,
            buffer_size: OptionalArg<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            vm.set_attr(&instance, "raw", raw)?;
            vm.set_attr(
                &instance,
                "buffer_size",
                vm.ctx.new_int(buffer_size.unwrap_or(DEFAULT_BUFFER_SIZE)),
            )?;
            Ok(())
        }

        // #[pymethod]
        fn fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "raw")?;
            vm.call_method(&raw, "fileno", ())
        }

        // #[pyproperty]
        fn mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "raw")?;
            vm.get_attribute(raw, "mode")
        }

        // #[pyproperty]
        fn name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "raw")?;
            vm.get_attribute(raw, "name")
        }

        // #[pymethod]
        fn tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "raw")?;
            vm.invoke(&vm.get_attribute(raw, "tell")?, ())
        }

        // #[pymethod]
        fn close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            let raw = vm.get_attribute(instance, "raw")?;
            vm.invoke(&vm.get_attribute(raw, "close")?, ())?;
            Ok(())
        }
    }

    // TextIO Base has no public constructor
    #[pyattr]
    #[pyclass(name = "_TextIOBase", base = "_IOBase")]
    struct _TextIOBase;

    #[pyimpl(flags(BASETYPE))]
    impl _TextIOBase {}

    #[pyattr]
    #[pyclass(name = "BufferedReader", base = "_BufferedIOBase")]
    struct BufferedReader;

    #[pyimpl(flags(BASETYPE))]
    impl BufferedReader {
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547

        #[pymethod(magic)]
        fn init(
            instance: PyObjectRef,
            raw: PyObjectRef,
            buffer_size: OptionalArg<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            _BufferedIOBase::init(instance, raw, buffer_size, vm)
        }

        #[pymethod]
        fn fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::fileno(instance, vm)
        }

        #[pyproperty]
        fn mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::mode(instance, vm)
        }

        #[pyproperty]
        fn name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::name(instance, vm)
        }

        #[pymethod]
        fn tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::tell(instance, vm)
        }

        #[pymethod]
        fn close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            _BufferedIOBase::close(instance, vm)
        }

        #[pymethod]
        fn read(instance: PyObjectRef, size: OptionalSize, vm: &VirtualMachine) -> PyResult {
            vm.call_method(
                &vm.get_attribute(instance, "raw")?,
                "read",
                (size.to_usize(),),
            )
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
            let raw = vm.get_attribute(instance, "raw")?;
            let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
            vm.invoke(&vm.get_attribute(raw, "seek")?, args)
        }
    }

    #[pyattr]
    #[pyclass(name = "BufferedWriter", base = "_BufferedIOBase")]
    struct BufferedWriter;

    #[pyimpl(flags(BASETYPE))]
    impl BufferedWriter {
        //workaround till the buffered classes can be fixed up to be more
        //consistent with the python model
        //For more info see: https://github.com/RustPython/RustPython/issues/547

        #[pymethod(magic)]
        fn init(
            instance: PyObjectRef,
            raw: PyObjectRef,
            buffer_size: OptionalArg<usize>,
            vm: &VirtualMachine,
        ) -> PyResult<()> {
            _BufferedIOBase::init(instance, raw, buffer_size, vm)
        }

        #[pymethod]
        fn fileno(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::fileno(instance, vm)
        }

        #[pyproperty]
        fn mode(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::mode(instance, vm)
        }

        #[pyproperty]
        fn name(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::name(instance, vm)
        }

        #[pymethod]
        fn tell(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            _BufferedIOBase::tell(instance, vm)
        }

        #[pymethod]
        fn close(instance: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
            _BufferedIOBase::close(instance, vm)
        }

        #[pymethod]
        fn write(instance: PyObjectRef, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            let raw = vm.get_attribute(instance, "raw").unwrap();

            //This should be replaced with a more appropriate chunking implementation
            vm.call_method(&raw, "write", (obj,))
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
            let raw = vm.get_attribute(instance, "raw")?;
            let args: Vec<_> = std::iter::once(offset).chain(how.into_option()).collect();
            vm.invoke(&vm.get_attribute(raw, "seek")?, args)
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
            object: OptionalArg<Option<PyObjectRef>>,
            _args: StringIOArgs,
            vm: &VirtualMachine,
        ) -> PyResult<StringIORef> {
            let raw_bytes = object
                .flatten()
                .map_or_else(Vec::new, |v| pystr::borrow_value(&v).as_bytes().to_vec());

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
            let buffer: Box<dyn Buffer> = Box::new(self.clone());
            let buffer = BufferRef::from(buffer);
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
            // TODO: UnsupportedOperation here
            vm.new_os_error(
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
    use crate::builtins::pystr::PyStrRef;
    use crate::builtins::pytype::PyTypeRef;
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
