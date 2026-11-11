"""WASIX host integration tests. Run with Wasmer, as documented in wasm/WASIX.md."""

import errno
import gc
import io
import os
import socket
import sys
import time
import unittest


class WasixSocketTests(unittest.TestCase):
    def pair(self, family=socket.AF_INET):
        listener = socket.socket(family=family, type=socket.SOCK_STREAM, proto=0)
        self.addCleanup(listener.close)
        listener.settimeout(2)
        listener.bind(("127.0.0.1" if family == socket.AF_INET else "::1", 0))
        listener.listen()
        client = socket.create_connection(listener.getsockname()[:2], timeout=2)
        self.addCleanup(client.close)
        server, address = listener.accept()
        self.addCleanup(server.close)
        server.settimeout(2)
        self.assertEqual(address, client.getsockname())
        self.assertEqual(client.getpeername(), listener.getsockname())
        return client, server

    def test_constructor_and_bidirectional_tcp(self):
        client, server = self.pair()
        self.assertGreater(client.fileno(), 2)
        self.assertEqual(
            client.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE), socket.SOCK_STREAM
        )
        self.assertEqual(
            server.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE), socket.SOCK_STREAM
        )
        client.sendall(b"client to server")
        self.assertEqual(server.recv(64), b"client to server")
        server.sendall(b"server to client")
        self.assertEqual(client.recv(64), b"server to client")

    def test_ipv6(self):
        client, server = self.pair(socket.AF_INET6)
        client.sendall(b"IPv6")
        self.assertEqual(server.recv(4), b"IPv6")
        self.assertEqual(len(client.getsockname()), 4)

    def test_makefile(self):
        client, server = self.pair()
        with (
            client.makefile("rwb") as client_file,
            server.makefile("rwb") as server_file,
        ):
            client_file.write(b"hello\n")
            client_file.flush()
            self.assertEqual(server_file.readline(), b"hello\n")
            server_file.write(b"reply\n")
            server_file.flush()
            self.assertEqual(client_file.readline(), b"reply\n")

    def test_http_server_and_reverse_lookup_fallback(self):
        import http.server

        with self.assertRaises(OSError) as error:
            socket.gethostbyaddr("127.0.0.1")
        self.assertEqual(error.exception.errno, errno.ENOTSUP)
        self.assertEqual(socket.getfqdn("127.0.0.1"), "127.0.0.1")

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                body = b"HTTP loopback works\n"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args):
                pass

        with http.server.HTTPServer(("127.0.0.1", 0), Handler) as server:
            server.timeout = 2
            with socket.create_connection(server.server_address, timeout=2) as client:
                client.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
                server.handle_request()
                response = bytearray()
                while chunk := client.recv(1024):
                    response.extend(chunk)
                self.assertTrue(response.startswith(b"HTTP/1.0 200 OK\r\n"))
                self.assertTrue(response.endswith(b"HTTP loopback works\n"))

    def test_mutable_send_buffers(self):
        client, server = self.pair()
        data = bytearray(b"mutable")
        self.assertEqual(client.send(data), len(data))
        self.assertEqual(server.recv(32), data)
        client.sendall(memoryview(data))
        self.assertEqual(server.recv(32), data)
        with (
            socket.socket(type=socket.SOCK_DGRAM) as receiver,
            socket.socket(type=socket.SOCK_DGRAM) as sender,
        ):
            receiver.bind(("127.0.0.1", 0))
            receiver.settimeout(2)
            self.assertEqual(sender.sendto(data, receiver.getsockname()), len(data))
            self.assertEqual(receiver.recv(32), data)

    def test_sendfile_and_partial_reads(self):
        client, server = self.pair()
        payload = bytes(range(256)) * 128
        with io.BytesIO(payload) as source:
            self.assertEqual(client.sendfile(source), len(payload))
            self.assertEqual(source.tell(), len(payload))
        received = bytearray()
        while len(received) < len(payload):
            chunk = server.recv(len(payload) - len(received))
            self.assertTrue(chunk)
            received.extend(chunk)
        self.assertEqual(received, payload)

    def test_peek_and_receive_into(self):
        client, server = self.pair()
        client.sendall(b"abcdef")
        self.assertEqual(server.recv(3, socket.MSG_PEEK), b"abc")
        buffer = bytearray(b"........")
        self.assertEqual(server.recv_into(memoryview(buffer)[1:7], 3), 3)
        self.assertEqual(buffer, b".abc....")
        self.assertEqual(server.recv_into(buffer=buffer, nbytes=3, flags=0), 3)
        self.assertEqual(buffer[:3], b"def")
        self.assertEqual(server.recv(0), b"")
        self.assertEqual(server.recv_into(bytearray()), 0)

    def test_udp_and_connected_udp(self):
        with (
            socket.socket(type=socket.SOCK_DGRAM) as receiver,
            socket.socket(type=socket.SOCK_DGRAM) as sender,
        ):
            receiver.settimeout(2)
            sender.settimeout(2)
            receiver.bind(("127.0.0.1", 0))
            sender.bind(("127.0.0.1", 0))
            self.assertEqual(sender.sendto(b"abc", 0, receiver.getsockname()), 3)
            self.assertEqual(receiver.recvfrom(32), (b"abc", sender.getsockname()))
            sender.sendto(b"def", receiver.getsockname())
            buffer = bytearray(6)
            self.assertEqual(
                receiver.recvfrom_into(buffer=buffer, nbytes=4),
                (3, sender.getsockname()),
            )
            self.assertEqual(buffer, b"def\0\0\0")
            sender.connect(receiver.getsockname())
            receiver.connect(sender.getsockname())
            sender.sendall(b"connected")
            self.assertEqual(receiver.recv(32), b"connected")
            # A zero-length UDP datagram is still a datagram.
            sender.sendto(b"", receiver.getsockname())
            self.assertEqual(receiver.recvfrom(0), (b"", sender.getsockname()))
            sender.sendall(b"")
            self.assertEqual(receiver.recvfrom(0), (b"", sender.getsockname()))

    def test_nonblocking_and_timeout(self):
        client, server = self.pair()
        server.setblocking(False)
        self.assertFalse(server.getblocking())
        self.assertEqual(server.gettimeout(), 0)
        with self.assertRaises(BlockingIOError):
            server.recv(1)
        server.settimeout(0.03)
        start = time.monotonic()
        with self.assertRaises(TimeoutError):
            server.recv(1)
        elapsed = time.monotonic() - start
        self.assertGreaterEqual(elapsed, 0.015)
        self.assertLess(elapsed, 2)
        client.sendall(b"x")
        self.assertEqual(server.recv(1), b"x")
        server.setblocking(True)
        self.assertIsNone(server.gettimeout())

    def test_sendall_timeout_under_backpressure(self):
        client, server = self.pair()
        client.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
        client.sendall(b"queued")
        client.settimeout(0.03)
        start = time.monotonic()
        with self.assertRaises(TimeoutError):
            client.sendall(bytes(4 * 1024 * 1024))
        self.assertLess(time.monotonic() - start, 2)
        # A timeout does not discard data queued before the timed operation.
        self.assertTrue(server.recv(1))

    def test_accept_timeout(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()
            listener.settimeout(0.03)
            with self.assertRaises(TimeoutError):
                listener.accept()

    def test_default_timeout(self):
        previous = socket.getdefaulttimeout()
        try:
            socket.setdefaulttimeout(0.05)
            with socket.socket() as sock:
                self.assertEqual(sock.gettimeout(), 0.05)
                self.assertTrue(sock.getblocking())
            for value, error in [
                (-1, ValueError),
                (float("nan"), ValueError),
                (float("inf"), OverflowError),
            ]:
                with self.assertRaises(error):
                    socket.setdefaulttimeout(value)
                self.assertEqual(socket.getdefaulttimeout(), 0.05)
        finally:
            socket.setdefaulttimeout(previous)

    def test_dup_detach_close(self):
        client, server = self.pair()
        duplicate = client.dup()
        self.addCleanup(duplicate.close)
        client.close()
        duplicate.sendall(b"dup")
        self.assertEqual(server.recv(3), b"dup")
        fd = duplicate.detach()
        self.assertEqual(duplicate.fileno(), -1)
        with socket.socket(
            socket.AF_INET, socket.SOCK_STREAM, 0, fileno=fd
        ) as restored:
            self.assertEqual(restored.fileno(), fd)
            restored.sendall(b"fd")
            self.assertEqual(server.recv(2), b"fd")
        self.assertEqual(server.recv(1), b"")
        with self.assertRaises(OSError) as error:
            os.fstat(fd)
        self.assertEqual(error.exception.errno, errno.EBADF)

    def test_explicit_protocols_and_failed_fd_inference(self):
        for kind, protocol in [
            (socket.SOCK_STREAM, socket.IPPROTO_TCP),
            (socket.SOCK_DGRAM, socket.IPPROTO_UDP),
        ]:
            with socket.socket(socket.AF_INET, kind, protocol) as sock:
                self.assertEqual(sock.proto, protocol)
        # Wasmer fdstat identifies connected TCP, but not bound UDP sockets.
        sock = socket.socket(type=socket.SOCK_DGRAM)
        sock.bind(("127.0.0.1", 0))
        fd = sock.detach()
        try:
            with self.assertRaises(OSError) as error:
                socket.socket(fileno=fd)
            self.assertEqual(error.exception.errno, errno.ENOTSUP)
            os.fstat(fd)
            # A caller's type assertion must not become a fake SO_TYPE result.
            wrapped = socket.socket(socket.AF_INET, socket.SOCK_STREAM, fileno=fd)
            try:
                with self.assertRaises(OSError) as error:
                    wrapped.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE)
                self.assertEqual(error.exception.errno, errno.ENOTSUP)
            finally:
                wrapped.detach()
        finally:
            socket.close(fd)

    def test_close_reports_error_without_retry(self):
        sock = socket.socket()
        os.close(sock.fileno())
        with self.assertRaises(OSError) as error:
            sock.close()
        self.assertEqual(error.exception.errno, errno.EBADF)
        self.assertEqual(sock.fileno(), -1)
        sock.close()

    def test_shutdown(self):
        client, server = self.pair()
        client.sendall(b"last")
        client.shutdown(socket.SHUT_WR)
        self.assertEqual(server.recv(4), b"last")
        self.assertEqual(server.recv(1), b"")
        server.sendall(b"still reading")
        self.assertEqual(client.recv(32), b"still reading")

    def test_gc_releases_descriptor(self):
        sock = socket.socket()
        fd = sock.fileno()
        del sock
        gc.collect()
        with self.assertRaises(OSError) as error:
            os.fstat(fd)
        self.assertEqual(error.exception.errno, errno.EBADF)

    def test_closed_descriptor(self):
        sock = socket.socket()
        sock.close()
        sock.close()
        self.assertEqual(sock.fileno(), -1)
        for action in [
            lambda: sock.send(b"x"),
            lambda: sock.sendall(b""),
            lambda: sock.recv(1),
            sock.getsockname,
            lambda: sock.setblocking(False),
        ]:
            with self.assertRaises(OSError) as error:
                action()
            self.assertEqual(error.exception.errno, errno.EBADF)

    def test_options_and_unsupported_operations(self):
        with socket.socket() as sock:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            self.assertEqual(sock.getsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR), 1)
            self.assertEqual(sock.getsockopt(socket.SOL_SOCKET, socket.SO_ERROR), 0)
            with self.assertRaises(OSError):
                sock.setsockopt(socket.SOL_SOCKET, 0x7FFFFFFF, 1)
            with self.assertRaises(OSError):
                sock.send(b"x", socket.MSG_OOB)
        for args in [
            (socket.AF_UNIX, socket.SOCK_STREAM),
            (socket.AF_INET, socket.SOCK_RAW),
            (socket.AF_INET, socket.SOCK_STREAM, socket.IPPROTO_UDP),
        ]:
            with self.assertRaises(OSError):
                socket.socket(*args)

    def test_address_validation(self):
        with socket.socket() as sock:
            for address, error in [
                (("127.0.0.1", -1), OverflowError),
                (("127.0.0.1", 65536), OverflowError),
                (("127.0.0.1", 0, 0), TypeError),
                ((123, 0), TypeError),
            ]:
                with self.assertRaises(error):
                    sock.bind(address)
            with self.assertRaises(ValueError):
                sock.recv(-1)
            with self.assertRaises(ValueError):
                sock.recv_into(bytearray(1), 2)
        with socket.socket(socket.AF_INET6) as sock:
            with self.assertRaises(OSError):
                sock.bind(("::1", 0, 0, 1))

    def test_dns_and_numeric_resolution(self):
        self.assertEqual(socket.gethostbyname("127.0.0.1"), "127.0.0.1")
        self.assertEqual(socket.gethostbyname("localhost"), "127.0.0.1")
        results = socket.getaddrinfo("localhost", 12345, type=socket.SOCK_STREAM)
        self.assertTrue(results)
        for family, kind, proto, canonical, address in results:
            self.assertIn(family, (socket.AF_INET, socket.AF_INET6))
            self.assertEqual(
                (kind, proto, canonical), (socket.SOCK_STREAM, socket.IPPROTO_TCP, "")
            )
            self.assertEqual(address[1], 12345)
        self.assertEqual(
            socket.getaddrinfo(
                "127.0.0.1",
                "80",
                socket.AF_INET,
                socket.SOCK_STREAM,
                0,
                socket.AI_NUMERICHOST,
            ),
            [
                (
                    socket.AF_INET,
                    socket.SOCK_STREAM,
                    socket.IPPROTO_TCP,
                    "",
                    ("127.0.0.1", 80),
                )
            ],
        )
        self.assertEqual(
            socket.getaddrinfo(
                None, 0, socket.AF_INET, socket.SOCK_DGRAM, 0, socket.AI_PASSIVE
            )[0][-1],
            ("0.0.0.0", 0),
        )
        with self.assertRaises(socket.gaierror):
            socket.getaddrinfo("localhost", 0, flags=socket.AI_NUMERICHOST)
        with self.assertRaises(socket.gaierror):
            socket.getaddrinfo("127.0.0.1", 0, flags=socket.AI_CANONNAME)

    def test_connection_refused(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            address = listener.getsockname()
        with socket.socket() as client:
            client.settimeout(0.2)
            self.assertNotEqual(client.connect_ex(address), 0)


def check_denied():
    # A host can allocate an unconnected descriptor before enforcing network
    # permissions. Binding must never appear to succeed without the capability.
    try:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
    except OSError as error:
        print("network capability denied:", type(error).__name__, error.errno)
    else:
        raise AssertionError("network operation succeeded without permission")


def check_unsupported():
    for args in [(), (socket.AF_INET, socket.SOCK_STREAM)]:
        try:
            socket.socket(*args)
        except OSError as error:
            assert error.errno == errno.ENOTSUP, error
        else:
            raise AssertionError("socket construction fabricated a descriptor")
    print("preview1 without WASIX: unsupported socket creation is explicit")


if __name__ == "__main__":
    if sys.argv[1:] == ["--denied"]:
        check_denied()
    elif sys.argv[1:] == ["--unsupported"]:
        check_unsupported()
    else:
        unittest.main()
