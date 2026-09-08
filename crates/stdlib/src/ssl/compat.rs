// spell-checker: ignore webpki ssleof sslerror akid certsign sslerr aesgcm

// OpenSSL compatibility layer for rustls
//
// This module provides OpenSSL-like abstractions over rustls APIs,
// making the code more readable and maintainable. Each function is named
// after its OpenSSL equivalent (e.g., ssl_do_handshake corresponds to SSL_do_handshake).

// SSL error code data tables (shared with OpenSSL backend for compatibility)
// These map OpenSSL error codes to human-readable strings
#[allow(
    clippy::duplicate_mod,
    reason = "This is duplicated only when running clippy. The two features are mutually exclusive"
)]
#[path = "../openssl/ssl_data_31.rs"]
mod ssl_data;

use crate::socket::{SockWaitKind, timeout_error_msg};
use crate::vm::VirtualMachine;
use rustls::Connection;
use rustpython_vm::builtins::{PyBaseException, PyBaseExceptionRef};
use rustpython_vm::convert::IntoPyException;
use rustpython_vm::function::ArgBytesLike;
use rustpython_vm::{AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject};
use std::io::Read;

// Import PySSLSocket from parent module
use super::_ssl::{PySSLSocket, SSL3_RT_MAX_PACKET_SIZE};

// Import error types and helper functions from error module
use super::error::{
    PySSLCertVerificationError, PySSLError, PySSLWantWriteError, create_ssl_eof_error,
    create_ssl_syscall_error, create_ssl_want_read_error, create_ssl_want_write_error,
    create_ssl_zero_return_error,
};

// OpenSSL Constants:

// OpenSSL error library codes (include/openssl/err.h)
// #define ERR_LIB_SSL 20
const ERR_LIB_SSL: i32 = 20;

// OpenSSL SSL error reason codes (include/openssl/sslerr.h)
// #define SSL_R_NO_SHARED_CIPHER 193
const SSL_R_NO_SHARED_CIPHER: i32 = 193;

use rustpython_host_env::ssl::x509::rustls_cert_error_to_verify_info;

/// Create SSLCertVerificationError with proper attributes
///
/// Matches CPython's _ssl.c fill_and_set_sslerror() behavior.
/// This function creates a Python SSLCertVerificationError exception with verify_code
/// and verify_message attributes set appropriately for the given rustls certificate error.
///
/// # Note
/// If attribute setting fails (extremely rare), returns the exception without attributes
pub(super) fn create_ssl_cert_verification_error(
    vm: &VirtualMachine,
    cert_err: &rustls::CertificateError,
) -> PyResult<PyBaseExceptionRef> {
    let (verify_code, verify_message) = rustls_cert_error_to_verify_info(cert_err);

    let msg =
        format!("[SSL: CERTIFICATE_VERIFY_FAILED] certificate verify failed: {verify_message}",);

    let exc = vm.new_os_subtype_error(
        PySSLCertVerificationError::class(&vm.ctx).to_owned(),
        None,
        msg,
    );

    // Set verify_code and verify_message attributes
    // Ignore errors as they're extremely rare (e.g., out of memory)
    exc.as_object().set_attr(
        "verify_code",
        vm.ctx.new_int(verify_code).as_object().to_owned(),
        vm,
    )?;
    exc.as_object().set_attr(
        "verify_message",
        vm.ctx.new_str(verify_message).as_object().to_owned(),
        vm,
    )?;

    exc.as_object()
        .set_attr("library", vm.ctx.new_str("SSL").as_object().to_owned(), vm)?;
    exc.as_object().set_attr(
        "reason",
        vm.ctx
            .new_str("CERTIFICATE_VERIFY_FAILED")
            .as_object()
            .to_owned(),
        vm,
    )?;

    Ok(exc.upcast())
}

/// Error types matching OpenSSL error codes
#[derive(Debug)]
pub(super) enum SslError {
    /// SSL_ERROR_WANT_READ
    WantRead,
    /// SSL_ERROR_WANT_WRITE
    WantWrite,
    /// SSL_ERROR_SYSCALL
    Syscall(String),
    /// SSL_ERROR_SSL
    Ssl(String),
    /// SSL_ERROR_ZERO_RETURN (clean closure with close_notify)
    ZeroReturn,
    /// Unexpected EOF without close_notify (protocol violation)
    Eof,
    /// Non-TLS data received before handshake completed
    PreauthData,
    /// Certificate verification error
    CertVerification(rustls::CertificateError),
    /// I/O error
    Io(std::io::Error),
    /// Timeout error (socket.timeout)
    Timeout(String),
    /// Python exception (pass through directly)
    Py(PyBaseExceptionRef),
    /// Preserve the TLS error until the Python exception boundary.
    Rustls(rustls::Error),
    /// TLS alert received with OpenSSL-compatible error code
    AlertReceived { lib: i32, reason: i32 },
    /// NO_SHARED_CIPHER error (OpenSSL SSL_R_NO_SHARED_CIPHER)
    NoCipherSuites,
}

impl SslError {
    /// Convert TLS alert code to OpenSSL error reason code
    /// OpenSSL uses reason = 1000 + alert_code for TLS alerts
    fn alert_to_openssl_reason(alert: rustls::AlertDescription) -> i32 {
        // AlertDescription can be converted to u8 via as u8 cast
        1000 + (u8::from(alert) as i32)
    }

