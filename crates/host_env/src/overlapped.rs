#![allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "This module exposes raw overlapped I/O wrappers over Win32 and Winsock APIs."
)]
#![allow(
    clippy::too_many_arguments,
    reason = "These helpers preserve the underlying Win32 and Winsock call shapes."
)]

use alloc::sync::Arc;
use std::{
    collections::HashMap,
    io,
    sync::{Mutex, OnceLock},
};
use windows_sys::Win32::{
    Foundation::{ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_SUCCESS, HANDLE},
    Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6},
    System::{
        Diagnostics::Debug::{
            FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_FROM_SYSTEM,
            FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
        },
        IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        Pipes::ConnectNamedPipe,
        Threading::{CreateEventW, SetEvent},
    },
};

pub struct TransferResult {
    pub transferred: u32,
    pub error: u32,
}

pub struct OverlappedResult {
    pub transferred: u32,
    pub error: u32,
}

pub struct Operation {
    overlapped: Box<OVERLAPPED>,
    handle: HANDLE,
    pending: bool,
    completed: bool,
    read_buffer: Option<Vec<u8>>,
    write_buffer: Option<Vec<u8>>,
}

impl core::fmt::Debug for Operation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Operation")
            .field("handle", &self.handle)
            .field("pending", &self.pending)
            .field("completed", &self.completed)
            .finish()
    }
}

unsafe impl Sync for Operation {}
unsafe impl Send for Operation {}

impl Operation {
    pub fn new(handle: HANDLE) -> io::Result<Self> {
        let event = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut overlapped: OVERLAPPED = unsafe { core::mem::zeroed() };
        overlapped.hEvent = event;
        Ok(Self {
            overlapped: Box::new(overlapped),
            handle,
            pending: false,
            completed: false,
            read_buffer: None,
            write_buffer: None,
        })
    }

    pub fn event(&self) -> HANDLE {
        self.overlapped.hEvent
    }

    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn read_buffer(&self) -> Option<&[u8]> {
        self.read_buffer.as_deref()
    }

    pub fn get_result(&mut self, wait: bool) -> io::Result<TransferResult> {
        use windows_sys::Win32::Foundation::{
            ERROR_IO_INCOMPLETE, ERROR_OPERATION_ABORTED, ERROR_SUCCESS, GetLastError,
        };

        let mut transferred = 0;
        let ret = unsafe {
            GetOverlappedResult(
                self.handle,
                &*self.overlapped,
                &mut transferred,
                i32::from(wait),
            )
        };

        let err = if ret == 0 {
            unsafe { GetLastError() }
        } else {
            ERROR_SUCCESS
        };

        match err {
            ERROR_SUCCESS | ERROR_MORE_DATA | ERROR_OPERATION_ABORTED => {
                self.completed = true;
                self.pending = false;
            }
            ERROR_IO_INCOMPLETE => {}
            _ => {
                self.pending = false;
                return Err(io::Error::from_raw_os_error(err as i32));
            }
        }

        if self.completed
            && let Some(read_buffer) = &mut self.read_buffer
            && transferred != read_buffer.len() as u32
        {
            read_buffer.truncate(transferred as usize);
        }

        Ok(TransferResult {
            transferred,
            error: err,
        })
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        let ret = if self.pending {
            unsafe { CancelIoEx(self.handle, &*self.overlapped) }
        } else {
            1
        };
        if ret == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if err != windows_sys::Win32::Foundation::ERROR_NOT_FOUND {
                return Err(io::Error::from_raw_os_error(err as i32));
            }
        }
        self.pending = false;
        Ok(())
    }

    pub fn connect_named_pipe(&mut self) -> io::Result<()> {
        use windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED;

        let err = start_connect_named_pipe(self.handle, &mut *self.overlapped);
        match err {
            ERROR_IO_PENDING => {
                self.pending = true;
            }
            ERROR_PIPE_CONNECTED => {
                if unsafe { SetEvent(self.overlapped.hEvent) } == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            _ => return Err(io::Error::from_raw_os_error(err as i32)),
        }
        Ok(())
    }

    pub fn write(&mut self, buffer: &[u8]) -> io::Result<u32> {
        let len = core::cmp::min(buffer.len(), u32::MAX as usize) as u32;
        self.write_buffer = Some(buffer[..len as usize].to_vec());
        let write_buf = self
            .write_buffer
            .as_ref()
            .expect("write buffer initialized");
        let err = start_write_file(self.handle, write_buf.as_ptr(), len, &mut *self.overlapped);

        if err != ERROR_SUCCESS && err != ERROR_IO_PENDING {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        if err == ERROR_IO_PENDING {
            self.pending = true;
        }

        Ok(err)
    }

    pub fn read(&mut self, size: u32) -> io::Result<u32> {
        self.read_buffer = Some(vec![0u8; size as usize]);
        let read_buf = self.read_buffer.as_mut().expect("read buffer initialized");
        let err = start_read_file(
            self.handle,
            read_buf.as_mut_ptr(),
            size,
            &mut *self.overlapped,
        );

        if err != ERROR_SUCCESS && err != ERROR_IO_PENDING && err != ERROR_MORE_DATA {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
        if err == ERROR_IO_PENDING {
            self.pending = true;
        }

        Ok(err)
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.overlapped.hEvent.is_null() {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.overlapped.hEvent) };
        }
    }
}

