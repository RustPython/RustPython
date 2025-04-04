// cspell:disable

pub(crate) use _overlapped::make_module;

#[allow(non_snake_case)]
#[pymodule]
mod _overlapped {
    // straight-forward port of Modules/overlapped.c

    use crate::vm::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytesRef, PyTypeRef},
        common::lock::PyMutex,
        convert::{ToPyException, ToPyObject},
        protocol::PyBuffer,
        stdlib::os::errno_err,
        types::Constructor,
    };
    use windows_sys::Win32::{
        Foundation::{self, GetLastError, HANDLE},
        Networking::WinSock::SOCKADDR_IN6,
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
        unsafe { std::mem::transmute(windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE) };

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

    impl std::fmt::Debug for Overlapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    impl std::fmt::Debug for OverlappedReadFrom {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

    impl std::fmt::Debug for OverlappedReadFromInto {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            #[cfg(target_pointer_width = "32")]
            let size = std::cmp::min(size, std::isize::MAX as _);

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
                return Err(errno_err(vm));
            }
            Ok(())
        }
    }

    impl Constructor for Overlapped {
        type Args = (isize,);

        fn py_new(cls: PyTypeRef, (mut event,): Self::Args, vm: &VirtualMachine) -> PyResult {
            if event == INVALID_HANDLE_VALUE {
                event = unsafe {
                    windows_sys::Win32::System::Threading::CreateEventA(
                        std::ptr::null(),
                        Foundation::TRUE,
                        Foundation::FALSE,
                        std::ptr::null(),
                    ) as isize
                };
                if event == NULL {
                    return Err(errno_err(vm));
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
            let overlapped = Overlapped {
                inner: PyMutex::new(inner),
            };
            overlapped.into_ref_with_type(vm, cls).map(Into::into)
        }
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
            return Err(errno_err(vm));
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
                return Err(errno_err(vm));
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
}