    /// Convert rustls error to SslError
    pub(super) fn from_rustls(err: rustls::Error) -> Self {
        Self::Rustls(err)
    }

    pub(super) fn is_eof(&self) -> bool {
        match self {
            Self::Rustls(error) => matches!(Self::map_rustls(error.clone()), Self::Eof),
            Self::Eof => true,
            _ => false,
        }
    }

    pub(super) fn is_zero_return(&self) -> bool {
        matches!(
            self,
            Self::ZeroReturn
                | Self::Rustls(rustls::Error::AlertReceived(
                    rustls::AlertDescription::CloseNotify
                ))
        )
    }

    fn map_rustls(err: rustls::Error) -> Self {
        match err {
            rustls::Error::InvalidCertificate(cert_err) => Self::CertVerification(cert_err),
            rustls::Error::AlertReceived(alert_desc) => {
                // Map TLS alerts to OpenSSL-compatible error codes
                // lib = 20 (ERR_LIB_SSL), reason = 1000 + alert_code
                match alert_desc {
                    rustls::AlertDescription::CloseNotify => {
                        // Special case: close_notify is handled as ZeroReturn
                        Self::ZeroReturn
                    }
                    _ => {
                        // All other alerts: convert to OpenSSL error code
                        // This includes InternalError (80 -> reason 1080)
                        Self::AlertReceived {
                            lib: ERR_LIB_SSL,
                            reason: Self::alert_to_openssl_reason(alert_desc),
                        }
                    }
                }
            }
            // OpenSSL 3.0 changed transport EOF from SSL_ERROR_SYSCALL with
            // zero return value to SSL_ERROR_SSL with SSL_R_UNEXPECTED_EOF_WHILE_READING.
            // In rustls, these cases correspond to unexpected connection closure:
            rustls::Error::InvalidMessage(_) => {
                // UnexpectedMessage, CorruptMessage, etc. → SSLEOFError
                // Matches CPython's "EOF occurred in violation of protocol"
                Self::Eof
            }
            rustls::Error::PeerIncompatible(peer_err) => {
                // Check for specific incompatibility types
                use rustls::PeerIncompatible;
                match peer_err {
                    PeerIncompatible::NoCipherSuitesInCommon => {
                        // Maps to OpenSSL SSL_R_NO_SHARED_CIPHER (lib=20, reason=193)
                        Self::NoCipherSuites
                    }
                    _ => {
                        // Other protocol incompatibilities → SSLEOFError
                        Self::Eof
                    }
                }
            }
            _ => Self::Ssl(format!("{err}")),
        }
    }

    /// Create SSLError with library and reason from string values
    ///
    /// This is the base helper for creating SSLError with _library and _reason
    /// attributes when you already have the string values.
    ///
    /// # Arguments
    /// * `vm` - Virtual machine reference
    /// * `library` - Library name (e.g., "PEM", "SSL")
    /// * `reason` - Error reason (e.g., "PEM lib", "NO_SHARED_CIPHER")
    /// * `message` - Main error message
    ///
    /// # Returns
    /// PyBaseExceptionRef with _library and _reason attributes set
    ///
    /// # Note
    /// If attribute setting fails (extremely rare), returns the exception without attributes
    pub(super) fn create_ssl_error_with_reason(
        vm: &VirtualMachine,
        library: Option<&str>,
        reason: &str,
        message: impl Into<String>,
    ) -> PyBaseExceptionRef {
        let msg = message.into();
        // SSLError args should be (errno, message) format
        // FIXME: Use 1 as generic SSL error code
        let exc = vm.new_os_subtype_error(PySSLError::class(&vm.ctx).to_owned(), Some(1), msg);

        // Set library and reason attributes
        // Ignore errors as they're extremely rare (e.g., out of memory)
        let library_obj = match library {
            Some(lib) => vm.ctx.new_str(lib).as_object().to_owned(),
            None => vm.ctx.none(),
        };
        let _ = exc.as_object().set_attr("library", library_obj, vm);
        let _ =
            exc.as_object()
                .set_attr("reason", vm.ctx.new_str(reason).as_object().to_owned(), vm);

        exc.upcast()
    }

    /// Create SSLError with library and reason from ssl_data codes
    ///
    /// This helper converts OpenSSL numeric error codes to Python SSLError exceptions
    /// with proper _library and _reason attributes by looking up the error strings
    /// in ssl_data tables, then delegates to create_ssl_error_with_reason.
    ///
    /// # Arguments
    /// * `vm` - Virtual machine reference
    /// * `lib` - OpenSSL library code (e.g., ERR_LIB_SSL = 20)
    /// * `reason` - OpenSSL reason code (e.g., SSL_R_NO_SHARED_CIPHER = 193)
    ///
    /// # Returns
    /// PyBaseExceptionRef with _library and _reason attributes set
    fn create_ssl_error_from_codes(
        vm: &VirtualMachine,
        lib: i32,
        reason: i32,
    ) -> PyBaseExceptionRef {
        // Look up error strings from ssl_data tables
        let key = ssl_data::encode_error_key(lib, reason);
        let reason_str = ssl_data::ERROR_CODES
            .get(&key)
            .copied()
            .unwrap_or("unknown error");

        let lib_str = ssl_data::LIBRARY_CODES
            .get(&(lib as u32))
            .copied()
            .unwrap_or("UNKNOWN");

        // Delegate to create_ssl_error_with_reason for actual exception creation
        Self::create_ssl_error_with_reason(
            vm,
            Some(lib_str),
            reason_str,
            format!("[SSL] {reason_str}"),
        )
    }