pub struct QueuedCompletionStatus {
    pub error: u32,
    pub bytes_transferred: u32,
    pub completion_key: usize,
    pub overlapped: usize,
}

pub struct WaitCallbackData {
    completion_port: HANDLE,
    overlapped: *mut OVERLAPPED,
}

pub enum WaitResult {
    Timeout,
    Queued(QueuedCompletionStatus),
}

pub enum SocketAddress {
    V4 {
        host: String,
        port: u16,
    },
    V6 {
        host: String,
        port: u16,
        flowinfo: u32,
        scope_id: u32,
    },
}

static ACCEPT_EX: OnceLock<usize> = OnceLock::new();
static CONNECT_EX: OnceLock<usize> = OnceLock::new();
static DISCONNECT_EX: OnceLock<usize> = OnceLock::new();
static TRANSMIT_FILE: OnceLock<usize> = OnceLock::new();
static WAIT_CALLBACK_REGISTRY: OnceLock<Mutex<HashMap<isize, Arc<WaitCallbackData>>>> =
    OnceLock::new();

fn wait_callback_registry() -> &'static Mutex<HashMap<isize, Arc<WaitCallbackData>>> {
    WAIT_CALLBACK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn initialize_winsock_extensions() -> io::Result<()> {
    use windows_sys::Win32::Networking::WinSock::{
        INVALID_SOCKET, IPPROTO_TCP, SIO_GET_EXTENSION_FUNCTION_POINTER, SOCK_STREAM, SOCKET_ERROR,
        WSAGetLastError, WSAIoctl, closesocket, socket,
    };

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

    if ACCEPT_EX.get().is_some()
        && CONNECT_EX.get().is_some()
        && DISCONNECT_EX.get().is_some()
        && TRANSMIT_FILE.get().is_some()
    {
        return Ok(());
    }

    let s = unsafe { socket(AF_INET as i32, SOCK_STREAM, IPPROTO_TCP) };
    if s == INVALID_SOCKET {
        return Err(io::Error::from_raw_os_error(
            unsafe { WSAGetLastError() } as i32
        ));
    }

    let mut dw_bytes = 0;

    macro_rules! get_extension {
        ($guid:expr, $lock:expr) => {{
            let mut func_ptr: usize = 0;
            let ret = unsafe {
                WSAIoctl(
                    s,
                    SIO_GET_EXTENSION_FUNCTION_POINTER,
                    &$guid as *const _ as *const _,
                    core::mem::size_of_val(&$guid) as u32,
                    &mut func_ptr as *mut _ as *mut _,
                    core::mem::size_of::<usize>() as u32,
                    &mut dw_bytes,
                    core::ptr::null_mut(),
                    None,
                )
            };
            if ret == SOCKET_ERROR {
                let err = unsafe { WSAGetLastError() };
                unsafe { closesocket(s) };
                return Err(io::Error::from_raw_os_error(err as i32));
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

pub fn mark_as_completed(ov: &mut OVERLAPPED) {
    ov.Internal = 0;
    if !ov.hEvent.is_null() {
        unsafe {
            let _ = SetEvent(ov.hEvent);
        }
    }
}

pub fn has_overlapped_io_completed(overlapped: &OVERLAPPED) -> bool {
    overlapped.Internal != (windows_sys::Win32::Foundation::STATUS_PENDING as usize)
}

pub fn cancel_overlapped(handle: HANDLE, overlapped: *const OVERLAPPED) -> io::Result<()> {
    let ret = unsafe { CancelIoEx(handle, overlapped) };
    if ret == 0 {
        let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        if err != windows_sys::Win32::Foundation::ERROR_NOT_FOUND {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
    Ok(())
}

pub fn get_overlapped_result(
    handle: HANDLE,
    overlapped: *const OVERLAPPED,
    wait: bool,
) -> OverlappedResult {
    let mut transferred = 0;
    let ret = unsafe { GetOverlappedResult(handle, overlapped, &mut transferred, i32::from(wait)) };
    let error = if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    };
    OverlappedResult { transferred, error }
}

pub fn cancel_overlapped_for_drop(
    handle: HANDLE,
    overlapped: *const OVERLAPPED,
) -> OverlappedResult {
    let cancelled = unsafe { CancelIoEx(handle, overlapped) } != 0;
    get_overlapped_result(handle, overlapped, cancelled)
}

pub fn start_read_file(
    handle: HANDLE,
    buffer: *mut u8,
    len: u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    let mut transferred = 0;
    let ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::ReadFile(
            handle,
            buffer.cast(),
            len,
            &mut transferred,
            overlapped,
        )
    };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    }
}

pub fn start_write_file(
    handle: HANDLE,
    buffer: *const u8,
    len: u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    let mut transferred = 0;
    let ret = unsafe {
        windows_sys::Win32::Storage::FileSystem::WriteFile(
            handle,
            buffer.cast(),
            len,
            &mut transferred,
            overlapped,
        )
    };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    }
}

pub fn start_wsa_recv(
    handle: usize,
    buffer: *mut u8,
    len: u32,
    flags: *mut u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecv};

    let wsabuf = WSABUF {
        buf: buffer.cast(),
        len,
    };
    let mut transferred = 0;
    let ret = unsafe {
        WSARecv(
            handle,
            &wsabuf,
            1,
            &mut transferred,
            flags,
            overlapped,
            None,
        )
    };
    if ret < 0 {
        unsafe { WSAGetLastError() as u32 }
    } else {
        ERROR_SUCCESS
    }
}

pub fn start_wsa_send(
    handle: usize,
    buffer: *const u8,
    len: u32,
    flags: u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSASend};

    let wsabuf = WSABUF {
        buf: buffer.cast_mut().cast(),
        len,
    };
    let mut transferred = 0;
    let ret = unsafe {
        WSASend(
            handle,
            &wsabuf,
            1,
            &mut transferred,
            flags,
            overlapped,
            None,
        )
    };
    if ret < 0 {
        unsafe { WSAGetLastError() as u32 }
    } else {
        ERROR_SUCCESS
    }
}

pub fn start_accept_ex(
    listen_socket: usize,
    accept_socket: usize,
    buffer: *mut u8,
    address_size: u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

    type AcceptExFn = unsafe extern "system" fn(
        s_listen_socket: usize,
        s_accept_socket: usize,
        lp_output_buffer: *mut core::ffi::c_void,
        dw_receive_data_length: u32,
        dw_local_address_length: u32,
        dw_remote_address_length: u32,
        lpdw_bytes_received: *mut u32,
        lp_overlapped: *mut OVERLAPPED,
    ) -> i32;

    let accept_ex: AcceptExFn = unsafe { core::mem::transmute(*ACCEPT_EX.get().unwrap()) };
    let mut bytes_received = 0;
    let ret = unsafe {
        accept_ex(
            listen_socket,
            accept_socket,
            buffer.cast(),
            0,
            address_size,
            address_size,
            &mut bytes_received,
            overlapped,
        )
    };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { WSAGetLastError() as u32 }
    }
}

