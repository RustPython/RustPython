import pathlib
import socket
import ssl
import threading
import time

from test.test_ssl import HOST, ThreadedEchoServer, testing_context

CERT = (
    pathlib.Path(__file__).resolve().parent.parent.parent
    / "Lib"
    / "test"
    / "certdata"
    / "keycert.pem"
)

TAIL = b"tail"

server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
server_context.load_cert_chain(str(CERT))
client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
client_context.check_hostname = False
client_context.verify_mode = ssl.CERT_NONE
server_context.maximum_version = client_context.maximum_version = ssl.TLSVersion.TLSv1_2

client_in, client_out = ssl.MemoryBIO(), ssl.MemoryBIO()
server_in, server_out = ssl.MemoryBIO(), ssl.MemoryBIO()
client = client_context.wrap_bio(client_in, client_out)
server = server_context.wrap_bio(server_in, server_out, server_side=True)

# A write that starts the handshake must release its caller's buffer on WANT.
# Save the valid retry before resizing and changing the original exporter.
exporter = bytearray(b"saved application bytes before WANT_READ")
saved_write = bytes(exporter)
try:
    client.write(exporter)
except ssl.SSLWantReadError:
    exporter[:] = b"changed after WANT"
    exporter.extend(b"; no outstanding buffer export")
else:
    raise AssertionError("write before the peer handshake did not need a read")

for _ in range(5):
    try:
        client.do_handshake()
    except ssl.SSLWantReadError:
        pass
    server_in.write(client_out.read())
    try:
        server.do_handshake()
    except ssl.SSLWantReadError:
        pass
    client_in.write(server_out.read())
client.do_handshake()
server.do_handshake()

# OpenSSL retries require the original content and length. Never retry with
# the mutated exporter: behavior for an invalid retry is not a parity claim.
assert client.write(saved_write) == len(saved_write)
server_in.write(client_out.read())
assert server.read(len(saved_write)) == saved_write

try:
    server.unwrap()
except ssl.SSLWantReadError:
    pass
client_in.write(server_out.read() + TAIL)
client.unwrap()
assert client_in.read() == TAIL


TIMEOUT = 10.0


class BoundedEchoServer(ThreadedEchoServer):
    # Keep the canonical accept, handshake and echo implementation. Its normal
    # context manager and handler joins need bounds for a deadlock regression.
    class ConnectionHandler(ThreadedEchoServer.ConnectionHandler):
        def __init__(self, server, connsock, addr):
            super().__init__(server, connsock, addr)
            self.sock.settimeout(TIMEOUT)
            server.handlers.append(self)

        def run(self):
            try:
                try:
                    super().run()
                finally:
                    self.close()
            except BaseException as exc:
                self.server.errors.append(exc)

        def join(self, timeout=TIMEOUT * 10):
            super().join(timeout)
            if self.is_alive():
                self.server.errors.append(
                    AssertionError("echo connection handler did not stop")
                )
                self.server.stop()

    def __init__(self, context):
        self.errors = []
        self.handlers = []
        super().__init__(context=context, chatty=False)

    def __enter__(self):
        assert not self._in_context
        self._in_context = True
        self.start(threading.Event())
        if not self.flag.wait(TIMEOUT):
            self.__exit__()
            raise AssertionError("echo server did not start")
        return self

    def run(self):
        try:
            try:
                super().run()
            finally:
                self.close()
        except BaseException as exc:
            self.errors.append(exc)

    def __exit__(self, *args):
        self.stop()
        self.join(TIMEOUT * 2)
        self._in_context = False
        assert not self.is_alive(), "echo server did not stop"
        for handler in self.handlers:
            handler.join(TIMEOUT)
            assert not handler.is_alive(), "echo connection handler did not stop"
        if self.errors:
            raise self.errors[0]
        assert not self.conn_errors, self.conn_errors


def test_write_return_during_reader_callback():
    client_context, server_context, hostname = testing_context()
    client_context.maximum_version = ssl.TLSVersion.TLSv1_2
    server_context.maximum_version = ssl.TLSVersion.TLSv1_2
    writer_thread = threading.current_thread()
    reader_thread = None
    round_active = False

    def message_callback(conn, direction, version, content_type, msg_type, data):
        if (
            not round_active
            or conn is not connection
            or content_type != ssl._TLSContentType.HEADER
            or msg_type != ssl._TLSContentType.APPLICATION_DATA
        ):
            return
        current_thread = threading.current_thread()
        if (
            direction == "write"
            and current_thread is writer_thread
            and not write_started.is_set()
        ):
            write_started.set()
            assert reader_started.wait(TIMEOUT), "reader did not start"
            # Encourage the reader to queue behind the native writer. Python
            # cannot force its acquisition into the native cleanup window.
            time.sleep(0.01)
        elif (
            direction == "read"
            and current_thread is reader_thread
            and not read_callback_seen.is_set()
        ):
            read_callback_seen.set()
            assert write_returned.wait(TIMEOUT), (
                "write did not return while another thread's read callback ran"
            )

    client_context._msg_callback = message_callback
    with BoundedEchoServer(server_context) as echo_server:
        with client_context.wrap_socket(
            socket.socket(), server_hostname=hostname
        ) as connection:
            connection.settimeout(TIMEOUT)
            connection.connect((HOST, echo_server.port))
            try:
                # This is a cleanup/callback race, not another generic duplex
                # smoke test. The canonical duplex test remains authoritative.
                for _ in range(20):
                    payload = b"WRITE RETURN DURING READER CALLBACK\n"
                    write_started = threading.Event()
                    reader_started = threading.Event()
                    write_returned = threading.Event()
                    read_callback_seen = threading.Event()
                    errors = []

                    def read_echo():
                        try:
                            assert write_started.wait(TIMEOUT), (
                                "writer did not enter its application-record callback"
                            )
                            reader_started.set()
                            received = b""
                            while len(received) < len(payload):
                                chunk = connection.read(len(payload) - len(received))
                                assert chunk, (
                                    "unexpected EOF in echoed application data"
                                )
                                received += chunk
                            assert received == payload.lower(), received
                        except BaseException as exc:
                            errors.append(exc)

                    reader_thread = threading.Thread(target=read_echo, daemon=True)
                    round_active = True
                    reader_thread.start()
                    try:
                        assert connection.write(payload) == len(payload)
                    finally:
                        # Set only after the Python write call has returned or
                        # raised. The reader callback never waits on itself.
                        write_returned.set()
                        reader_thread.join(TIMEOUT * 2)
                        if reader_thread.is_alive():
                            # Interrupt transport I/O without starting SSL I/O.
                            try:
                                socket.socket.shutdown(connection, socket.SHUT_RDWR)
                            except OSError:
                                pass
                            reader_thread.join(TIMEOUT * 2)
                        round_active = False
                        assert not reader_thread.is_alive(), "reader did not stop"
                    if errors:
                        raise errors[0]
                    assert write_started.is_set(), "write callback was not exercised"
                    assert read_callback_seen.is_set(), (
                        "read callback was not exercised"
                    )
                connection.write(b"over\n")
            finally:
                round_active = False
                client_context._msg_callback = None


