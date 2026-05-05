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
    use rustpython_host_env::{
        overlapped as host_overlapped, winapi as host_winapi, windows as host_windows,
    };

    pub(crate) fn module_exec(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
        let _ = vm.import("_socket", 0)?;
        host_overlapped::initialize_winsock_extensions()
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))?;
        __module_exec(vm, module);
        Ok(())
    }

    #[pyattr]
    const ERROR_IO_PENDING: u32 = host_winapi::ERROR_IO_PENDING;
    #[pyattr]
    const ERROR_NETNAME_DELETED: u32 = host_winapi::ERROR_NETNAME_DELETED;
    #[pyattr]
    const ERROR_OPERATION_ABORTED: u32 = host_winapi::ERROR_OPERATION_ABORTED;
    #[pyattr]
    const ERROR_PIPE_BUSY: u32 = host_winapi::ERROR_PIPE_BUSY;
    #[pyattr]
    const ERROR_PORT_UNREACHABLE: u32 = host_winapi::ERROR_PORT_UNREACHABLE;
    #[pyattr]
    const ERROR_SEM_TIMEOUT: u32 = host_winapi::ERROR_SEM_TIMEOUT;
    #[pyattr]
    const SO_UPDATE_ACCEPT_CONTEXT: i32 = host_overlapped::SO_UPDATE_ACCEPT_CONTEXT_VALUE;
    #[pyattr]
    const SO_UPDATE_CONNECT_CONTEXT: i32 = host_overlapped::SO_UPDATE_CONNECT_CONTEXT_VALUE;
    #[pyattr]
    const TF_REUSE_SOCKET: u32 = host_overlapped::TF_REUSE_SOCKET_FLAG;
    #[pyattr]
    const INFINITE: u32 = host_winapi::INFINITE_TIMEOUT;

    #[pyattr]
    const INVALID_HANDLE_VALUE: isize = host_overlapped::INVALID_HANDLE_VALUE_ISIZE;

    #[pyattr]
    const NULL: isize = 0;

    #[pyattr]
    #[pyclass(name, traverse)]
    #[derive(PyPayload)]
    struct Overlapped {
        inner: PyMutex<OverlappedInner>,
    }

    struct OverlappedInner {
        overlapped: host_overlapped::OverlappedIo,
        handle: host_overlapped::Handle,
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
        address: host_overlapped::SocketAddrV6,
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
        address: host_overlapped::SocketAddrV6,
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

    fn set_from_windows_err(err: u32, vm: &VirtualMachine) -> PyBaseExceptionRef {
        let err = if err == 0 {
            host_winapi::get_last_error()
        } else {
            err
        };
        let errno = rustpython_host_env::os::winerror_to_errno(err as i32);
        let message = std::io::Error::from_raw_os_error(err as i32).to_string();
        let exc = vm.new_errno_error(errno, message);
        let _ = exc
            .as_object()
            .set_attr("winerror", err.to_pyobject(vm), vm);
        exc.upcast()
    }

    /// Parse a Python address tuple to SOCKADDR
    fn parse_address(addr_obj: &PyTupleRef, vm: &VirtualMachine) -> PyResult<(Vec<u8>, i32)> {
        match addr_obj.len() {
            2 => {
                // IPv4: (host, port)
                let host: PyStrRef = addr_obj[0].clone().try_into_value(vm)?;
                let port: u16 = addr_obj[1].clone().try_to_value(vm)?;
                let host_wide: Vec<u16> = host.as_wtf8().encode_wide().chain([0]).collect();
                host_overlapped::parse_address_v4_wide(&host_wide, port)
                    .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
            }
            4 => {
                // IPv6: (host, port, flowinfo, scope_id)
                let host: PyStrRef = addr_obj[0].clone().try_into_value(vm)?;
                let port: u16 = addr_obj[1].clone().try_to_value(vm)?;
                let flowinfo: u32 = addr_obj[2].clone().try_to_value(vm)?;
                let scope_id: u32 = addr_obj[3].clone().try_to_value(vm)?;
                let host_wide: Vec<u16> = host.as_wtf8().encode_wide().chain([0]).collect();
                host_overlapped::parse_address_v6_wide(&host_wide, port, flowinfo, scope_id)
                    .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
            }
            _ => Err(vm.new_value_error("illegal address_as_bytes argument")),
        }
    }

    /// Parse a SOCKADDR_IN6 (which can also hold IPv4 addresses) to a Python address tuple
    fn unparse_address(
        addr: &host_overlapped::SocketAddrV6,
        addr_len: i32,
        vm: &VirtualMachine,
    ) -> PyResult {
        match host_overlapped::unparse_address(addr, addr_len)
            .map_err(|_| vm.new_value_error("recvfrom returned unsupported address family"))?
        {
            host_overlapped::SocketAddress::V4 { host, port } => Ok((host, port).to_pyobject(vm)),
            host_overlapped::SocketAddress::V6 {
                host,
                port,
                flowinfo,
                scope_id,
            } => Ok((host, port, flowinfo, scope_id).to_pyobject(vm)),
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
            !host_overlapped::has_overlapped_io_completed(&inner.overlapped)
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
            if !host_overlapped::has_overlapped_io_completed(&inner.overlapped) {
                host_overlapped::cancel_overlapped(inner.handle, &inner.overlapped).map_err(
                    |err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm),
                )?;
            }
            Ok(())
        }

        #[pymethod]
        fn getresult(zelf: &Py<Self>, wait: OptionalArg<bool>, vm: &VirtualMachine) -> PyResult {
            use host_winapi::{ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            let wait = wait.unwrap_or(false);

            // Check operation state
            if matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation not yet attempted"));
            }
            if matches!(inner.data, OverlappedData::NotStarted) {
                return Err(vm.new_value_error("operation failed to start"));
            }

            let result =
                host_overlapped::get_overlapped_result(inner.handle, &inner.overlapped, wait);
            let transferred = result.transferred;
            let err = result.error;
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; core::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as host_overlapped::Handle;
            inner.data = OverlappedData::Read(buf.clone());

            let err = host_overlapped::start_read_file(
                handle as host_overlapped::Handle,
                buf.as_bytes().as_ptr() as *mut u8,
                size,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = handle as host_overlapped::Handle;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            // For async read, buffer must be contiguous - we can't use a temporary copy
            // because Windows writes data directly to the buffer after this call returns
            let Some(mut contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            inner.data = OverlappedData::ReadInto(buf.clone());

            let err = host_overlapped::start_read_file(
                handle as host_overlapped::Handle,
                contiguous.as_mut_ptr(),
                buf_len as u32,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let mut flags = flags.unwrap_or(0);

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; core::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as host_overlapped::Handle;
            inner.data = OverlappedData::Read(buf.clone());

            let err = host_overlapped::start_wsa_recv(
                handle as usize,
                buf.as_bytes().as_ptr() as *mut u8,
                size,
                &mut flags,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let mut flags = flags;
            inner.handle = handle as host_overlapped::Handle;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            let Some(mut contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            inner.data = OverlappedData::ReadInto(buf.clone());

            let err = host_overlapped::start_wsa_recv(
                handle as usize,
                contiguous.as_mut_ptr(),
                buf_len as u32,
                &mut flags,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = handle as host_overlapped::Handle;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            // For async write, buffer must be contiguous - we can't use a temporary copy
            // because Windows reads from the buffer after this call returns
            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            inner.data = OverlappedData::Write(buf.clone());

            let err = host_overlapped::start_write_file(
                handle as host_overlapped::Handle,
                contiguous.as_ptr(),
                buf_len as u32,
                &mut inner.overlapped,
            );
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = handle as host_overlapped::Handle;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            inner.data = OverlappedData::Write(buf.clone());

            let err = host_overlapped::start_wsa_send(
                handle as usize,
                contiguous.as_ptr(),
                buf_len as u32,
                flags,
                &mut inner.overlapped,
            );
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            // Buffer size: local address + remote address
            let size = core::mem::size_of::<host_overlapped::SocketAddrV6>() + 16;
            let buf = vec![0u8; size * 2];
            let buf = vm.ctx.new_bytes(buf);

            inner.handle = listen_socket as host_overlapped::Handle;
            inner.data = OverlappedData::Accept(buf.clone());

            let err = host_overlapped::start_accept_ex(
                listen_socket as usize,
                accept_socket as usize,
                buf.as_bytes().as_ptr() as *mut u8,
                size as u32,
                &mut inner.overlapped,
            );
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let (addr_bytes, addr_len) = parse_address(&address, vm)?;

            inner.handle = socket as host_overlapped::Handle;
            // Store addr_bytes in OverlappedData to keep it alive during async operation
            inner.data = OverlappedData::Connect(addr_bytes);

            // Get pointer to the stored address data
            let addr_ptr = match &inner.data {
                OverlappedData::Connect(bytes) => bytes.as_ptr(),
                _ => unreachable!(),
            };

            let err = host_overlapped::start_connect_ex(
                socket as usize,
                addr_ptr as *const host_overlapped::SocketAddrRaw,
                addr_len,
                &mut inner.overlapped,
            );
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = socket as host_overlapped::Handle;
            inner.data = OverlappedData::Disconnect;

            let err =
                host_overlapped::start_disconnect_ex(socket as usize, flags, &mut inner.overlapped);
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
        #[allow(
            clippy::too_many_arguments,
            reason = "mirrors Windows TransmitFile argument structure"
        )]
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = socket as host_overlapped::Handle;
            inner.data = OverlappedData::TransmitFile;
            let err = host_overlapped::start_transmit_file(
                socket as usize,
                file as host_overlapped::Handle,
                count_to_write,
                count_per_send,
                flags,
                offset,
                offset_high,
                &mut inner.overlapped,
            );
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            inner.handle = pipe as host_overlapped::Handle;
            inner.data = OverlappedData::ConnectNamedPipe;

            let err = host_overlapped::start_connect_named_pipe(
                pipe as host_overlapped::Handle,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_PIPE_CONNECTED => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{ERROR_IO_PENDING, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let (addr_bytes, addr_len) = parse_address(&address, vm)?;

            inner.handle = handle as host_overlapped::Handle;
            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            let Some(contiguous) = buf.as_contiguous() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            // Store both buffer and address in OverlappedData to keep them alive
            inner.data = OverlappedData::WriteTo(OverlappedWriteTo {
                buf: buf.clone(),
                address: addr_bytes,
            });

            // Get pointer to the stored address data
            let addr_ptr = match &inner.data {
                OverlappedData::WriteTo(wt) => wt.address.as_ptr(),
                _ => unreachable!(),
            };

            let err = host_overlapped::start_wsa_send_to(
                handle as usize,
                contiguous.as_ptr(),
                buf_len as u32,
                flags,
                addr_ptr as *const host_overlapped::SocketAddrRaw,
                addr_len,
                &mut inner.overlapped,
            );
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let mut flags = flags.unwrap_or(0);

            #[cfg(target_pointer_width = "32")]
            let size = core::cmp::min(size, isize::MAX as u32);

            let buf = vec![0u8; core::cmp::max(size, 1) as usize];
            let buf = vm.ctx.new_bytes(buf);
            inner.handle = handle as host_overlapped::Handle;

            let address: host_overlapped::SocketAddrV6 = unsafe { core::mem::zeroed() };
            let address_length = core::mem::size_of::<host_overlapped::SocketAddrV6>() as i32;

            inner.data = OverlappedData::ReadFrom(OverlappedReadFrom {
                result: None,
                allocated_buffer: buf.clone(),
                address,
                address_length,
            });

            // Get mutable reference to address in inner.data
            let (addr_ptr, addr_len_ptr) = match &mut inner.data {
                OverlappedData::ReadFrom(rf) => (
                    &mut rf.address as *mut host_overlapped::SocketAddrV6,
                    &mut rf.address_length as *mut i32,
                ),
                _ => unreachable!(),
            };

            let err = host_overlapped::start_wsa_recv_from(
                handle as usize,
                buf.as_bytes().as_ptr() as *mut u8,
                size,
                &mut flags,
                addr_ptr as *mut host_overlapped::SocketAddrRaw,
                addr_len_ptr,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
            use host_winapi::{
                ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS,
            };

            let mut inner = zelf.inner.lock();
            if !matches!(inner.data, OverlappedData::None) {
                return Err(vm.new_value_error("operation already attempted"));
            }

            let mut flags = flags.unwrap_or(0);
            inner.handle = handle as host_overlapped::Handle;

            let Some(mut contiguous) = buf.as_contiguous_mut() else {
                return Err(vm.new_buffer_error("buffer is not contiguous"));
            };

            let buf_len = buf.desc.len;
            if buf_len > u32::MAX as usize {
                return Err(vm.new_value_error("buffer too large"));
            }

            let address: host_overlapped::SocketAddrV6 = unsafe { core::mem::zeroed() };
            let address_length = core::mem::size_of::<host_overlapped::SocketAddrV6>() as i32;

            inner.data = OverlappedData::ReadFromInto(OverlappedReadFromInto {
                result: None,
                user_buffer: buf.clone(),
                address,
                address_length,
            });

            // Get mutable reference to address in inner.data
            let (addr_ptr, addr_len_ptr) = match &mut inner.data {
                OverlappedData::ReadFromInto(rfi) => (
                    &mut rfi.address as *mut host_overlapped::SocketAddrV6,
                    &mut rfi.address_length as *mut i32,
                ),
                _ => unreachable!(),
            };

            let err = host_overlapped::start_wsa_recv_from(
                handle as usize,
                contiguous.as_mut_ptr(),
                size,
                &mut flags,
                addr_ptr as *mut host_overlapped::SocketAddrRaw,
                addr_len_ptr,
                &mut inner.overlapped,
            );
            inner.error = err;

            match err {
                ERROR_BROKEN_PIPE => {
                    host_overlapped::mark_as_completed(&mut inner.overlapped);
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
                event = host_winapi::create_event_w(true, false, core::ptr::null())
                    .map(|handle| handle as isize)
                    .map_err(|err| {
                        set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm)
                    })?;
            }

            let mut overlapped: host_overlapped::OverlappedIo = unsafe { core::mem::zeroed() };
            if event != NULL {
                overlapped.hEvent = event as host_overlapped::Handle;
            }
            let inner = OverlappedInner {
                overlapped,
                handle: NULL as host_overlapped::Handle,
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
            use host_winapi::{ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_SUCCESS};

            let mut inner = zelf.inner.lock();
            let olderr = host_winapi::get_last_error();

            // Cancel pending I/O and wait for completion
            if !host_overlapped::has_overlapped_io_completed(&inner.overlapped)
                && !matches!(inner.data, OverlappedData::NotStarted)
            {
                match host_overlapped::cancel_overlapped_for_drop(inner.handle, &inner.overlapped)
                    .error
                {
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
                let _ = host_winapi::close_handle(inner.overlapped.hEvent);
                inner.overlapped.hEvent = core::ptr::null_mut();
            }

            // Restore last error
            host_windows::set_last_error(olderr);

            Ok(())
        }
    }

    #[pyfunction]
    fn ConnectPipe(address: String, vm: &VirtualMachine) -> PyResult<isize> {
        host_overlapped::connect_pipe(&address)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn CreateIoCompletionPort(
        handle: isize,
        port: isize,
        key: usize,
        concurrency: u32,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        host_overlapped::create_io_completion_port(handle, port, key, concurrency)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn GetQueuedCompletionStatus(port: isize, msecs: u32, vm: &VirtualMachine) -> PyResult {
        match host_overlapped::get_queued_completion_status(port, msecs)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))?
        {
            host_overlapped::WaitResult::Timeout => Ok(vm.ctx.none()),
            host_overlapped::WaitResult::Queued(status) => Ok(vm
                .ctx
                .new_tuple(vec![
                    status.error.to_pyobject(vm),
                    status.bytes_transferred.to_pyobject(vm),
                    status.completion_key.to_pyobject(vm),
                    status.overlapped.to_pyobject(vm),
                ])
                .into()),
        }
    }

    #[pyfunction]
    fn PostQueuedCompletionStatus(
        port: isize,
        bytes: u32,
        key: usize,
        address: usize,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        host_overlapped::post_queued_completion_status(port, bytes, key, address)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn RegisterWaitWithQueue(
        object: isize,
        completion_port: isize,
        overlapped: usize,
        timeout: u32,
        vm: &VirtualMachine,
    ) -> PyResult<isize> {
        host_overlapped::register_wait_with_queue(object, completion_port, overlapped, timeout)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn UnregisterWait(wait_handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_overlapped::unregister_wait(wait_handle)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn UnregisterWaitEx(wait_handle: isize, event: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_overlapped::unregister_wait_ex(wait_handle, event)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn BindLocal(socket: isize, family: i32, vm: &VirtualMachine) -> PyResult<()> {
        if family != host_overlapped::AF_INET_FAMILY && family != host_overlapped::AF_INET6_FAMILY {
            return Err(vm.new_value_error("expected tuple of length 2 or 4"));
        }
        host_overlapped::bind_local(socket, family)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn FormatMessage(error_code: u32, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(host_overlapped::format_message(error_code))
    }

    #[pyfunction]
    fn WSAConnect(socket: isize, address: PyTupleRef, vm: &VirtualMachine) -> PyResult<()> {
        let (addr_bytes, addr_len) = parse_address(&address, vm)?;
        host_overlapped::wsa_connect(
            socket,
            addr_bytes.as_ptr() as *const host_overlapped::SocketAddrRaw,
            addr_len,
        )
        .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
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

        let name_wide: Option<Vec<u16>> =
            name.map(|n| n.encode_utf16().chain(core::iter::once(0)).collect());
        host_winapi::create_event_w(
            manual_reset,
            initial_state,
            name_wide.as_ref().map_or(core::ptr::null(), |n| n.as_ptr()),
        )
        .map(|h| h as isize)
        .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn SetEvent(handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::set_event(handle as host_winapi::Handle)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }

    #[pyfunction]
    fn ResetEvent(handle: isize, vm: &VirtualMachine) -> PyResult<()> {
        host_winapi::reset_event(handle as host_winapi::Handle)
            .map_err(|err| set_from_windows_err(err.raw_os_error().unwrap_or(0) as u32, vm))
    }
}