    /// Convert to Python exception
    pub(super) fn into_py_err(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        match self {
            Self::Rustls(error) => Self::map_rustls(error).into_py_err(vm),
            Self::WantRead => create_ssl_want_read_error(vm).upcast(),
            Self::WantWrite => create_ssl_want_write_error(vm).upcast(),
            Self::Timeout(msg) => timeout_error_msg(vm, msg).upcast(),
            Self::Syscall(msg) => {
                // SSLSyscallError with errno=SSL_ERROR_SYSCALL (5)
                create_ssl_syscall_error(vm, msg).upcast()
            }
            Self::Ssl(msg) => vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    format!("SSL error: {msg}"),
                )
                .upcast(),
            Self::ZeroReturn => create_ssl_zero_return_error(vm).upcast(),
            Self::Eof => create_ssl_eof_error(vm).upcast(),
            Self::PreauthData => {
                // Non-TLS data received before handshake
                Self::create_ssl_error_with_reason(
                    vm,
                    None,
                    "before TLS handshake with data",
                    "before TLS handshake with data",
                )
            }
            Self::CertVerification(cert_err) => {
                // Use the proper cert verification error creator
                create_ssl_cert_verification_error(vm, &cert_err).expect("unlikely to happen")
            }
            Self::Io(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                create_ssl_eof_error(vm).upcast()
            }
            Self::Io(err) if err.raw_os_error().is_none() => vm
                .new_os_subtype_error(
                    PySSLError::class(&vm.ctx).to_owned(),
                    None,
                    format!("SSL error: {err}"),
                )
                .upcast(),
            Self::Io(err) => err.into_pyexception(vm),

            Self::Py(exc) => exc,
            Self::AlertReceived { lib, reason } => {
                Self::create_ssl_error_from_codes(vm, lib, reason)
            }
            Self::NoCipherSuites => {
                // OpenSSL error: lib=20 (ERR_LIB_SSL), reason=193 (SSL_R_NO_SHARED_CIPHER)
                Self::create_ssl_error_from_codes(vm, ERR_LIB_SSL, SSL_R_NO_SHARED_CIPHER)
            }
        }
    }
}

pub(super) type SslResult<T> = Result<T, SslError>;
pub(super) use rustpython_host_env::ssl::config::{
    ClientConfigOptions, MultiCertResolver, ProtocolSettings, ServerConfigOptions,
    create_client_config, create_server_config, curve_name_to_kx_group,
};

/// Helper function - check if error is BlockingIOError
pub(super) fn is_blocking_io_error(err: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    err.fast_isinstance(vm.ctx.exceptions.blocking_io_error)
}

// Socket I/O Helper Functions

/// Run `f` on the rustls connection. Do not perform socket or MemoryBIO I/O
/// inside `f`; rustls is sans-I/O and another thread may need this lock to
/// encrypt or decrypt while we wait on the transport.
fn with_conn_mut<R>(
    socket: &PySSLSocket,
    vm: &VirtualMachine,
    f: impl FnOnce(&mut Connection) -> SslResult<R>,
) -> SslResult<R> {
    let mut guard = socket.connection().lock();
    let conn = guard
        .as_mut()
        .ok_or_else(|| SslError::Py(vm.new_value_error("Connection not established")))?;
    f(conn)
}

/// Send all bytes to socket, handling partial sends with blocking wait
///
/// Loops until all bytes are sent. For blocking sockets, this will wait
/// until all data is sent. For non-blocking sockets, returns WantWrite
/// if no progress can be made.
/// Optional deadline parameter allows respecting a read deadline during flush.
pub(super) fn send_all_bytes(
    socket: &PySSLSocket,
    buf: Vec<u8>,
    vm: &VirtualMachine,
    deadline: Option<std::time::Instant>,
) -> SslResult<()> {
    // Retain newly drained records before a fallible flush of earlier output.
    socket.observe_tls(true, &buf, vm);
    socket.pending_tls_output.lock().extend_from_slice(&buf);
    socket
        .flush_pending_tls_output(vm, deadline)
        .map_err(|error| {
            if error.fast_isinstance(PySSLWantWriteError::class(&vm.ctx)) {
                SslError::WantWrite
            } else if error.fast_isinstance(vm.ctx.exceptions.timeout_error) {
                SslError::Timeout("The write operation timed out".to_owned())
            } else {
                SslError::Py(error)
            }
        })
}

// Handshake Helper Functions

/// Write TLS handshake data to socket/BIO
///
/// Drains all pending TLS data from rustls and sends it to the peer.
/// Returns whether any progress was made.
fn handshake_write_loop(socket: &PySSLSocket, vm: &VirtualMachine) -> SslResult<bool> {
    let mut made_progress = false;

    // Flush any previously pending TLS data before generating new output
    // Must succeed before sending new data to maintain order
    socket
        .flush_pending_tls_output(vm, None)
        .map_err(SslError::Py)?;

    loop {
        let wants_write = with_conn_mut(socket, vm, |conn| Ok(conn.wants_write()))?;
        if !wants_write {
            break;
        }
        let buf = with_conn_mut(socket, vm, ssl_write_tls_records)?;
        if buf.is_empty() {
            break;
        }
        send_all_bytes(socket, buf, vm, None)?;
        made_progress = true;
    }

    Ok(made_progress)
}

