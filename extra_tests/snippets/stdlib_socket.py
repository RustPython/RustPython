import _socket
import os
import socket

from testutils import assert_raises

assert _socket.socket == _socket.SocketType

MESSAGE_A = b"aaaa"
MESSAGE_B = b"bbbbb"

# TCP

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.bind(("127.0.0.1", 0))
listener.listen(1)

connector = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
connector.connect(("127.0.0.1", listener.getsockname()[1]))
(connection, addr) = listener.accept()
assert addr == connector.getsockname()

connector.send(MESSAGE_A)
connection.send(MESSAGE_B)
recv_a = connection.recv(len(MESSAGE_A))
recv_b = connector.recv(len(MESSAGE_B))
assert recv_a == MESSAGE_A
assert recv_b == MESSAGE_B

fd = open("README.md", "rb")
connector.sendfile(fd)
recv_readme = connection.recv(os.stat("README.md").st_size)
# need this because sendfile leaves the cursor at the end of the file
fd.seek(0)
assert recv_readme == fd.read()
fd.close()

# fileno
if os.name == "posix":
    connector_fd = connector.fileno()
    connection_fd = connection.fileno()
    os.write(connector_fd, MESSAGE_A)
    connection.send(MESSAGE_B)
    recv_a = connection.recv(len(MESSAGE_A))
    recv_b = os.read(connector_fd, (len(MESSAGE_B)))
    assert recv_a == MESSAGE_A
    assert recv_b == MESSAGE_B

connection.close()
connector.close()
listener.close()

s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
with assert_raises(TypeError):
    s.connect(("127.0.0.1", 8888, 8888))

with assert_raises(OSError):
    # Lets hope nobody is listening on port 1
    s.connect(("127.0.0.1", 1))

with assert_raises(TypeError):
    s.bind(("127.0.0.1", 8888, 8888))

with assert_raises(OSError):
    # Lets hope nobody run this test on machine with ip 1.2.3.4
    s.bind(("1.2.3.4", 8888))

with assert_raises(TypeError):
    s.bind((888, 8888))

s.close()
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind(("127.0.0.1", 0))
with assert_raises(OSError):
    s.recv(100)

with assert_raises(OSError):
    s.send(MESSAGE_A)

s.close()

# UDP
sock1 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock1.bind(("127.0.0.1", 0))

sock2 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

sock2.sendto(MESSAGE_A, sock1.getsockname())
(recv_a, addr1) = sock1.recvfrom(len(MESSAGE_A))
assert recv_a == MESSAGE_A

sock2.sendto(MESSAGE_B, sock1.getsockname())
(recv_b, addr2) = sock1.recvfrom(len(MESSAGE_B))
assert recv_b == MESSAGE_B
assert addr1[0] == addr2[0]
assert addr1[1] == addr2[1]

sock2.close()

sock3 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock3.bind(("127.0.0.1", 0))
sock3.sendto(MESSAGE_A, sock1.getsockname())
(recv_a, addr) = sock1.recvfrom(len(MESSAGE_A))
assert recv_a == MESSAGE_A
assert addr == sock3.getsockname()

sock1.connect(("127.0.0.1", sock3.getsockname()[1]))
sock3.connect(("127.0.0.1", sock1.getsockname()[1]))

sock1.send(MESSAGE_A)
sock3.send(MESSAGE_B)
recv_a = sock3.recv(len(MESSAGE_A))
recv_b = sock1.recv(len(MESSAGE_B))
assert recv_a == MESSAGE_A
assert recv_b == MESSAGE_B
sock1.close()
sock3.close()

s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
with assert_raises(OSError):
    s.bind(("1.2.3.4", 888))

s.close()
### Errors
with assert_raises(OSError):
    socket.socket(100, socket.SOCK_STREAM)

with assert_raises(OSError):
    socket.socket(socket.AF_INET, 1000)

with assert_raises(OSError):
    socket.inet_aton("test")

# TODO: RUSTPYTHON
# with assert_raises(ValueError):
#     socket.htonl(-1)

assert socket.htonl(0) == 0
assert socket.htonl(10) == 167772160

assert socket.inet_aton("127.0.0.1") == b"\x7f\x00\x00\x01"
assert socket.inet_aton("255.255.255.255") == b"\xff\xff\xff\xff"


assert socket.inet_ntoa(b"\x7f\x00\x00\x01") == "127.0.0.1"
assert socket.inet_ntoa(b"\xff\xff\xff\xff") == "255.255.255.255"