pub fn start_connect_ex(
    socket: usize,
    address: *const SOCKADDR,
    address_len: i32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

    type ConnectExFn = unsafe extern "system" fn(
        s: usize,
        name: *const SOCKADDR,
        namelen: i32,
        lp_send_buffer: *const core::ffi::c_void,
        dw_send_data_length: u32,
        lpdw_bytes_sent: *mut u32,
        lp_overlapped: *mut OVERLAPPED,
    ) -> i32;

    let connect_ex: ConnectExFn = unsafe { core::mem::transmute(*CONNECT_EX.get().unwrap()) };
    let ret = unsafe {
        connect_ex(
            socket,
            address,
            address_len,
            core::ptr::null(),
            0,
            core::ptr::null_mut(),
            overlapped,
        )
    };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { WSAGetLastError() as u32 }
    }
}

pub fn start_disconnect_ex(socket: usize, flags: u32, overlapped: *mut OVERLAPPED) -> u32 {
    use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

    type DisconnectExFn = unsafe extern "system" fn(
        s: usize,
        lp_overlapped: *mut OVERLAPPED,
        dw_flags: u32,
        dw_reserved: u32,
    ) -> i32;

    let disconnect_ex: DisconnectExFn =
        unsafe { core::mem::transmute(*DISCONNECT_EX.get().unwrap()) };
    let ret = unsafe { disconnect_ex(socket, overlapped, flags, 0) };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { WSAGetLastError() as u32 }
    }
}