/// Read at most one TLS record from the TCP socket.
///
/// May return incomplete data but never returns more when completes a
/// previously incomplete TLS record.
///
/// OpenSSL reads one TLS record at a time (no read-ahead by default).
/// Rustls, however, consumes all available TCP data when fed via read_tls().
/// If a close_notify or other control record arrives alongside application
/// data, the eager read drains the TCP buffer, leaving the control record in
/// rustls's internal buffer where select() cannot see it.  This causes
/// asyncore-based servers (which rely on select() for readability) to miss
/// the data and the peer times out.
///
/// Fix: peek at the TCP buffer to find the first complete TLS record boundary
/// and recv() only that many bytes.  Any remaining data stays in the kernel
/// buffer and remains visible to select().
pub(super) fn recv_at_most_one_tls_record(
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<PyObjectRef> {
    let bytes = socket.sock_recv_at_most_one_tls_record(vm).map_err(|e| {
        if is_blocking_io_error(&e, vm) {
            SslError::WantRead
        } else {
            SslError::Py(e)
        }
    })?;
    socket.observe_tls(false, bytes.as_bytes(), vm);
    if bytes.is_empty() {
        Err(if socket.is_bio_mode() && !socket.transport_eof() {
            SslError::WantRead
        } else {
            SslError::Eof
        })
    } else {
        Ok(bytes.into())
    }
}

/// Read up to a single TLS record for post-handshake I/O while preserving the
/// SSL-vs-socket error precedence from the old sock_recv() path.
fn recv_at_most_one_tls_record_for_data(
    socket: &PySSLSocket,
    vm: &VirtualMachine,
) -> SslResult<PyObjectRef> {
    match recv_at_most_one_tls_record(socket, vm) {
        Ok(data) => Ok(data),
        Err(SslError::Eof) => {
            with_conn_mut(socket, vm, |conn| {
                conn.process_new_packets().map_err(SslError::from_rustls)?;
                Ok(())
            })?;
            Ok(vm.ctx.new_bytes(vec![]).into())
        }
        Err(SslError::Py(e)) => {
            with_conn_mut(socket, vm, |conn| {
                conn.process_new_packets().map_err(SslError::from_rustls)?;
                Ok(())
            })?;
            if is_connection_closed_error(&e, vm) {
                return Err(SslError::Eof);
            }
            Err(SslError::Py(e))
        }
        Err(e) => Err(e),
    }
}

fn handshake_read_data(socket: &PySSLSocket, vm: &VirtualMachine) -> SslResult<()> {
    if socket
        .sock_wait_for_io_impl(SockWaitKind::Read, vm)
        .map_err(SslError::Py)?
    {
        return Err(SslError::Timeout(
            "The handshake operation timed out".to_owned(),
        ));
    }
    let data = recv_at_most_one_tls_record(socket, vm)?;
    with_conn_mut(socket, vm, |conn| {
        ssl_read_tls_records(conn, data, socket.is_bio_mode(), vm)
    })
}

/// Try to read plaintext data from TLS connection buffer
///
/// Returns Ok(Some(n)) if n bytes were read, Ok(None) if would block,
/// or Err on real errors.
fn try_read_plaintext(conn: &mut Connection, buf: &mut [u8]) -> SslResult<Option<usize>> {
    let mut reader = conn.reader();
    match reader.read(buf) {
        Ok(0) => {
            // EOF from TLS connection
            Ok(Some(0))
        }
        Ok(n) => {
            // Successfully read n bytes
            Ok(Some(n))
        }
        Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
            // Real error
            Err(SslError::Io(e))
        }
        Err(_) => {
            // WouldBlock - no plaintext available
            Ok(None)
        }
    }
}

/// Equivalent to OpenSSL's SSL_do_handshake()
///
/// Performs TLS handshake by exchanging data with the peer until completion.
/// This abstracts away the low-level rustls read_tls/write_tls loop.
///
/// = SSL_do_handshake()
pub(super) fn ssl_do_handshake(socket: &PySSLSocket, vm: &VirtualMachine) -> SslResult<()> {
    loop {
        // Both transports drain writes first and feed complete/partial records
        // through the same path. An empty BIO naturally returns WantRead.
        handshake_write_loop(socket, vm)?;
        let keep_going = with_conn_mut(socket, vm, |conn| {
            if !conn.is_handshaking() {
                return Ok(false);
            }
            if !conn.wants_read() {
                return Err(SslError::WantRead);
            }
            Ok(true)
        })?;
        if !keep_going {
            return Ok(());
        }
        handshake_read_data(socket, vm)?;
        with_conn_mut(socket, vm, |conn| {
            if let Err(error) = conn.process_new_packets() {
                return Err(if matches!(error, rustls::Error::InvalidMessage(_)) {
                    SslError::PreauthData
                } else {
                    SslError::from_rustls(error)
                });
            }
            Ok(())
        })?;
    }
}