with assert_raises(OSError):
    socket.inet_ntoa(b"\xff\xff\xff\xff\xff")

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
    pass

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    connector = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    connector.connect(("127.0.0.1", listener.getsockname()[1]))
    (connection, addr) = listener.accept()
    connection.settimeout(1.0)
    with assert_raises(OSError):  # TODO: check that it raises a socket.timeout
        # testing that it doesn't work with the timeout; that it stops blocking eventually
        connection.recv(len(MESSAGE_A))

for exc, expected_name in [
    (socket.gaierror, "gaierror"),
    (socket.herror, "herror"),
]:
    assert exc.__module__ == "socket"
    assert exc.__name__ == expected_name

# assert socket.timeout.__module__ == "builtins"
# assert socket.timeout.__name__ == "TimeoutError"


# recv() sizes its buffer from the argument, so an unreachable size has to be
# reported rather than reserved.
sizes = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
with sizes:
    for bufsize in (2**62, 2**48):
        try:
            sizes.recv(bufsize)
        except (MemoryError, OSError, OverflowError):
            # A size that does not fit the platform's C int is reported while
            # converting the argument, before there is anything to reserve.
            pass
    with assert_raises(ValueError):
        sizes.recvfrom(-1)


import sys


def receive_sizes_and_truncation():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as receiver:
        receiver.bind(("127.0.0.1", 0))
        receiver.settimeout(2)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sender:
            for method in (receiver.recv_into, receiver.recvfrom_into):
                for size in (-1, 5):
                    try:
                        method(bytearray(4), size)
                    except ValueError:
                        pass
                    else:
                        raise AssertionError("invalid receive size accepted")
                sender.sendto(b"abc", receiver.getsockname())
                destination = bytearray(b"------")
                result = method(memoryview(destination)[1:5], 0)
                assert (result[0] if isinstance(result, tuple) else result) == 3
                assert destination == b"-abc--"

            if sys.platform.startswith("linux"):
                for method in (receiver.recv_into, receiver.recvfrom_into):
                    sender.sendto(b"abcdefgh", receiver.getsockname())
                    for flags in (socket.MSG_PEEK | socket.MSG_TRUNC, socket.MSG_TRUNC):
                        destination = bytearray(b"------")
                        result = method(memoryview(destination)[1:5], 3, flags)
                        assert (result[0] if isinstance(result, tuple) else result) == 8
                        assert destination == b"-abc--"
                for method in (receiver.recv, receiver.recvfrom, receiver.recvmsg):
                    sender.sendto(b"abcdefgh", receiver.getsockname())
                    result = (
                        method(3, 0, socket.MSG_TRUNC)
                        if method == receiver.recvmsg
                        else method(3, socket.MSG_TRUNC)
                    )
                    data = result[0] if isinstance(result, tuple) else result
                    assert data[:3] == b"abc"
                    if sys.implementation.name == "rustpython":
                        # Some CPython recv paths enlarge the bytes result to the
                        # reported datagram length, including an unwritten tail.
                        assert data == b"abc"


def tcp_truncation_preserves_buffer(protocol=0):
    import signal

    try:
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM, protocol)
    except OSError:
        if protocol:
            return  # MPTCP is optional in the running kernel.
        raise
    with listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        with socket.create_connection(listener.getsockname()) as sender:
            receiver, _ = listener.accept()
            with receiver:
                receiver.settimeout(2)
                for method in (receiver.recv_into, receiver.recvfrom_into):
                    destination = bytearray(b"original")

                    def send_after_mutation(signum, frame):
                        destination[0] = ord("X")
                        sender.sendall(b"abc")

                    previous = signal.signal(signal.SIGALRM, send_after_mutation)
                    try:
                        signal.setitimer(signal.ITIMER_REAL, 0.05)
                        result = method(destination, 0, socket.MSG_TRUNC)
                        assert (result[0] if isinstance(result, tuple) else result) == 3
                        assert destination == b"Xriginal"
                    finally:
                        signal.setitimer(signal.ITIMER_REAL, 0)
                        signal.signal(signal.SIGALRM, previous)


receive_sizes_and_truncation()
if sys.platform.startswith("linux"):
    tcp_truncation_preserves_buffer()
    if hasattr(socket, "IPPROTO_MPTCP"):
        tcp_truncation_preserves_buffer(socket.IPPROTO_MPTCP)