test_write_return_during_reader_callback()


# Limit initial nonblocking writes to 4 MiB. Small socket buffers plus a peer
# paused after its handshake must actually produce WANT_WRITE on this platform.
WRITE_SIZE = 64 * 1024
MAX_INITIAL_WRITES = 64


class PausedSinkServer(BoundedEchoServer):
    class ConnectionHandler(BoundedEchoServer.ConnectionHandler):
        def __init__(self, server, connsock, addr):
            super().__init__(server, connsock, addr)
            self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)

        def read(self):
            assert self.server.read_enabled.wait(TIMEOUT), "sink was not released"
            data = super().read()
            if not data:
                self.server.reached_eof.set()
            else:
                assert data == b"x" * len(data), "pending write data changed"
                self.server.received += len(data)
                assert self.server.received <= (MAX_INITIAL_WRITES + 1) * WRITE_SIZE
            return data

        def write(self, data):
            # Data was checked before canonical echo processing normalizes it.
            # Send nothing back, avoiding two-peer application backpressure.
            pass

        def run(self):
            try:
                super().run()
            finally:
                self.server.finished.set()

    def __init__(self, context):
        self.read_enabled = threading.Event()
        self.finished = threading.Event()
        self.reached_eof = threading.Event()
        self.received = 0
        super().__init__(context)


def test_exporter_release_after_application_want_write():
    client_context, server_context, hostname = testing_context()
    client_context.maximum_version = ssl.TLSVersion.TLSv1_2
    server_context.maximum_version = ssl.TLSVersion.TLSv1_2
    with PausedSinkServer(server_context) as sink:
        try:
            with client_context.wrap_socket(
                socket.socket(), server_hostname=hostname
            ) as connection:
                connection.settimeout(TIMEOUT)
                connection.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
                connection.connect((HOST, sink.port))
                connection.setblocking(False)
                exporter = bytearray(b"x" * WRITE_SIZE)
                saved_write = bytes(exporter)
                acknowledged = 0
                for _ in range(MAX_INITIAL_WRITES):
                    try:
                        acknowledged += connection.write(exporter)
                    except ssl.SSLWantWriteError:
                        break
                else:
                    raise AssertionError(
                        "bounded application writes never reached WANT_WRITE"
                    )

                # Returning WANT must release the export while native retries
                # retain stable original bytes. Resize and mutate only the old
                # exporter; the retry still uses identical saved bytes/length.
                exporter[:] = b"changed after application WANT_WRITE"
                exporter.extend(b"; exporter is no longer held")
                sink.read_enabled.set()
                # A successful write may accept a prefix. The first retry
                # must still use identical bytes/length; advance only after a
                # positive result, and keep the same pending slice on WANT.
                retry_deadline = time.monotonic() + TIMEOUT
                accepted = 0
                pending = saved_write
                for _ in range(64):
                    remaining = retry_deadline - time.monotonic()
                    assert remaining > 0, "saved-byte retry deadline expired"
                    connection.settimeout(remaining)
                    try:
                        count = connection.write(pending)
                    except (ssl.SSLWantReadError, ssl.SSLWantWriteError):
                        continue
                    assert 0 < count <= len(pending), count
                    accepted += count
                    if accepted == len(saved_write):
                        break
                    pending = saved_write[accepted:]
                else:
                    raise AssertionError("saved-byte retry exceeded 64 attempts")
                assert accepted == len(saved_write)

                # TCP EOF follows every sent TLS byte. Let the sink consume to
                # EOF before checking the total, so extra replay is detected.
                socket.socket.shutdown(connection, socket.SHUT_WR)
                assert sink.finished.wait(TIMEOUT), "sink did not consume through EOF"
                assert sink.reached_eof.is_set(), "sink stopped before observing EOF"
                assert sink.received == acknowledged + len(saved_write), sink.received
        finally:
            # Setup or oracle failure must not strand the paused peer.
            sink.read_enabled.set()


test_exporter_release_after_application_want_write()