/// Equivalent to OpenSSL's SSL_read()
///
/// Reads application data from TLS connection.
/// Automatically handles TLS record I/O as needed.
///
/// = SSL_read_ex()
pub(super) fn ssl_read(
    socket: &PySSLSocket,
    buf: &mut [u8],
    vm: &VirtualMachine,
) -> SslResult<usize> {
    let is_bio = socket.is_bio_mode();

    // Get socket timeout and calculate deadline (= _PyDeadline_Init)
    let deadline = if !is_bio {
        match socket.get_socket_timeout(vm).map_err(SslError::Py)? {
            Some(timeout) if !timeout.is_zero() => Some(std::time::Instant::now() + timeout),
            _ => None, // None = blocking (no deadline), Some(0) = non-blocking (handled below)
        }
    } else {
        None // BIO mode has no deadline
    };

    // CRITICAL: Flush any pending TLS output before reading
    // This ensures data from previous write() calls is sent before we wait for response.
    // Without this, write() may leave data in pending_tls_output (if socket buffer was full),
    // and read() would timeout waiting for a response that the server never received.
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

    // Loop to handle TLS records and post-handshake messages
    // Matches SSL_read behavior which loops until data is available
    //   - CPython uses OpenSSL's SSL_read which loops on SSL_ERROR_WANT_READ/WANT_WRITE
    //   - We use rustls which requires manual read_tls/process_new_packets loop
    //   - No iteration limit: relies on deadline and blocking I/O
    //   - Blocking sockets: sock_select() and recv() wait at kernel level (no CPU busy-wait)
    //   - Non-blocking sockets: immediate return on first WantRead
    //   - Deadline prevents timeout issues

    loop {
        // Check deadline
        if let Some(deadline) = deadline
            && std::time::Instant::now() >= deadline
        {
            // Timeout expired
            return Err(SslError::Timeout(
                "The read operation timed out".to_string(),
            ));
        }
        // Check if we need to read more TLS records BEFORE trying plaintext read
        // This ensures we don't miss data that's already been processed
        let (needs_more_tls, plaintext) = with_conn_mut(socket, vm, |conn| {
            Ok((conn.wants_read(), try_read_plaintext(conn, buf)?))
        })?;

        if let Some(n) = plaintext {
            if n == 0 {
                // EOF from TLS - close_notify received
                // Return ZeroReturn so Python raises SSLZeroReturnError
                return Err(SslError::ZeroReturn);
            }
            return Ok(n);
        }

        // No plaintext available and rustls doesn't want to read more TLS records
        if !needs_more_tls {
            // Check if connection needs to write data first (e.g., TLS key update, renegotiation)
            // This mirrors the handshake logic which checks both wants_read() and wants_write()
            let wants_write = with_conn_mut(socket, vm, |conn| Ok(conn.wants_write()))?;
            if wants_write && !is_bio {
                // Check deadline BEFORE attempting flush
                if let Some(deadline) = deadline
                    && std::time::Instant::now() >= deadline
                {
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
                }

                // Flush pending TLS data before continuing
                // CRITICAL: Pass deadline so flush respects read timeout
                let tls_data = with_conn_mut(socket, vm, ssl_write_tls_records)?;
                if !tls_data.is_empty() {
                    // Use best-effort send - don't fail READ just because WRITE couldn't complete
                    match send_all_bytes(socket, tls_data, vm, deadline) {
                        Ok(()) => {}
                        Err(SslError::WantWrite) => {
                            // Socket buffer full - acceptable during READ operation
                            // Pending data will be sent on next write/read call
                        }
                        Err(SslError::Timeout(_)) => {
                            // Timeout during flush is acceptable during READ
                            // Pending data stays buffered for next operation
                        }
                        Err(e) => return Err(e),
                    }
                }

                // Check deadline AFTER flush attempt
                if let Some(deadline) = deadline
                    && std::time::Instant::now() >= deadline
                {
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
                }

                // After flushing, rustls may want to read again - continue loop
                continue;
            }

            // BIO mode: check for EOF
            if is_bio && let Some(bio_obj) = socket.incoming_bio() {
                let is_eof = bio_obj
                    .get_attr("eof", vm)
                    .and_then(|v| v.try_into_value::<bool>(vm))
                    .unwrap_or(false);
                if is_eof {
                    return Err(SslError::Eof);
                }
            }

            // For non-blocking sockets, return WantRead so caller can poll and retry.
            // For blocking sockets (or sockets with timeout), wait for more data.
            if !is_bio {
                let timeout = socket.get_socket_timeout(vm).map_err(SslError::Py)?;
                if let Some(t) = timeout
                    && t.is_zero()
                {
                    // Non-blocking socket: check if peer has closed before returning WantRead
                    // If close_notify was received, we should return ZeroReturn (EOF), not WantRead
                    // This is critical for asyncore-based applications that rely on recv() returning
                    // 0 or raising SSL_ERROR_ZERO_RETURN to detect connection close.
                    let peer_closed = with_conn_mut(socket, vm, |conn| {
                        conn.process_new_packets()
                            .map(|io_state| io_state.peer_has_closed())
                            .map_err(SslError::from_rustls)
                    })?;
                    if peer_closed {
                        return Err(SslError::ZeroReturn);
                    }
                    // Non-blocking socket: return immediately
                    return Err(SslError::WantRead);
                }
                // Blocking socket or socket with timeout: try to read more data from socket.
                // Even though rustls says it doesn't want to read, more TLS records may arrive.
                // Use single-record reading to avoid consuming close_notify alongside data.
                let data = recv_at_most_one_tls_record_for_data(socket, vm)?;

                let bytes_read = data
                    .clone()
                    .try_into_value::<rustpython_vm::builtins::PyBytes>(vm)
                    .map_or(0, |b| b.as_bytes().len());

                if bytes_read == 0 {
                    // No more data available - check if this is clean shutdown or unexpected EOF
                    // If close_notify was already received, return ZeroReturn (clean closure)
                    // Otherwise, return Eof (unexpected EOF)
                    let peer_closed = with_conn_mut(socket, vm, |conn| {
                        conn.process_new_packets()
                            .map(|io_state| io_state.peer_has_closed())
                            .map_err(SslError::from_rustls)
                    })?;
                    if peer_closed {
                        return Err(SslError::ZeroReturn);
                    }
                    return Err(SslError::Eof);
                }

                // Feed data to rustls and process
                with_conn_mut(socket, vm, |conn| {
                    ssl_read_tls_records(conn, data, false, vm)?;
                    conn.process_new_packets().map_err(SslError::from_rustls)?;
                    Ok(())
                })?;

                // Continue loop to try reading plaintext
                continue;
            }

            return Err(SslError::WantRead);
        }

        // Read and process TLS records
        match ssl_ensure_data_available(socket, vm) {
            Ok(_bytes_read) => {
                // Successfully read and processed TLS data
                // Continue loop to try reading plaintext
            }
            Err(e) => {
                // Other errors - check for buffered plaintext before propagating
                match with_conn_mut(socket, vm, |conn| try_read_plaintext(conn, buf))? {
                    Some(n) if n > 0 => {
                        // Have buffered plaintext - return it successfully
                        return Ok(n);
                    }
                    _ => {
                        // No buffered data - propagate the error
                        return Err(e);
                    }
                }
            }
        }
    }
}