pub fn start_transmit_file(
    socket: usize,
    file: HANDLE,
    count_to_write: u32,
    count_per_send: u32,
    flags: u32,
    offset: u32,
    offset_high: u32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::WSAGetLastError;

    type TransmitFileFn = unsafe extern "system" fn(
        h_socket: usize,
        h_file: HANDLE,
        n_number_of_bytes_to_write: u32,
        n_number_of_bytes_per_send: u32,
        lp_overlapped: *mut OVERLAPPED,
        lp_transmit_buffers: *const core::ffi::c_void,
        dw_reserved: u32,
    ) -> i32;

    unsafe {
        (*overlapped).Anonymous.Anonymous.Offset = offset;
        (*overlapped).Anonymous.Anonymous.OffsetHigh = offset_high;
    }

    let transmit_file: TransmitFileFn =
        unsafe { core::mem::transmute(*TRANSMIT_FILE.get().unwrap()) };
    let ret = unsafe {
        transmit_file(
            socket,
            file,
            count_to_write,
            count_per_send,
            overlapped,
            core::ptr::null(),
            flags,
        )
    };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { WSAGetLastError() as u32 }
    }
}

pub fn start_connect_named_pipe(pipe: HANDLE, overlapped: *mut OVERLAPPED) -> u32 {
    let ret = unsafe { ConnectNamedPipe(pipe, overlapped) };
    if ret != 0 {
        ERROR_SUCCESS
    } else {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    }
}

pub fn start_wsa_send_to(
    handle: usize,
    buffer: *const u8,
    len: u32,
    flags: u32,
    address: *const SOCKADDR,
    address_len: i32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSASendTo};

    let wsabuf = WSABUF {
        buf: buffer.cast_mut().cast(),
        len,
    };
    let mut transferred = 0;
    let ret = unsafe {
        WSASendTo(
            handle,
            &wsabuf,
            1,
            &mut transferred,
            flags,
            address,
            address_len,
            overlapped,
            None,
        )
    };
    if ret < 0 {
        unsafe { WSAGetLastError() as u32 }
    } else {
        ERROR_SUCCESS
    }
}

