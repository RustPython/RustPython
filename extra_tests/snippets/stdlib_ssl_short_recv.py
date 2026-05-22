import os
import socket
import ssl
import sys
import threading

if sys.implementation.name.lower() != "rustpython":
    print("Ignored: stdlib_ssl_short_recv (RustPython only)")
    raise SystemExit

ROOT_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CERTFILE = os.path.join(ROOT_DIR, "Lib/test/certdata/keycert.pem")
DATA = b"x" * 128

orig_recv = socket.socket.recv
client_sockname = None
recv_n = {}


def new_recv(sock, bufsize, flags=0):
    sockname = sock.getsockname()
    if sockname not in recv_n:
        recv_n[sockname] = 0

    bufsize = 1

    if flags & socket.MSG_PEEK == 0:
        recv_n[sockname] += 1
    return orig_recv(sock, bufsize, flags)


socket.socket.recv = new_recv

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(1)
addr, port = listener.getsockname()
server_errors = []


def server():
    try:
        server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        server_context.load_cert_chain(CERTFILE)

        sock, _ = listener.accept()
        sock.settimeout(5.0)

        ssock = server_context.wrap_socket(sock, server_side=True)
        try:
            ssock.sendall(DATA)
        finally:
            ssock.close()
    except BaseException as exc:
        server_errors.append(exc)
    finally:
        listener.close()


thread = threading.Thread(target=server)
thread.start()

raw = socket.create_connection((addr, port), timeout=5.0)
client_sockname = raw.getsockname()
raw.settimeout(5.0)

client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
client_context.check_hostname = False
client_context.verify_mode = ssl.CERT_NONE

client = client_context.wrap_socket(raw, server_hostname=None)
try:
    chunks = []
    while sum(len(chunk) for chunk in chunks) < len(DATA):
        chunk = client.recv(20000)
        if not chunk:
            break
        chunks.append(chunk)
finally:
    client.close()

thread.join(10.0)
assert not thread.is_alive(), "server thread did not stop"
assert not server_errors, server_errors
assert b"".join(chunks) == DATA
assert len(recv_n) == 2
assert all(n > 100 for n in recv_n.values())