/// Equivalent to OpenSSL's SSL_write()
///
/// Writes application data to TLS connection.
/// Automatically handles TLS record I/O as needed.
///
/// = SSL_write_ex()
pub(super) fn ssl_write(
    socket: &PySSLSocket,
    data: &[u8],
    vm: &VirtualMachine,
) -> SslResult<usize> {
    if data.is_empty() {
        return Ok(0);
    }

    let is_bio = socket.is_bio_mode();

    // Get socket timeout and calculate deadline (= _PyDeadline_Init)
    let deadline = if !is_bio {
        match socket.get_socket_timeout(vm).map_err(SslError::Py)? {
            Some(timeout) if !timeout.is_zero() => Some(std::time::Instant::now() + timeout),
            _ => None,
        }
    } else {
        None
    };

    // Flush any pending TLS output before writing new data
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

    // Check if we already have data buffered from a previous retry
    // (prevents duplicate writes when retrying after WantWrite/WantRead)
    let already_buffered = *socket.write_buffered_len.lock();

    // Only write plaintext if not already buffered
    // Track how much we wrote for partial write handling
    let mut bytes_written_to_rustls = 0usize;

    if already_buffered == 0 {
        // Write plaintext to rustls (= SSL_write_ex internal buffer write)
        bytes_written_to_rustls = with_conn_mut(socket, vm, |conn| {
            let mut writer = conn.writer();
            use std::io::Write;
            // Use write() instead of write_all() to support partial writes.
            // In BIO mode (asyncio), when the internal buffer is full,
            // we want to write as much as possible and return that count,
            // rather than failing completely.
            match writer.write(data) {
                Ok(0) if !data.is_empty() => {
                    // Buffer is full and nothing could be written.
                    // In BIO mode, return WantWrite so the caller can
                    // drain the outgoing BIO and retry.
                    if is_bio {
                        return Err(SslError::WantWrite);
                    }
                    Err(SslError::Syscall("Write failed: buffer full".to_string()))
                }
                Ok(n) => Ok(n),
                Err(e) => {
                    if is_bio {
                        // In BIO mode, treat write errors as WantWrite
                        return Err(SslError::WantWrite);
                    }
                    Err(SslError::Syscall(format!("Write failed: {e}")))
                }
            }
        })?;
        // Mark data as buffered (only the portion we actually wrote)
        *socket.write_buffered_len.lock() = bytes_written_to_rustls;
    } else if already_buffered != data.len() {
        // Caller is retrying with different data - this is a protocol error
        // Clear the buffer state and return an SSL error (bad write retry)
        *socket.write_buffered_len.lock() = 0;
        return Err(SslError::Ssl("bad write retry".to_string()));
    }
    // else: already_buffered == data.len(), this is a valid retry

    // Loop to send TLS records, handling WANT_READ/WANT_WRITE
    // Matches CPython's do-while loop on SSL_ERROR_WANT_READ/WANT_WRITE
    loop {
        // Check deadline
        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
        {
            return Err(SslError::Timeout(
                "The write operation timed out".to_string(),
            ));
        }

        // Check if rustls has TLS data to send
        let wants_write = with_conn_mut(socket, vm, |conn| Ok(conn.wants_write()))?;
        if !wants_write {
            // All TLS data sent successfully
            break;
        }

        // Get TLS records from rustls
        let tls_data = with_conn_mut(socket, vm, ssl_write_tls_records)?;
        if tls_data.is_empty() {
            break;
        }

        // Send TLS data to socket
        match send_all_bytes(socket, tls_data, vm, deadline) {
            Ok(()) => {
                // Successfully sent, continue loop to check for more data
            }
            Err(SslError::WantWrite) => {
                // Non-blocking socket would block - return WANT_WRITE
                // If we had a partial write to rustls, return partial success
                // instead of error to match OpenSSL partial-write semantics
                if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                    *socket.write_buffered_len.lock() = 0;
                    return Ok(bytes_written_to_rustls);
                }
                // Keep write_buffered_len set so we don't re-buffer on retry
                return Err(SslError::WantWrite);
            }
            Err(SslError::WantRead) => {
                // Need to read before write can complete (e.g., renegotiation)
                if is_bio {
                    // If we had a partial write to rustls, return partial success
                    if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                        *socket.write_buffered_len.lock() = 0;
                        return Ok(bytes_written_to_rustls);
                    }
                    // Keep write_buffered_len set so we don't re-buffer on retry
                    return Err(SslError::WantRead);
                }
                // For socket mode, try to read TLS data
                let recv_result = recv_at_most_one_tls_record_for_data(socket, vm)?;
                with_conn_mut(socket, vm, |conn| {
                    ssl_read_tls_records(conn, recv_result, false, vm)?;
                    conn.process_new_packets().map_err(SslError::from_rustls)?;
                    Ok(())
                })?;
                // Continue loop
            }
            Err(e @ SslError::Timeout(_)) => {
                // If we had a partial write to rustls, return partial success
                if bytes_written_to_rustls > 0 && bytes_written_to_rustls < data.len() {
                    *socket.write_buffered_len.lock() = 0;
                    return Ok(bytes_written_to_rustls);
                }
                // Preserve buffered state so retry doesn't duplicate data
                // (send_all_bytes saved unsent TLS bytes to pending_tls_output)
                return Err(e);
            }
            Err(e) => {
                // Clear buffer state on error
                *socket.write_buffered_len.lock() = 0;
                return Err(e);
            }
        }
    }

    // Final flush to ensure all data is sent
    if !is_bio {
        socket
            .flush_pending_tls_output(vm, deadline)
            .map_err(SslError::Py)?;
    }

    // Determine how many bytes we actually wrote
    let actual_written = if bytes_written_to_rustls > 0 {
        // Fresh write: return what we wrote to rustls
        bytes_written_to_rustls
    } else if already_buffered > 0 {
        // Retry of previous write: return the full buffered amount
        already_buffered
    } else {
        data.len()
    };

    // Write completed successfully - clear buffer state
    *socket.write_buffered_len.lock() = 0;

    Ok(actual_written)
}