pub fn start_wsa_recv_from(
    handle: usize,
    buffer: *mut u8,
    len: u32,
    flags: *mut u32,
    address: *mut SOCKADDR,
    address_len: *mut i32,
    overlapped: *mut OVERLAPPED,
) -> u32 {
    use windows_sys::Win32::Networking::WinSock::{WSABUF, WSAGetLastError, WSARecvFrom};

    let wsabuf = WSABUF {
        buf: buffer.cast(),
        len,
    };
    let mut transferred = 0;
    let ret = unsafe {
        WSARecvFrom(
            handle,
            &wsabuf,
            1,
            &mut transferred,
            flags,
            address,
            address_len,
            overlapped,
            None,
        )
    };
    if ret < 0 {
        unsafe { WSAGetLastError() as u32 }
    } else {
        ERROR_SUCCESS
    }
}

pub fn connect_pipe(address: &str) -> io::Result<isize> {
    use windows_sys::Win32::{
        Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING},
    };

    let address_wide: Vec<u16> = address.encode_utf16().chain(core::iter::once(0)).collect();
    let handle = unsafe {
        CreateFileW(
            address_wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            core::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle as isize)
    }
}

pub fn create_io_completion_port(
    handle: isize,
    port: isize,
    key: usize,
    concurrency: u32,
) -> io::Result<isize> {
    let r = unsafe {
        windows_sys::Win32::System::IO::CreateIoCompletionPort(
            handle as HANDLE,
            port as HANDLE,
            key,
            concurrency,
        ) as isize
    };
    if r == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

pub fn get_queued_completion_status(port: isize, msecs: u32) -> io::Result<WaitResult> {
    let mut bytes_transferred = 0;
    let mut completion_key = 0;
    let mut overlapped: *mut OVERLAPPED = core::ptr::null_mut();
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
        windows_sys::Win32::Foundation::ERROR_SUCCESS
    } else {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    };
    if overlapped.is_null() {
        if err == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
            Ok(WaitResult::Timeout)
        } else {
            Err(io::Error::from_raw_os_error(err as i32))
        }
    } else {
        Ok(WaitResult::Queued(QueuedCompletionStatus {
            error: err,
            bytes_transferred,
            completion_key,
            overlapped: overlapped as usize,
        }))
    }
}

pub fn post_queued_completion_status(
    port: isize,
    bytes: u32,
    key: usize,
    address: usize,
) -> io::Result<()> {
    let ret = unsafe {
        windows_sys::Win32::System::IO::PostQueuedCompletionStatus(
            port as HANDLE,
            bytes,
            key,
            address as *mut OVERLAPPED,
        )
    };
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

unsafe impl Send for WaitCallbackData {}
unsafe impl Sync for WaitCallbackData {}

unsafe extern "system" fn post_to_queue_callback(
    parameter: *mut core::ffi::c_void,
    timer_or_wait_fired: bool,
) {
    let data = unsafe { Arc::from_raw(parameter as *const WaitCallbackData) };
    unsafe {
        let _ = windows_sys::Win32::System::IO::PostQueuedCompletionStatus(
            data.completion_port,
            if timer_or_wait_fired { 1 } else { 0 },
            0,
            data.overlapped,
        );
    }
}

pub fn register_wait_with_queue(
    object: isize,
    completion_port: isize,
    overlapped: usize,
    timeout: u32,
) -> io::Result<isize> {
    use windows_sys::Win32::System::Threading::{
        RegisterWaitForSingleObject, WT_EXECUTEINWAITTHREAD, WT_EXECUTEONLYONCE,
    };

    let data = Arc::new(WaitCallbackData {
        completion_port: completion_port as HANDLE,
        overlapped: overlapped as *mut OVERLAPPED,
    });
    let data_ptr = Arc::into_raw(data.clone());

    let mut new_wait_object: HANDLE = core::ptr::null_mut();
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
        unsafe {
            let _ = Arc::from_raw(data_ptr);
        }
        return Err(io::Error::last_os_error());
    }

    let wait_handle = new_wait_object as isize;
    if let Ok(mut registry) = wait_callback_registry().lock() {
        registry.insert(wait_handle, data);
    }
    Ok(wait_handle)
}

