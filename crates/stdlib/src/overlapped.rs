// spell-checker:disable

pub(crate) use _overlapped::make_module;

#[allow(non_snake_case)]
#[pymodule]
mod _overlapped {
    // straight-forward port of Modules/overlapped.c

    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytesRef, PyType},
        common::lock::PyMutex,
        convert::{ToPyException, ToPyObject},
        function::OptionalArg,
        protocol::PyBuffer,
        types::Constructor,
    };
    use windows_sys::Win32::{
        Foundation::{self, GetLastError, HANDLE},
        Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN, SOCKADDR_IN6},
        System::IO::OVERLAPPED,
    };

    #[pyattr]
    use windows_sys::Win32::{
        Foundation::{
            ERROR_IO_PENDING, ERROR_NETNAME_DELETED, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY,
            ERROR_PORT_UNREACHABLE, ERROR_SEM_TIMEOUT,
        },
        Networking::WinSock::{
            SO_UPDATE_ACCEPT_CONTEXT, SO_UPDATE_CONNECT_CONTEXT, TF_REUSE_SOCKET,
        },
        System::Threading::INFINITE,
    };

    #[pyattr]
    const INVALID_HANDLE_VALUE: isize =
        unsafe { core::mem::transmute(windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE) };

    #[pyattr]
    const NULL: isize = 0;

    #[pyattr]
    #[pyclass(name)]
    #[derive(PyPayload)]
    struct Overlapped {
        inner: PyMutex<OverlappedInner>,
    }

    struct OverlappedInner {
        overlapped: OVERLAPPED,
        handle: HANDLE,
        error: u32,
        data: OverlappedData,
    }

    unsafe impl Sync for OverlappedInner {}
    unsafe impl Send for OverlappedInner {}

    impl core::fmt::Debug for Overlapped {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            let zelf = self.inner.lock();
            f.debug_struct("Overlapped")
                // .field("overlapped", &(self.overlapped as *const _ as usize))
                .field("handle", &zelf.handle)
                .field("error", &zelf.error)
                .field("data", &zelf.data)
                .finish()
        }
    }

    #[allow(dead_code)] // TODO: remove when done
    #[derive(Debug)]
    enum OverlappedData {
        None,
        NotStarted,
        Read(PyBytesRef),
        ReadInto(PyBuffer),
        Write(PyBuffer),
        Accept(PyObjectRef),
        Connect,
        Disconnect,
        ConnectNamedPipe,
        WaitNamedPipeAndConnect,
        TransmitFile,
        ReadFrom(OverlappedReadFrom),
        WriteTo(PyBuffer),
        ReadFromInto(OverlappedReadFromInto),
    }

    struct OverlappedReadFrom {
        // A (buffer, (host, port)) tuple
        result: PyObjectRef,
        // The actual read buffer
        allocated_buffer: PyObjectRef,
        #[allow(dead_code)]
        address: SOCKADDR_IN6, // TODO: remove when done
        address_length: libc::c_int,
    }

    impl core::fmt::Debug for OverlappedReadFrom {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedReadFrom")
                .field("result", &self.result)
                .field("allocated_buffer", &self.allocated_buffer)
                // .field("address", &self.address)
                .field("address_length", &self.address_length)
                .finish()
        }
    }

    struct OverlappedReadFromInto {
        // A (number of bytes read, (host, port)) tuple
        result: PyObjectRef,
        /* Buffer passed by the user */
        user_buffer: PyBuffer,
        #[allow(dead_code)]
        address: SOCKADDR_IN6, // TODO: remove when done
        address_length: libc::c_int,
    }

    impl core::fmt::Debug for OverlappedReadFromInto {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedReadFromInto")
                .field("result", &self.result)
                .field("user_buffer", &self.user_buffer)
                // .field("address", &self.address)
                .field("address_length", &self.address_length)
                .finish()
        }
    }

    fn mark_as_completed(ov: &mut OVERLAPPED) {
        ov.Internal = 0;
        if !ov.hEvent.is_null() {
            unsafe { windows_sys::Win32::System::Threading::SetEvent(ov.hEvent) };
        }
    }

    fn from_windows_err(err: u32, vm: &VirtualMachine) -> PyBaseExceptionRef {
        use Foundation::{ERROR_CONNECTION_ABORTED, ERROR_CONNECTION_REFUSED};
        debug_assert_ne!(err, 0, "call errno_err instead");
        let exc = match err {
            ERROR_CONNECTION_REFUSED => vm.ctx.exceptions.connection_refused_error,
            ERROR_CONNECTION_ABORTED => vm.ctx.exceptions.connection_aborted_error,
            err => return std::io::Error::from_raw_os_error(err as i32).to_pyexception(vm),
        };
        // TODO: set errno and winerror
        vm.new_exception_empty(exc.to_owned())
    }

    fn HasOverlappedIoCompleted(overlapped: &OVERLAPPED) -> bool {
        overlapped.Internal != (Foundation::STATUS_PENDING as usize)
    }

    /// Parse a SOCKADDR_IN6 (which can also hold IPv4 addresses) to a Python address tuple
    fn unparse_address(
        addr: &SOCKADDR_IN6,
        _addr_len: libc::c_int,
        vm: &VirtualMachine,
    ) -> PyObjectRef {
        use crate::vm::convert::ToPyObject;

        unsafe {
            let family = addr.sin6_family;
            if family == AF_INET {
                // IPv4 address stored in SOCKADDR_IN6 structure
                let addr_in = &*(addr as *const SOCKADDR_IN6 as *const SOCKADDR_IN);
                let ip_bytes = addr_in.sin_addr.S_un.S_un_b;
                let ip_str = format!(
                    "{}.{}.{}.{}",
                    ip_bytes.s_b1, ip_bytes.s_b2, ip_bytes.s_b3, ip_bytes.s_b4
                );
                let port = u16::from_be(addr_in.sin_port);
                (ip_str, port).to_pyobject(vm)
            } else if family == AF_INET6 {
                // IPv6 address
                let ip_bytes = addr.sin6_addr.u.Byte;
                let ip_str = format!(
                    "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                    u16::from_be_bytes([ip_bytes[0], ip_bytes[1]]),
                    u16::from_be_bytes([ip_bytes[2], ip_bytes[3]]),
                    u16::from_be_bytes([ip_bytes[4], ip_bytes[5]]),
                    u16::from_be_bytes([ip_bytes[6], ip_bytes[7]]),
                    u16::from_be_bytes([ip_bytes[8], ip_bytes[9]]),
                    u16::from_be_bytes([ip_bytes[10], ip_bytes[11]]),
                    u16::from_be_bytes([ip_bytes[12], ip_bytes[13]]),
                    u16::from_be_bytes([ip_bytes[14], ip_bytes[15]]),
                );
                let port = u16::from_be(addr.sin6_port);
                let flowinfo = addr.sin6_flowinfo;
                let scope_id = addr.Anonymous.sin6_scope_id;
                (ip_str, port, flowinfo, scope_id).to_pyobject(vm)
            } else {
                // Unknown address family, return None
                vm.ctx.none()
            }
        }
    }

    #[pyclass(with(Constructor))]
    impl Overlapped {
        #[pygetset]
        fn address(&self, _vm: &VirtualMachine) -> usize {
            let inner = self.inner.lock();
            &inner.overlapped as *const _ as usize
        }

        #[pygetset]
        fn pending(&self, _vm: &VirtualMachine) -> bool {
            let inner = self.inner.lock();
            !HasOverlappedIoCompleted(&inner.overlapped)
                && !matches!(inner.data, OverlappedData::NotStarted)
        }

        fn WSARecv_inner(
            inner: &mut OverlappedInner,
            handle: isize,
            buf: &[u8],
            mut flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let wsabuf = windows_sys::Win32::Networking::WinSock::WSABUF {
                buf: buf.as_ptr() as *mut _,
                len: buf.len() as _,
            };
            let mut n_read: u32 = 0;
            // TODO: optimization with MaybeUninit
            let ret = unsafe {
                windows_sys::Win32::Networking::WinSock::WSARecv(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut n_read,
                    &mut flags,
                    &mut inner.overlapped,
                    None,
                )
            };
            let err = if ret < 0 {
                unsafe { windows_sys::Win32::Networking::WinSock::WSAGetLastError() as u32 }
            } else {
                Foundation::ERROR_SUCCESS
            };
            inner.error = err;
            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => Err(from_windows_err(err, vm)),
            }
        }

        #[pymethod]
        fn WSARecv(
            zelf: &Py<Self>,
            handle: isize,
            size: u32,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, std::isize::MAX as _);

            let buf = vec![0u8; std::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as _;

            let r = Self::WSARecv_inner(&mut inner, handle as _, buf.as_bytes(), flags, vm);
            inner.data = OverlappedData::Read(buf);
            r
        }

        #[pymethod]
        fn cancel(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            let inner = zelf.inner.lock();
            if matches!(
                inner.data,
                OverlappedData::NotStarted | OverlappedData::WaitNamedPipeAndConnect
            ) {
                return Ok(());
            }
            let ret = if !HasOverlappedIoCompleted(&inner.overlapped) {
                unsafe {
                    windows_sys::Win32::System::IO::CancelIoEx(inner.handle, &inner.overlapped)
                }
            } else {
                1
            };
            // CancelIoEx returns ERROR_NOT_FOUND if the I/O completed in-between
            if ret == 0 && unsafe { GetLastError() } != Foundation::ERROR_NOT_FOUND {
                return Err(vm.new_last_os_error());
            }
            Ok(())
        }

        #[pymethod]
        fn getresult(zelf: &Py<Self>, wait: OptionalArg<bool>, vm: &VirtualMachine) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::System::IO::GetOverlappedResult;

            let mut inner = zelf.inner.lock();
            let wait = wait.unwrap_or(false);

            // Check operation state
            if matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation not yet attempted".to_owned()));
            }
            if matches!(inner.data, OverlappedData::NotStarted) {
                return Err(vm.new_value_error("operation failed to start".to_owned()));
            }

            // Get the result
            let mut transferred: u32 = 0;
            let ret = unsafe {
                GetOverlappedResult(
                    inner.handle,
                    &inner.overlapped,
                    &mut transferred,
                    if wait { 1 } else { 0 },
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { GetLastError() }
            };
            inner.error = err;

            // Handle errors
            match err {
                ERROR_SUCCESS | ERROR_MORE_DATA => {}
                ERROR_BROKEN_PIPE => {
                    // For read operations, broken pipe is acceptable
                    match &inner.data {
                        OverlappedData::Read(_) | OverlappedData::ReadInto(_) => {}
                        OverlappedData::ReadFrom(rf)
                            if rf.result.is(&vm.ctx.none())
                                || rf.allocated_buffer.is(&vm.ctx.none()) =>
                        {
                            return Err(from_windows_err(err, vm));
                        }
                        OverlappedData::ReadFrom(_) => {}
                        OverlappedData::ReadFromInto(rfi) if rfi.result.is(&vm.ctx.none()) => {
                            return Err(from_windows_err(err, vm));
                        }
                        OverlappedData::ReadFromInto(_) => {}
                        _ => return Err(from_windows_err(err, vm)),
                    }
                }
                ERROR_IO_PENDING => {
                    return Err(from_windows_err(err, vm));
                }
                _ => return Err(from_windows_err(err, vm)),
            }

            // Return result based on operation type
            match &inner.data {
                OverlappedData::Read(buf) => {
                    // Resize buffer to actual bytes read
                    let bytes = buf.as_bytes();
                    let result = if transferred as usize != bytes.len() {
                        vm.ctx.new_bytes(bytes[..transferred as usize].to_vec())
                    } else {
                        buf.clone()
                    };
                    Ok(result.into())
                }
                OverlappedData::ReadInto(_) => {
                    // Return number of bytes read
                    Ok(vm.ctx.new_int(transferred).into())
                }
                OverlappedData::Write(_) => {
                    // Return number of bytes written
                    Ok(vm.ctx.new_int(transferred).into())
                }
                OverlappedData::Accept(_) => {
                    // Return None for accept
                    Ok(vm.ctx.none())
                }
                OverlappedData::Connect => {
                    // Return None for connect
                    Ok(vm.ctx.none())
                }
                OverlappedData::Disconnect => {
                    // Return None for disconnect
                    Ok(vm.ctx.none())
                }
                OverlappedData::ConnectNamedPipe => {
                    // Return None for connect named pipe
                    Ok(vm.ctx.none())
                }
                OverlappedData::WaitNamedPipeAndConnect => {
                    // Return None
                    Ok(vm.ctx.none())
                }
                OverlappedData::ReadFrom(rf) => {
                    // Return (resized_buffer, (host, port)) tuple
                    let buf = rf
                        .allocated_buffer
                        .downcast_ref::<crate::vm::builtins::PyBytes>()
                        .unwrap();
                    let bytes = buf.as_bytes();
                    let resized_buf = if transferred as usize != bytes.len() {
                        vm.ctx.new_bytes(bytes[..transferred as usize].to_vec())
                    } else {
                        buf.to_owned()
                    };
                    let addr_tuple = unparse_address(&rf.address, rf.address_length, vm);
                    Ok(vm
                        .ctx
                        .new_tuple(vec![resized_buf.into(), addr_tuple])
                        .into())
                }
                OverlappedData::ReadFromInto(rfi) => {
                    // Return (transferred, (host, port)) tuple
                    let addr_tuple = unparse_address(&rfi.address, rfi.address_length, vm);
                    Ok(vm
                        .ctx
                        .new_tuple(vec![vm.ctx.new_int(transferred).into(), addr_tuple])
                        .into())
                }
                _ => Ok(vm.ctx.none()),
            }
        }
    }

    impl Constructor for Overlapped {
        type Args = (isize,);

        fn py_new(
            _cls: &Py<PyType>,
            (mut event,): Self::Args,
            vm: &VirtualMachine,
        ) -> PyResult<Self> {
            if event == INVALID_HANDLE_VALUE {
                event = unsafe {
                    windows_sys::Win32::System::Threading::CreateEventA(
                        core::ptr::null(),
                        Foundation::TRUE,
                        Foundation::FALSE,
                        core::ptr::null(),
                    ) as isize
                };
                if event == NULL {
                    return Err(vm.new_last_os_error());
                }
            }

            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            if event != NULL {
                overlapped.hEvent = event as _;
            }
            let inner = OverlappedInner {
                overlapped,
                handle: NULL as _,
                error: 0,
                data: OverlappedData::None,
            };
            Ok(Overlapped {
                inner: PyMutex::new(inner),
            })
        }
    }

    unsafe fn u64_to_handle(raw_ptr_value: u64) -> HANDLE {
        raw_ptr_value as HANDLE
    }

    #[pyfunction]
    fn CreateIoCompletionPort(
        handle: isize,
        port: isize,
        key: usize,
        concurrency: u32,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        let r = unsafe {
            windows_sys::Win32::System::IO::CreateIoCompletionPort(
                handle as _,
                port as _,
                key,
                concurrency,
            ) as isize
        };
        if r as usize == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(r)
    }

    #[pyfunction]
    fn GetQueuedCompletionStatus(port: isize, msecs: u32, vm: &VirtualMachine) -> PyResult {
        let mut bytes_transferred = 0;
        let mut completion_key = 0;
        let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();
        let ret = unsafe {
            windows_sys::Win32::System::IO::GetQueuedCompletionStatus(
                port as _,
                &mut bytes_transferred,
                &mut completion_key,
                &mut overlapped,
                msecs,
            )
        };
        let err = if ret != 0 {
            Foundation::ERROR_SUCCESS
        } else {
            unsafe { Foundation::GetLastError() }
        };
        if overlapped.is_null() {
            if err == Foundation::WAIT_TIMEOUT {
                return Ok(vm.ctx.none());
            } else {
                return Err(vm.new_last_os_error());
            }
        }

        let value = vm.ctx.new_tuple(vec![
            err.to_pyobject(vm),
            completion_key.to_pyobject(vm),
            bytes_transferred.to_pyobject(vm),
            (overlapped as usize).to_pyobject(vm),
        ]);
        Ok(value.into())
    }

    #[pyfunction]
    fn CreateEvent(
        event_attributes: PyObjectRef,
        manual_reset: bool,
        initial_state: bool,
        name: Option<String>,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        if !vm.is_none(&event_attributes) {
            return Err(vm.new_value_error("EventAttributes must be None"));
        }

        let name = match name {
            Some(name) => {
                let name = widestring::WideCString::from_str(&name).unwrap();
                name.as_ptr()
            }
            None => core::ptr::null(),
        };
        let event = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                core::ptr::null(),
                manual_reset as _,
                initial_state as _,
                name,
            ) as isize
        };
        if event == NULL {
            return Err(vm.new_last_os_error());
        }
        Ok(event)
    }

    #[pyfunction]
    fn SetEvent(handle: u64, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { windows_sys::Win32::System::Threading::SetEvent(u64_to_handle(handle)) };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(())
    }

    #[pyfunction]
    fn ResetEvent(handle: u64, vm: &VirtualMachine) -> PyResult<()> {
        let ret =
            unsafe { windows_sys::Win32::System::Threading::ResetEvent(u64_to_handle(handle)) };
        if ret == 0 {
            return Err(vm.new_last_os_error());
        }
        Ok(())
    }
}