// Helper functions (private-ish, used by public SSL functions)

/// Write TLS records from rustls to socket
fn ssl_write_tls_records(conn: &mut Connection) -> SslResult<Vec<u8>> {
    let mut buf = Vec::new();
    let n = conn
        .write_tls(&mut buf as &mut dyn std::io::Write)
        .map_err(SslError::Io)?;

    if n > 0 { Ok(buf) } else { Ok(Vec::new()) }
}

/// Read TLS records from socket to rustls
pub(super) fn ssl_read_tls_records(
    conn: &mut Connection,
    data: PyObjectRef,
    is_bio: bool,
    vm: &VirtualMachine,
) -> SslResult<()> {
    // Convert PyObject to bytes-like (supports bytes, bytearray, etc.)
    let bytes = ArgBytesLike::try_from_object(vm, data)
        .map_err(|_| SslError::Syscall("Expected bytes-like object".to_string()))?;

    let bytes_data = bytes.borrow_buf();

    if bytes_data.is_empty() {
        // different error for BIO vs socket mode
        if is_bio {
            // In BIO mode, no data means WANT_READ
            return Err(SslError::WantRead);
        }
        // In socket mode, empty recv() means TCP EOF (FIN received)
        // Need to distinguish:
        // 1. Clean shutdown: received TLS close_notify → return ZeroReturn (0 bytes)
        // 2. Unexpected EOF: no close_notify → return Eof (SSLEOFError)
        //
        // SSL_ERROR_ZERO_RETURN vs SSL_ERROR_EOF logic
        // CPython checks SSL_get_shutdown() & SSL_RECEIVED_SHUTDOWN
        //
        // Process any buffered TLS records (may contain close_notify)
        match conn.process_new_packets() {
            Ok(io_state) => {
                if io_state.peer_has_closed() {
                    // Received close_notify - normal SSL closure (SSL_ERROR_ZERO_RETURN)
                    return Err(SslError::ZeroReturn);
                }
                // No close_notify - ragged EOF (SSL_ERROR_EOF → SSLEOFError)
                // CPython raises SSLEOFError here, which SSLSocket.read() handles
                // based on suppress_ragged_eofs setting
                return Err(SslError::Eof);
            }
            Err(e) => return Err(SslError::from_rustls(e)),
        }
    }

    // Feed all received data to read_tls - loop to consume all data
    // read_tls may not consume all data in one call, and buffer may become full
    let mut offset = 0;
    while offset < bytes_data.len() {
        let remaining = &bytes_data[offset..];
        let mut cursor = std::io::Cursor::new(remaining);

        match conn.read_tls(&mut cursor) {
            Ok(read_bytes) => {
                if read_bytes == 0 {
                    // Buffer is full - process existing packets to make room
                    conn.process_new_packets().map_err(SslError::from_rustls)?;

                    // Try again - if we still can't consume, break
                    let mut retry_cursor = std::io::Cursor::new(remaining);
                    match conn.read_tls(&mut retry_cursor) {
                        Ok(0) => {
                            // Still can't consume - break to avoid infinite loop
                            break;
                        }
                        Ok(n) => {
                            offset += n;
                            if offset < bytes_data.len() {
                                conn.process_new_packets().map_err(SslError::from_rustls)?;
                            }
                        }
                        Err(e) => {
                            return Err(SslError::Io(e));
                        }
                    }
                } else {
                    offset += read_bytes;
                    if offset < bytes_data.len() {
                        conn.process_new_packets().map_err(SslError::from_rustls)?;
                    }
                }
            }
            Err(e) => {
                // Real error - propagate it
                return Err(SslError::Io(e));
            }
        }
    }

    Ok(())
}

