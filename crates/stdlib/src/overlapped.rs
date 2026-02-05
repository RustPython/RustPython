// spell-checker:disable

pub(crate) use _overlapped::module_def;

#[allow(non_snake_case)]
#[pymodule]
mod _overlapped {
    // straight-forward port of Modules/overlapped.c

    use crate::vm::{
        AsObject, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyBaseExceptionRef, PyBytesRef, PyModule, PyStrRef, PyTupleRef, PyType},
        common::lock::PyMutex,
        convert::ToPyObject,
        function::OptionalArg,
        object::{Traverse, TraverseFn},
        protocol::PyBuffer,
        types::{Constructor, Destructor},
    };
    use windows_sys::Win32::{
        Foundation::{self, GetLastError, HANDLE},
        Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6},
        System::IO::OVERLAPPED,
    };

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_socket", 0)?;
        initialize_winsock_extensions(vm)?;
        __module_exec(vm, module);
        Ok(())
    }

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

    // Function pointers for Winsock extension functions
    static ACCEPT_EX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static CONNECT_EX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static DISCONNECT_EX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static TRANSMIT_FILE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

    fn initialize_winsock_extensions(vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Networking::WinSock::{
            INVALID_SOCKET, IPPROTO_TCP, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCK_STREAM,
            SOCKET_ERROR, WSAGetLastError, WSAIoctl, closesocket, socket,
        };

        // GUIDs for extension functions
        const WSAID_ACCEPTEX: windows_sys::core::GUID = windows_sys::core::GUID {
            data1: 0xb5367df1,
            data2: 0xcbac,
            data3: 0x11cf,
            data4: [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
        };
        const WSAID_CONNECTEX: windows_sys::core::GUID = windows_sys::core::GUID {
            data1: 0x25a207b9,
            data2: 0xddf3,
            data3: 0x4660,
            data4: [0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e],
        };
        const WSAID_DISCONNECTEX: windows_sys::core::GUID = windows_sys::core::GUID {
            data1: 0x7fda2e11,
            data2: 0x8630,
            data3: 0x436f,
            data4: [0xa0, 0x31, 0xf5, 0x36, 0xa6, 0xee, 0xc1, 0x57],
        };
        const WSAID_TRANSMITFILE: windows_sys::core::GUID = windows_sys::core::GUID {
            data1: 0xb5367df0,
            data2: 0xcbac,
            data3: 0x11cf,
            data4: [0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92],
        };

        // Check all four locks to prevent partial initialization
        if ACCEPT_EX.get().is_some()
            && CONNECT_EX.get().is_some()
            && DISCONNECT_EX.get().is_some()
            && TRANSMIT_FILE.get().is_some()
        {
            return Ok(());
        }

        let s = unsafe { socket(AF_INET as i32, SOCK_STREAM, IPPROTO_TCP) };
        if s == INVALID_SOCKET {
            let err = unsafe { WSAGetLastError() } as u32;
            return Err(set_from_windows_err(err, vm));
        }

        let mut dw_bytes: u32 = 0;

        macro_rules! get_extension {
            ($guid:expr, $lock:expr) => {{
                let mut func_ptr: usize = 0;
                let ret = unsafe {
                    WSAIoctl(
                        s,
                        SIO_GET_EXTENSION_FUNCTION_POINTER,
                        &$guid as *const _ as *const _,
                        std::mem::size_of_val(&$guid) as u32,
                        &mut func_ptr as *mut _ as *mut _,
                        std::mem::size_of::<usize>() as u32,
                        &mut dw_bytes,
                        std::ptr::null_mut(),
                        None,
                    )
                };
                if ret == SOCKET_ERROR {
                    let err = unsafe { WSAGetLastError() } as u32;
                    unsafe { closesocket(s) };
                    return Err(set_from_windows_err(err, vm));
                }
                let _ = $lock.set(func_ptr);
            }};
        }

        get_extension!(WSAID_ACCEPTEX, ACCEPT_EX);
        get_extension!(WSAID_CONNECTEX, CONNECT_EX);
        get_extension!(WSAID_DISCONNECTEX, DISCONNECT_EX);
        get_extension!(WSAID_TRANSMITFILE, TRANSMIT_FILE);

        unsafe { closesocket(s) };
        Ok(())
    }

    #[pyattr]
    #[pyclass(name, traverse)]
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

    unsafe impl Traverse for OverlappedInner {
        fn traverse(&self, traverse_fn: &mut TraverseFn<'_>) {
            match &self.data {
                OverlappedData::Read(buf) | OverlappedData::Accept(buf) => {
                    buf.traverse(traverse_fn);
                }
                OverlappedData::ReadInto(buf) | OverlappedData::Write(buf) => {
                    buf.traverse(traverse_fn);
                }
                OverlappedData::WriteTo(wt) => {
                    wt.buf.traverse(traverse_fn);
                }
                OverlappedData::ReadFrom(rf) => {
                    if let Some(result) = &rf.result {
                        result.traverse(traverse_fn);
                    }
                    rf.allocated_buffer.traverse(traverse_fn);
                }
                OverlappedData::ReadFromInto(rfi) => {
                    if let Some(result) = &rfi.result {
                        result.traverse(traverse_fn);
                    }
                    rfi.user_buffer.traverse(traverse_fn);
                }
                _ => {}
            }
        }
    }

    impl core::fmt::Debug for Overlapped {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            let zelf = self.inner.lock();
            f.debug_struct("Overlapped")
                .field("handle", &zelf.handle)
                .field("error", &zelf.error)
                .field("data", &zelf.data)
                .finish()
        }
    }

    #[derive(Debug)]
    enum OverlappedData {
        None,
        NotStarted,
        Read(PyBytesRef),
        // Fields below store buffers that must be kept alive during async operations
        #[allow(dead_code)]
        ReadInto(PyBuffer),
        #[allow(dead_code)]
        Write(PyBuffer),
        #[allow(dead_code)]
        Accept(PyBytesRef),
        Connect(Vec<u8>), // Store address bytes to keep them alive during async operation
        Disconnect,
        ConnectNamedPipe,
        #[allow(dead_code)] // Reserved for named pipe support
        WaitNamedPipeAndConnect,
        TransmitFile,
        ReadFrom(OverlappedReadFrom),
        WriteTo(OverlappedWriteTo), // Store address bytes for WSASendTo
        ReadFromInto(OverlappedReadFromInto),
    }

    struct OverlappedReadFrom {
        // A (buffer, (host, port)) tuple
        result: Option<PyObjectRef>,
        // The actual read buffer
        allocated_buffer: PyBytesRef,
        address: SOCKADDR_IN6,
        address_length: i32,
    }

    impl core::fmt::Debug for OverlappedReadFrom {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedReadFrom")
                .field("result", &self.result)
                .field("allocated_buffer", &self.allocated_buffer)
                .field("address_length", &self.address_length)
                .finish()
        }
    }

    struct OverlappedReadFromInto {
        // A (number of bytes read, (host, port)) tuple
        result: Option<PyObjectRef>,
        /* Buffer passed by the user */
        user_buffer: PyBuffer,
        address: SOCKADDR_IN6,
        address_length: i32,
    }

    impl core::fmt::Debug for OverlappedReadFromInto {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedReadFromInto")
                .field("result", &self.result)
                .field("user_buffer", &self.user_buffer)
                .field("address_length", &self.address_length)
                .finish()
        }
    }

    struct OverlappedWriteTo {
        buf: PyBuffer,
        address: Vec<u8>, // Keep address alive during async operation
    }

    impl core::fmt::Debug for OverlappedWriteTo {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("OverlappedWriteTo")
                .field("buf", &self.buf)
                .field("address", &self.address.len())
                .finish()
        }
    }

    fn mark_as_completed(ov: &mut OVERLAPPED) {
        ov.Internal = 0;
        if !ov.hEvent.is_null() {
            unsafe { windows_sys::Win32::System::Threading::SetEvent(ov.hEvent) };
        }
    }

    fn set_from_windows_err(err: u32, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let err = if err == 0 {
            unsafe { GetLastError() }
        } else {
            err
        };
        let errno = crate::vm::common::os::winerror_to_errno(err as i32);
        let message = std::io::Error::from_raw_os_error(err as i32).to_string();
        let exc = vm.new_errno_error(errno, message);
        let _ = exc
            .as_object()
            .set_attr("winerror", err.to_pyobject(vm), vm);
        exc.upcast()
    }

    fn HasOverlappedIoCompleted(overlapped: &OVERLAPPED) -> bool {
        overlapped.Internal != (Foundation::STATUS_PENDING as usize)
    }

    /// Parse a Python address tuple to SOCKADDR
    fn parse_address(addr_obj: &PyTupleRef, vm: &VirtualMachine) -> PyResult<(Vec<u8>, i32)> {
        use windows_sys::Win32::Networking::WinSock::{WSAGetLastError, WSAStringToAddressW};

        match addr_obj.len() {
            2 => {
                // IPv4: (host, port)
                let host: PyStrRef = addr_obj[0].clone().try_into_value(vm)?;
                let port: u16 = addr_obj[1].clone().try_to_value(vm)?;

                let mut addr: SOCKADDR_IN = unsafe { std::mem::zeroed() };
                addr.sin_family = AF_INET;

                let host_wide: Vec<u16> = host.as_str().encode_utf16().chain([0]).collect();
                let mut addr_len = std::mem::size_of::<SOCKADDR_IN>() as i32;

                let ret = unsafe {
                    WSAStringToAddressW(
                        host_wide.as_ptr(),
                        AF_INET as i32,
                        std::ptr::null(),
                        &mut addr as *mut _ as *mut SOCKADDR,
                        &mut addr_len,
                    )
                };

                if ret < 0 {
                    let err = unsafe { WSAGetLastError() } as u32;
                    return Err(set_from_windows_err(err, vm));
                }

                // Restore port (WSAStringToAddressW overwrites it)
                addr.sin_port = port.to_be();

                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &addr as *const _ as *const u8,
                        std::mem::size_of::<SOCKADDR_IN>(),
                    )
                };
                Ok((bytes.to_vec(), addr_len))
            }
            4 => {
                // IPv6: (host, port, flowinfo, scope_id)
                let host: PyStrRef = addr_obj[0].clone().try_into_value(vm)?;
                let port: u16 = addr_obj[1].clone().try_to_value(vm)?;
                let flowinfo: u32 = addr_obj[2].clone().try_to_value(vm)?;
                let scope_id: u32 = addr_obj[3].clone().try_to_value(vm)?;

                let mut addr: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
                addr.sin6_family = AF_INET6;

                let host_wide: Vec<u16> = host.as_str().encode_utf16().chain([0]).collect();
                let mut addr_len = std::mem::size_of::<SOCKADDR_IN6>() as i32;

                let ret = unsafe {
                    WSAStringToAddressW(
                        host_wide.as_ptr(),
                        AF_INET6 as i32,
                        std::ptr::null(),
                        &mut addr as *mut _ as *mut SOCKADDR,
                        &mut addr_len,
                    )
                };

                if ret < 0 {
                    let err = unsafe { WSAGetLastError() } as u32;
                    return Err(set_from_windows_err(err, vm));
                }

                // Restore fields that WSAStringToAddressW might overwrite
                addr.sin6_port = port.to_be();
                addr.sin6_flowinfo = flowinfo;
                addr.Anonymous.sin6_scope_id = scope_id;

                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        &addr as *const _ as *const u8,
                        std::mem::size_of::<SOCKADDR_IN6>(),
                    )
                };
                Ok((bytes.to_vec(), addr_len))
            }
            _ => Err(vm.new_value_error("illegal address_as_bytes argument".to_owned())),
        }
    }

    /// Parse a SOCKADDR_IN6 (which can also hold IPv4 addresses) to a Python address tuple
    fn unparse_address(addr: &SOCKADDR_IN6, _addr_len: i32, vm: &VirtualMachine) -> PyResult {
        use std::net::{Ipv4Addr, Ipv6Addr};

        unsafe {
            let family = addr.sin6_family;
            if family == AF_INET {
                // IPv4 address stored in SOCKADDR_IN6 structure
                let addr_in = &*(addr as *const SOCKADDR_IN6 as *const SOCKADDR_IN);
                let ip_bytes = addr_in.sin_addr.S_un.S_un_b;
                let ip_str =
                    Ipv4Addr::new(ip_bytes.s_b1, ip_bytes.s_b2, ip_bytes.s_b3, ip_bytes.s_b4)
                        .to_string();
                let port = u16::from_be(addr_in.sin_port);
                Ok((ip_str, port).to_pyobject(vm))
            } else if family == AF_INET6 {
                // IPv6 address
                let ip_bytes = addr.sin6_addr.u.Byte;
                let ip_str = Ipv6Addr::from(ip_bytes).to_string();
                let port = u16::from_be(addr.sin6_port);
                let flowinfo = u32::from_be(addr.sin6_flowinfo);
                let scope_id = addr.Anonymous.sin6_scope_id;
                Ok((ip_str, port, flowinfo, scope_id).to_pyobject(vm))
            } else {
                Err(vm.new_value_error("recvfrom returned unsupported address family".to_owned()))
            }
        }
    }

    #[pyclass(with(Constructor, Destructor))]
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

        #[pygetset]
        fn error(&self, _vm: &VirtualMachine) -> u32 {
            let inner = self.inner.lock();
            inner.error
        }

        #[pygetset]
        fn event(&self, _vm: &VirtualMachine) -> isize {
            let inner = self.inner.lock();
            inner.overlapped.hEvent as isize
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
                return Err(set_from_windows_err(0, vm));
            }
            Ok(())
        }

        #[pymethod]
        fn getresult(zelf: &Py<Self>, wait: OptionalArg<bool>, vm: &VirtualMachine) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

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
                windows_sys::Win32::System::IO::GetOverlappedResult(
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
                    let allow_broken_pipe = match &inner.data {
                        OverlappedData::Read(_) | OverlappedData::ReadInto(_) => true,
                        OverlappedData::ReadFrom(_) => true,
                        OverlappedData::ReadFromInto(rfi) => rfi.result.is_some(),
                        _ => false,
                    };
                    if !allow_broken_pipe {
                        return Err(set_from_windows_err(err, vm));
                    }
                }
                _ => return Err(set_from_windows_err(err, vm)),
            }

            // Return result based on operation type
            match &mut inner.data {
                OverlappedData::Read(buf) => {
                    let len = buf.as_bytes().len();
                    let result = if transferred as usize != len {
                        let resized = vm
                            .ctx
                            .new_bytes(buf.as_bytes()[..transferred as usize].to_vec());
                        *buf = resized.clone();
                        resized
                    } else {
                        buf.clone()
                    };
                    Ok(result.into())
                }
                OverlappedData::ReadFrom(rf) => {
                    let len = rf.allocated_buffer.as_bytes().len();
                    let resized_buf = if transferred as usize != len {
                        let resized = vm.ctx.new_bytes(
                            rf.allocated_buffer.as_bytes()[..transferred as usize].to_vec(),
                        );
                        rf.allocated_buffer = resized.clone();
                        resized
                    } else {
                        rf.allocated_buffer.clone()
                    };
                    let addr_tuple = unparse_address(&rf.address, rf.address_length, vm)?;
                    if let Some(result) = &rf.result {
                        return Ok(result.clone());
                    }
                    let result = vm.ctx.new_tuple(vec![resized_buf.into(), addr_tuple]);
                    rf.result = Some(result.clone().into());
                    Ok(result.into())
                }
                OverlappedData::ReadFromInto(rfi) => {
                    let addr_tuple = unparse_address(&rfi.address, rfi.address_length, vm)?;
                    if let Some(result) = &rfi.result {
                        return Ok(result.clone());
                    }
                    let result = vm
                        .ctx
                        .new_tuple(vec![vm.ctx.new_int(transferred).into(), addr_tuple]);
                    rfi.result = Some(result.clone().into());
                    Ok(result.into())
                }
                _ => Ok(vm.ctx.new_int(transferred).into()),
            }
        }

        // ReadFile
        #[pymethod]
        fn ReadFile(zelf: &Py<Self>, handle: isize, size: u32, vm: &VirtualMachine) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Storage::FileSystem::ReadFile;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; std::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as HANDLE;
            inner.data = OverlappedData::Read(buf.clone());

            let mut nread: u32 = 0;
            let ret = unsafe {
                ReadFile(
                    handle as HANDLE,
                    buf.as_bytes().as_ptr() as *mut _,
                    size,
                    &mut nread,
                    &mut inner.overlapped,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { GetLastError() }
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // ReadFileInto
        #[pymethod]
        fn ReadFileInto(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Storage::FileSystem::ReadFile;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = handle as HANDLE;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            // For async read, buffer must be contiguous - we can't use a temporary copy
            // because Windows writes data directly to the buffer after this call returns
            let Some(contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            inner.data = OverlappedData::ReadInto(buf.clone());

            let mut nread: u32 = 0;
            let ret = unsafe {
                ReadFile(
                    handle as HANDLE,
                    contiguous.as_ptr() as *mut _,
                    buf_len as u32,
                    &mut nread,
                    &mut inner.overlapped,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { GetLastError() }
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSARecv
        #[pymethod]
        fn WSARecv(
            zelf: &Py<Self>,
            handle: isize,
            size: u32,
            flags: OptionalArg<u32>,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecv};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let mut flags = flags.unwrap_or(0);

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; std::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as HANDLE;
            inner.data = OverlappedData::Read(buf.clone());

            let wsabuf = WSABUF {
                buf: buf.as_bytes().as_ptr() as *mut _,
                len: size,
            };
            let mut nread: u32 = 0;

            let ret = unsafe {
                WSARecv(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut nread,
                    &mut flags,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSARecvInto
        #[pymethod]
        fn WSARecvInto(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecv};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let mut flags = flags;
            inner.handle = handle as HANDLE;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            let Some(contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            inner.data = OverlappedData::ReadInto(buf.clone());

            let wsabuf = WSABUF {
                buf: contiguous.as_ptr() as *mut _,
                len: buf_len as u32,
            };
            let mut nread: u32 = 0;

            let ret = unsafe {
                WSARecv(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut nread,
                    &mut flags,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WriteFile
        #[pymethod]
        fn WriteFile(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Storage::FileSystem::WriteFile;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = handle as HANDLE;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            // For async write, buffer must be contiguous - we can't use a temporary copy
            // because Windows reads from the buffer after this call returns
            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            inner.data = OverlappedData::Write(buf.clone());

            let mut written: u32 = 0;
            let ret = unsafe {
                WriteFile(
                    handle as HANDLE,
                    contiguous.as_ptr() as *const _,
                    buf_len as u32,
                    &mut written,
                    &mut inner.overlapped,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { GetLastError() }
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSASend
        #[pymethod]
        fn WSASend(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSASend};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = handle as HANDLE;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            inner.data = OverlappedData::Write(buf.clone());

            let wsabuf = WSABUF {
                buf: contiguous.as_ptr() as *mut _,
                len: buf_len as u32,
            };
            let mut written: u32 = 0;

            let ret = unsafe {
                WSASend(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut written,
                    flags,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // AcceptEx
        #[pymethod]
        fn AcceptEx(
            zelf: &Py<Self>,
            listen_socket: isize,
            accept_socket: isize,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            // Buffer size: local address + remote address
            let size = std::mem::size_of::<SOCKADDR_IN6>() + 16;
            let buf = vec![0u8; size * 2];
            let buf = vm.ctx.new_bytes(buf);

            inner.handle = listen_socket as HANDLE;
            inner.data = OverlappedData::Accept(buf.clone());

            let mut bytes_received: u32 = 0;

            type AcceptExFn = unsafe extern "system" fn(
                sListenSocket: usize,
                sAcceptSocket: usize,
                lpOutputBuffer: *mut core::ffi::c_void,
                dwReceiveDataLength: u32,
                dwLocalAddressLength: u32,
                dwRemoteAddressLength: u32,
                lpdwBytesReceived: *mut u32,
                lpOverlapped: *mut OVERLAPPED,
            ) -> i32;

            let accept_ex: AcceptExFn = unsafe { std::mem::transmute(*ACCEPT_EX.get().unwrap()) };

            let ret = unsafe {
                accept_ex(
                    listen_socket as _,
                    accept_socket as _,
                    buf.as_bytes().as_ptr() as *mut _,
                    0,
                    size as u32,
                    size as u32,
                    &mut bytes_received,
                    &mut inner.overlapped,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { WSAGetLastError() as u32 }
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // ConnectEx
        #[pymethod]
        fn ConnectEx(
            zelf: &Py<Self>,
            socket: isize,
            address: PyTupleRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let (addr_bytes, addr_len) = parse_address(&address, vm)?;

            inner.handle = socket as HANDLE;
            // Store addr_bytes in OverlappedData to keep it alive during async operation
            inner.data = OverlappedData::Connect(addr_bytes);

            type ConnectExFn = unsafe extern "system" fn(
                s: usize,
                name: *const SOCKADDR,
                namelen: i32,
                lpSendBuffer: *const core::ffi::c_void,
                dwSendDataLength: u32,
                lpdwBytesSent: *mut u32,
                lpOverlapped: *mut OVERLAPPED,
            ) -> i32;

            let connect_ex: ConnectExFn =
                unsafe { std::mem::transmute(*CONNECT_EX.get().unwrap()) };

            // Get pointer to the stored address data
            let addr_ptr = match &inner.data {
                OverlappedData::Connect(bytes) => bytes.as_ptr(),
                _ => unreachable!(),
            };

            let ret = unsafe {
                connect_ex(
                    socket as _,
                    addr_ptr as *const SOCKADDR,
                    addr_len,
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    &mut inner.overlapped,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { WSAGetLastError() as u32 }
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // DisconnectEx
        #[pymethod]
        fn DisconnectEx(
            zelf: &Py<Self>,
            socket: isize,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = socket as HANDLE;
            inner.data = OverlappedData::Disconnect;

            type DisconnectExFn = unsafe extern "system" fn(
                s: usize,
                lpOverlapped: *mut OVERLAPPED,
                dwFlags: u32,
                dwReserved: u32,
            ) -> i32;

            let disconnect_ex: DisconnectExFn =
                unsafe { std::mem::transmute(*DISCONNECT_EX.get().unwrap()) };

            let ret = unsafe { disconnect_ex(socket as _, &mut inner.overlapped, flags, 0) };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { WSAGetLastError() as u32 }
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // TransmitFile
        #[allow(clippy::too_many_arguments)]
        #[pymethod]
        fn TransmitFile(
            zelf: &Py<Self>,
            socket: isize,
            file: isize,
            offset: u32,
            offset_high: u32,
            count_to_write: u32,
            count_per_send: u32,
            flags: u32,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = socket as HANDLE;
            inner.data = OverlappedData::TransmitFile;
            inner.overlapped.Anonymous.Anonymous.Offset = offset;
            inner.overlapped.Anonymous.Anonymous.OffsetHigh = offset_high;

            type TransmitFileFn = unsafe extern "system" fn(
                hSocket: usize,
                hFile: HANDLE,
                nNumberOfBytesToWrite: u32,
                nNumberOfBytesPerSend: u32,
                lpOverlapped: *mut OVERLAPPED,
                lpTransmitBuffers: *const core::ffi::c_void,
                dwReserved: u32,
            ) -> i32;

            let transmit_file: TransmitFileFn =
                unsafe { std::mem::transmute(*TRANSMIT_FILE.get().unwrap()) };

            let ret = unsafe {
                transmit_file(
                    socket as _,
                    file as HANDLE,
                    count_to_write,
                    count_per_send,
                    &mut inner.overlapped,
                    std::ptr::null(),
                    flags,
                )
            };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { WSAGetLastError() as u32 }
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // ConnectNamedPipe
        #[pymethod]
        fn ConnectNamedPipe(zelf: &Py<Self>, pipe: isize, vm: &VirtualMachine) -> PyResult<bool> {
            use windows_sys::Win32::Foundation::{
                ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, ERROR_SUCCESS,
            };
            use windows_sys::Win32::System::Pipes::ConnectNamedPipe;

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            inner.handle = pipe as HANDLE;
            inner.data = OverlappedData::ConnectNamedPipe;

            let ret = unsafe { ConnectNamedPipe(pipe as HANDLE, &mut inner.overlapped) };

            let err = if ret != 0 {
                ERROR_SUCCESS
            } else {
                unsafe { GetLastError() }
            };
            inner.error = err;

            match err {
                ERROR_PIPE_CONNECTED => {
                    mark_as_completed(&mut inner.overlapped);
                    Ok(true)
                }
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(false),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSASendTo
        #[pymethod]
        fn WSASendTo(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            flags: u32,
            address: PyTupleRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_SUCCESS};
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSASendTo};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let (addr_bytes, addr_len) = parse_address(&address, vm)?;

            inner.handle = handle as HANDLE;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            // Store both buffer and address in OverlappedData to keep them alive
            inner.data = OverlappedData::WriteTo(OverlappedWriteTo {
                buf: buf.clone(),
                address: addr_bytes,
            });

            let wsabuf = WSABUF {
                buf: contiguous.as_ptr() as *mut _,
                len: buf_len as u32,
            };
            let mut written: u32 = 0;

            // Get pointer to the stored address data
            let addr_ptr = match &inner.data {
                OverlappedData::WriteTo(wt) => wt.address.as_ptr(),
                _ => unreachable!(),
            };

            let ret = unsafe {
                WSASendTo(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut written,
                    flags,
                    addr_ptr as *const SOCKADDR,
                    addr_len,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_SUCCESS | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSARecvFrom
        #[pymethod]
        fn WSARecvFrom(
            zelf: &Py<Self>,
            handle: isize,
            size: u32,
            flags: OptionalArg<u32>,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecvFrom};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let mut flags = flags.unwrap_or(0);

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; std::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as HANDLE;

            let address: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
            let address_length = std::mem::size_of::<SOCKADDR_IN6>() as i32;

            inner.data = OverlappedData::ReadFrom(OverlappedReadFrom {
                result: None,
                allocated_buffer: buf.clone(),
                address,
                address_length,
            });

            let wsabuf = WSABUF {
                buf: buf.as_bytes().as_ptr() as *mut _,
                len: size,
            };
            let mut nread: u32 = 0;

            // Get mutable reference to address in inner.data
            let (addr_ptr, addr_len_ptr) = match &mut inner.data {
                OverlappedData::ReadFrom(rf) => (
                    &mut rf.address as *mut SOCKADDR_IN6,
                    &mut rf.address_length as *mut i32,
                ),
                _ => unreachable!(),
            };

            let ret = unsafe {
                WSARecvFrom(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut nread,
                    &mut flags,
                    addr_ptr as *mut SOCKADDR,
                    addr_len_ptr,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }

        // WSARecvFromInto
        #[pymethod]
        fn WSARecvFromInto(
            zelf: &Py<Self>,
            handle: isize,
            buf: PyBuffer,
            size: u32,
            flags: OptionalArg<u32>,
            vm: &VirtualMachine,
        ) -> PyResult {
            use windows_sys::Win32::Foundation::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };
            use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecvFrom};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted".to_owned()));
            }

            let mut flags = flags.unwrap_or(0);
            inner.handle = handle as HANDLE;

            let Some(contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous".to_owned()));
            };

            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large".to_owned()));
            }

            let address: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
            let address_length = std::mem::size_of::<SOCKADDR_IN6>() as i32;

            inner.data = OverlappedData::ReadFromInto(OverlappedReadFromInto {
                result: None,
                user_buffer: buf.clone(),
                address,
                address_length,
            });

            let wsabuf = WSABUF {
                buf: contiguous.as_ptr() as *mut _,
                len: size,
            };
            let mut nread: u32 = 0;

            // Get mutable reference to address in inner.data
            let (addr_ptr, addr_len_ptr) = match &mut inner.data {
                OverlappedData::ReadFromInto(rfi) => (
                    &mut rfi.address as *mut SOCKADDR_IN6,
                    &mut rfi.address_length as *mut i32,
                ),
                _ => unreachable!(),
            };

            let ret = unsafe {
                WSARecvFrom(
                    handle as _,
                    &wsabuf,
                    1,
                    &mut nread,
                    &mut flags,
                    addr_ptr as *mut SOCKADDR,
                    addr_len_ptr,
                    &mut inner.overlapped,
                    None,
                )
            };

            let err = if ret < 0 {
                unsafe { WSAGetLastError() as u32 }
            } else {
                ERROR_SUCCESS
            };
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    mark_as_completed(&mut inner.overlapped);
                    Err(set_from_windows_err(err, vm))
                }
                ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_IO_PENDING => Ok(vm.ctx.none()),
                _ => {
                    inner.data = OverlappedData::NotStarted;
                    Err(set_from_windows_err(err, vm))
                }
            }
        }
    }

    impl Constructor for Overlapped {
        type Args = (OptionalArg<isize>,);

        fn py_new(_cls: &Py<PyType>, (event,): Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            let mut event = event.unwrap_or(INVALID_HANDLE_VALUE);

            if event == INVALID_HANDLE_VALUE {
                event = unsafe {
                    windows_sys::Win32::System::Threading::CreateEventW(
                        core::ptr::null(),
                        Foundation::TRUE,
                        Foundation::FALSE,
                        core::ptr::null(),
                    ) as isize
                };
                if event == NULL {
                    return Err(set_from_windows_err(0, vm));
                }
            }

            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            if event != NULL {
                overlapped.hEvent = event as HANDLE;
            }
            let inner = OverlappedInner {
                overlapped,
                handle: NULL as HANDLE,
                error: 0,
                data: OverlappedData::None,
            };
            Ok(Overlapped {
                inner: PyMutex::new(inner),
            })
        }
    }

    impl Destructor for Overlapped {
        fn del(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<()> {
            use windows_sys::Win32::Foundation::{
                ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_SUCCESS,
            };
            use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

            let mut inner = zelf.inner.lock();
            let olderr = unsafe { GetLastError() };

            // Cancel pending I/O and wait for completion
            if !HasOverlappedIoCompleted(&inner.overlapped)
                && !matches!(inner.data, OverlappedData::NotStarted)
            {
                let cancelled = unsafe { CancelIoEx(inner.handle, &inner.overlapped) } != 0;
                let mut transferred: u32 = 0;
                let ret = unsafe {
                    GetOverlappedResult(
                        inner.handle,
                        &inner.overlapped,
                        &mut transferred,
                        if cancelled { 1 } else { 0 },
                    )
                };

                let err = if ret != 0 {
                    ERROR_SUCCESS
                } else {
                    unsafe { GetLastError() }
                };
                match err {
                    ERROR_SUCCESS | ERROR_NOT_FOUND | ERROR_OPERATION_ABORTED => {}
                    _ => {
                        let msg = format!(
                            "{:?} still has pending operation at deallocation, the process may crash",
                            zelf
                        );
                        let exc = vm.new_runtime_error(msg);
                        let err_msg = Some(format!(
                            "Exception ignored while deallocating overlapped operation {:?}",
                            zelf
                        ));
                        let obj: PyObjectRef = zelf.to_owned().into();
                        vm.run_unraisable(exc, err_msg, obj);
                    }
                }
            }

            // Close the event handle
            if !inner.overlapped.hEvent.is_null() {
                unsafe {
                    Foundation::CloseHandle(inner.overlapped.hEvent);
                }
                inner.overlapped.hEvent = std::ptr::null_mut();
            }

            // Restore last error
            unsafe { Foundation::SetLastError(olderr) };

            Ok(())
        }
    }

    #[pyfunction]
    fn ConnectPipe(address: String, vm: &VirtualMachine) -> PyResult<isize> {
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        };

        let address_wide: Vec<u16> = address.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                address_wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };

        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(set_from_windows_err(0, vm));
        }

        Ok(handle as isize)
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
                handle as HANDLE,
                port as HANDLE,
                key,
                concurrency,
            ) as isize
        };
        if r == 0 {
            return Err(set_from_windows_err(0, vm));
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
                port as HANDLE,
                &mut bytes_transferred,
                &mut completion_key,
                &mut overlapped,
                msecs,
            )
        };
        let err = if ret != 0 {
            Foundation::ERROR_SUCCESS
        } else {
            unsafe { GetLastError() }
        };
        if overlapped.is_null() {
            if err == Foundation::WAIT_TIMEOUT {
                return Ok(vm.ctx.none());
            } else {
                return Err(set_from_windows_err(err, vm));
            }
        }

        let value = vm.ctx.new_tuple(vec![
            err.to_pyobject(vm),
            bytes_transferred.to_pyobject(vm),
            completion_key.to_pyobject(vm),
            (overlapped as usize).to_pyobject(vm),
        ]);
        Ok(value.into())
    }

    #[pyfunction]
    fn PostQueuedCompletionStatus(
        port: isize,
        bytes: u32,
        key: usize,
        address: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let ret = unsafe {
            windows_sys::Win32::System::IO::PostQueuedCompletionStatus(
                port as HANDLE,
                bytes,
                key,
                address as *mut OVERLAPPED,
            )
        };
        if ret == 0 {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(())
    }

    // Registry to track callback data for proper cleanup
    // Uses Arc for reference counting to prevent use-after-free when callback
    // and UnregisterWait race - the data stays alive until both are done
    static WAIT_CALLBACK_REGISTRY: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<isize, std::sync::Arc<PostCallbackData>>>,
    > = std::sync::OnceLock::new();

    fn wait_callback_registry()
    -> &'static std::sync::Mutex<std::collections::HashMap<isize, std::sync::Arc<PostCallbackData>>>
    {
        WAIT_CALLBACK_REGISTRY
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
    }

    // Callback data for RegisterWaitWithQueue
    // Uses Arc to ensure the data stays alive while callback is executing
    struct PostCallbackData {
        completion_port: HANDLE,
        overlapped: *mut OVERLAPPED,
    }

    // SAFETY: The pointers are handles/addresses passed from Python and are
    // only used to call Windows APIs. They are not dereferenced as Rust pointers.
    unsafe impl Send for PostCallbackData {}
    unsafe impl Sync for PostCallbackData {}

    unsafe extern "system" fn post_to_queue_callback(
        parameter: *mut core::ffi::c_void,
        timer_or_wait_fired: bool,
    ) {
        // Reconstruct Arc from raw pointer - this gives us ownership of one reference
        // The Arc prevents use-after-free since we own a reference count
        let data = unsafe { std::sync::Arc::from_raw(parameter as *const PostCallbackData) };

        unsafe {
            let _ = windows_sys::Win32::System::IO::PostQueuedCompletionStatus(
                data.completion_port,
                if timer_or_wait_fired { 1 } else { 0 },
                0,
                data.overlapped,
            );
        }
        // Arc is dropped here, decrementing refcount
        // Memory is freed only when all references (callback + registry) are gone
    }

    #[pyfunction]
    fn RegisterWaitWithQueue(
        object: isize,
        completion_port: isize,
        overlapped: usize,
        timeout: u32,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        use windows_sys::Win32::System::Threading::{
            RegisterWaitForSingleObject, WT_EXECUTEINWAITTHREAD, WT_EXECUTEONLYONCE,
        };

        let data = std::sync::Arc::new(PostCallbackData {
            completion_port: completion_port as HANDLE,
            overlapped: overlapped as *mut OVERLAPPED,
        });

        // Create raw pointer for the callback - this increments refcount
        let data_ptr = std::sync::Arc::into_raw(data.clone());

        let mut new_wait_object: HANDLE = std::ptr::null_mut();
        let ret = unsafe {
            RegisterWaitForSingleObject(
                &mut new_wait_object,
                object as HANDLE,
                Some(post_to_queue_callback),
                data_ptr as *mut _,
                timeout,
                WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE,
            )
        };

        if ret == 0 {
            // Registration failed - reconstruct Arc to drop the extra reference
            unsafe {
                let _ = std::sync::Arc::from_raw(data_ptr);
            }
            return Err(set_from_windows_err(0, vm));
        }

        // Store in registry for cleanup tracking
        let wait_handle = new_wait_object as isize;
        if let Ok(mut registry) = wait_callback_registry().lock() {
            registry.insert(wait_handle, data);
        }

        Ok(wait_handle)
    }

    // Helper to cleanup callback data when unregistering
    // Just removes from registry - Arc ensures memory stays alive if callback is running
    fn cleanup_wait_callback_data(wait_handle: isize) {
        if let Ok(mut registry) = wait_callback_registry().lock() {
            // Removing from registry drops one Arc reference
            // If callback already ran, this frees the memory
            // If callback is still pending/running, it holds the other reference
            registry.remove(&wait_handle);
        }
    }

    #[pyfunction]
    fn UnregisterWait(wait_handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::System::Threading::UnregisterWait;

        let ret = unsafe { UnregisterWait(wait_handle as HANDLE) };
        // Cleanup callback data regardless of UnregisterWait result
        // (callback may have already fired, or may never fire)
        cleanup_wait_callback_data(wait_handle);
        if ret == 0 {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn UnregisterWaitEx(wait_handle: isize, event: isize, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::System::Threading::UnregisterWaitEx;

        let ret = unsafe { UnregisterWaitEx(wait_handle as HANDLE, event as HANDLE) };
        // Cleanup callback data regardless of UnregisterWaitEx result
        cleanup_wait_callback_data(wait_handle);
        if ret == 0 {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn BindLocal(socket: isize, family: i32, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Networking::WinSock::{
            INADDR_ANY, SOCKET_ERROR, WSAGetLastError, bind,
        };

        let ret = if family == AF_INET as i32 {
            let mut addr: SOCKADDR_IN = unsafe { std::mem::zeroed() };
            addr.sin_family = AF_INET;
            addr.sin_port = 0;
            addr.sin_addr.S_un.S_addr = INADDR_ANY;
            unsafe {
                bind(
                    socket as _,
                    &addr as *const _ as *const SOCKADDR,
                    std::mem::size_of::<SOCKADDR_IN>() as i32,
                )
            }
        } else if family == AF_INET6 as i32 {
            // in6addr_any is all zeros, which we have from zeroed()
            let mut addr: SOCKADDR_IN6 = unsafe { std::mem::zeroed() };
            addr.sin6_family = AF_INET6;
            addr.sin6_port = 0;
            unsafe {
                bind(
                    socket as _,
                    &addr as *const _ as *const SOCKADDR,
                    std::mem::size_of::<SOCKADDR_IN6>() as i32,
                )
            }
        } else {
            return Err(vm.new_value_error("expected tuple of length 2 or 4".to_owned()));
        };

        if ret == SOCKET_ERROR {
            let err = unsafe { WSAGetLastError() } as u32;
            return Err(set_from_windows_err(err, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn FormatMessage(error_code: u32, _vm: &VirtualMachine) -> PyResult<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::System::Diagnostics::Debug::{
            FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
            FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
        };

        // LANG_NEUTRAL = 0, SUBLANG_DEFAULT = 1
        const LANG_NEUTRAL: u32 = 0;
        const SUBLANG_DEFAULT: u32 = 1;

        let mut buffer: *mut u16 = std::ptr::null_mut();

        let len = unsafe {
            FormatMessageW(
                FORMAT_MESSAGE_ALLOCATE_BUFFER
                    | FORMAT_MESSAGE_FROM_SYSTEM
                    | FORMAT_MESSAGE_IGNORE_INSERTS,
                std::ptr::null(),
                error_code,
                (SUBLANG_DEFAULT << 10) | LANG_NEUTRAL,
                &mut buffer as *mut _ as *mut u16,
                0,
                std::ptr::null(),
            )
        };

        if len == 0 || buffer.is_null() {
            if !buffer.is_null() {
                unsafe { LocalFree(buffer as *mut _) };
            }
            return Ok(format!("unknown error code {}", error_code));
        }

        // Convert to Rust string, trimming trailing whitespace
        let slice = unsafe { std::slice::from_raw_parts(buffer, len as usize) };
        let msg = String::from_utf16_lossy(slice).trim_end().to_string();

        unsafe { LocalFree(buffer as *mut _) };

        Ok(msg)
    }

    #[pyfunction]
    fn WSAConnect(socket: isize, address: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
        use windows_sys::Win32::Networking::WinSock::{SOCKET_ERROR, WSAConnect, WSAGetLastError};

        let (addr_bytes, addr_len) = parse_address(&address, vm)?;

        let ret = unsafe {
            WSAConnect(
                socket as _,
                addr_bytes.as_ptr() as *const SOCKADDR,
                addr_len,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };

        if ret == SOCKET_ERROR {
            let err = unsafe { WSAGetLastError() } as u32;
            return Err(set_from_windows_err(err, vm));
        }
        Ok(())
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
            return Err(vm.new_value_error("EventAttributes must be None".to_owned()));
        }

        let name_wide: Option<Vec<u16>> =
            name.map(|n| n.encode_utf16().chain(std::iter::once(0)).collect());
        let name_ptr = name_wide
            .as_ref()
            .map(|n| n.as_ptr())
            .unwrap_or(std::ptr::null());

        let event = unsafe {
            windows_sys::Win32::System::Threading::CreateEventW(
                std::ptr::null(),
                if manual_reset { 1 } else { 0 },
                if initial_state { 1 } else { 0 },
                name_ptr,
            ) as isize
        };
        if event == NULL {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(event)
    }

    #[pyfunction]
    fn SetEvent(handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { windows_sys::Win32::System::Threading::SetEvent(handle as HANDLE) };
        if ret == 0 {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(())
    }

    #[pyfunction]
    fn ResetEvent(handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        let ret = unsafe { windows_sys::Win32::System::Threading::ResetEvent(handle as HANDLE) };
        if ret == 0 {
            return Err(set_from_windows_err(0, vm));
        }
        Ok(())
    }
}