fn cleanup_wait_callback_data(wait_handle: isize) {
    if let Ok(mut registry) = wait_callback_registry().lock() {
        registry.remove(&wait_handle);
    }
}

pub fn unregister_wait(wait_handle: isize) -> io::Result<()> {
    let ret =
        unsafe { windows_sys::Win32::System::Threading::UnregisterWait(wait_handle as HANDLE) };
    cleanup_wait_callback_data(wait_handle);
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn unregister_wait_ex(wait_handle: isize, event: isize) -> io::Result<()> {
    let ret = unsafe {
        windows_sys::Win32::System::Threading::UnregisterWaitEx(
            wait_handle as HANDLE,
            event as HANDLE,
        )
    };
    cleanup_wait_callback_data(wait_handle);
    if ret == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn bind_local(socket: isize, family: i32) -> io::Result<()> {
    use windows_sys::Win32::Networking::WinSock::{
        INADDR_ANY, SOCKET_ERROR, WSAGetLastError, bind,
    };

    let ret = if family == AF_INET as i32 {
        let mut addr: SOCKADDR_IN = unsafe { core::mem::zeroed() };
        addr.sin_family = AF_INET;
        addr.sin_port = 0;
        addr.sin_addr.S_un.S_addr = INADDR_ANY;
        unsafe {
            bind(
                socket as _,
                &addr as *const _ as *const SOCKADDR,
                core::mem::size_of::<SOCKADDR_IN>() as i32,
            )
        }
    } else if family == AF_INET6 as i32 {
        let mut addr: SOCKADDR_IN6 = unsafe { core::mem::zeroed() };
        addr.sin6_family = AF_INET6;
        addr.sin6_port = 0;
        unsafe {
            bind(
                socket as _,
                &addr as *const _ as *const SOCKADDR,
                core::mem::size_of::<SOCKADDR_IN6>() as i32,
            )
        }
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected tuple of length 2 or 4",
        ));
    };

    if ret == SOCKET_ERROR {
        Err(io::Error::from_raw_os_error(
            unsafe { WSAGetLastError() } as i32
        ))
    } else {
        Ok(())
    }
}

pub fn format_message(error_code: u32) -> String {
    use windows_sys::Win32::Foundation::LocalFree;

    const LANG_NEUTRAL: u32 = 0;
    const SUBLANG_DEFAULT: u32 = 1;

    let mut buffer: *mut u16 = core::ptr::null_mut();
    let len = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            core::ptr::null(),
            error_code,
            (SUBLANG_DEFAULT << 10) | LANG_NEUTRAL,
            &mut buffer as *mut _ as *mut u16,
            0,
            core::ptr::null(),
        )
    };

    if len == 0 || buffer.is_null() {
        if !buffer.is_null() {
            unsafe { LocalFree(buffer as *mut _) };
        }
        return format!("unknown error code {}", error_code);
    }

    let slice = unsafe { core::slice::from_raw_parts(buffer, len as usize) };
    let msg = String::from_utf16_lossy(slice).trim_end().to_string();
    unsafe { LocalFree(buffer as *mut _) };
    msg
}

pub fn wsa_connect(socket: isize, addr_ptr: *const SOCKADDR, addr_len: i32) -> io::Result<()> {
    use windows_sys::Win32::Networking::WinSock::{SOCKET_ERROR, WSAConnect, WSAGetLastError};

    let ret = unsafe {
        WSAConnect(
            socket as _,
            addr_ptr,
            addr_len,
            core::ptr::null(),
            core::ptr::null_mut(),
            core::ptr::null(),
            core::ptr::null(),
        )
    };
    if ret == SOCKET_ERROR {
        Err(io::Error::from_raw_os_error(
            unsafe { WSAGetLastError() } as i32
        ))
    } else {
        Ok(())
    }
}