/// Check if an exception is a connection closed error
/// In SSL context, these errors indicate unexpected connection termination without proper TLS shutdown
fn is_connection_closed_error(exc: &Py<PyBaseException>, vm: &VirtualMachine) -> bool {
    use rustpython_vm::stdlib::errno::errors;

    // Check for ConnectionAbortedError, ConnectionResetError (Python exception types)
    if exc.fast_isinstance(vm.ctx.exceptions.connection_aborted_error)
        || exc.fast_isinstance(vm.ctx.exceptions.connection_reset_error)
    {
        return true;
    }

    // Also check OSError with specific errno values (ECONNABORTED, ECONNRESET)
    if exc.fast_isinstance(vm.ctx.exceptions.os_error)
        && let Ok(errno) = exc.as_object().get_attr("errno", vm)
        && let Ok(errno_int) = errno.try_int(vm)
        && let Ok(errno_val) = errno_int.try_to_primitive::<i32>(vm)
    {
        return errno_val == errors::ECONNABORTED || errno_val == errors::ECONNRESET;
    }
    false
}

/// Ensure TLS data is available for reading
/// Returns the number of bytes read from the socket
fn ssl_ensure_data_available(socket: &PySSLSocket, vm: &VirtualMachine) -> SslResult<usize> {
    // Unlike OpenSSL's SSL_read, rustls requires explicit I/O
    if with_conn_mut(socket, vm, |conn| Ok(conn.wants_read()))? {
        let is_bio = socket.is_bio_mode();

        // For non-BIO mode (regular sockets), check if socket is ready first
        // PERFORMANCE OPTIMIZATION: Only use select for sockets with timeout
        // - Blocking sockets (timeout=None): Skip select, recv() will block naturally
        // - Timeout sockets: Use select to enforce timeout
        // - Non-blocking sockets: Skip select, recv() will return EAGAIN immediately
        if !is_bio {
            let timeout = socket.get_socket_timeout(vm).map_err(SslError::Py)?;

            // Only use select if socket has a positive timeout
            if let Some(t) = timeout
                && !t.is_zero()
            {
                // Socket has timeout - use select to enforce it
                let timed_out = socket
                    .sock_wait_for_io_impl(SockWaitKind::Read, vm)
                    .map_err(SslError::Py)?;
                if timed_out {
                    // Socket not ready within timeout - raise socket.timeout
                    return Err(SslError::Timeout(
                        "The read operation timed out".to_string(),
                    ));
                }
            }
            // else: non-blocking socket (timeout=0) or blocking socket (timeout=None) - skip select
        }

        // Read one TLS record at a time for non-BIO sockets (matching
        // OpenSSL's default no-read-ahead behaviour).  This prevents
        // consuming a close_notify that arrives alongside application data,
        // keeping it in the kernel buffer where select() can detect it.
        let data = if !is_bio {
            recv_at_most_one_tls_record_for_data(socket, vm)?
        } else {
            match socket.sock_recv(SSL3_RT_MAX_PACKET_SIZE, vm) {
                Ok(data) => data,
                Err(e) => {
                    if is_blocking_io_error(&e, vm) {
                        return Err(SslError::WantRead);
                    }
                    with_conn_mut(socket, vm, |conn| {
                        conn.process_new_packets().map_err(SslError::from_rustls)?;
                        Ok(())
                    })?;
                    if is_connection_closed_error(&e, vm) {
                        return Err(SslError::Eof);
                    }
                    return Err(SslError::Py(e));
                }
            }
        };

        // Get the size of received data
        let bytes_read = data
            .clone()
            .try_into_value::<rustpython_vm::builtins::PyBytes>(vm)
            .map_or(0, |b| b.as_bytes().len());

        // Check if BIO has EOF set (incoming BIO closed)
        let is_eof = if is_bio {
            // Check incoming BIO's eof property
            if let Some(bio_obj) = socket.incoming_bio() {
                bio_obj
                    .get_attr("eof", vm)
                    .and_then(|v| v.try_into_value::<bool>(vm))
                    .unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        };

        // If BIO EOF is set and no data available, treat as connection EOF
        if is_eof && bytes_read == 0 {
            return Err(SslError::Eof);
        }

        if is_bio {
            let bytes = ArgBytesLike::try_from_object(vm, data.clone())
                .map_err(|_| SslError::Syscall("Expected bytes-like object".to_string()))?;
            socket.observe_tls(false, bytes.borrow_buf().as_ref(), vm);
        }

        // Feed data to rustls and process packets
        with_conn_mut(socket, vm, |conn| {
            ssl_read_tls_records(conn, data, is_bio, vm)?;
            conn.process_new_packets().map_err(SslError::from_rustls)?;
            Ok(())
        })?;

        Ok(bytes_read)
    } else {
        // No data to read
        Ok(0)
    }
}
