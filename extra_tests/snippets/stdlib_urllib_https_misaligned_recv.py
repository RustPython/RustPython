import os
import socket
import ssl
import sys
import threading
import time
import urllib.request

ROOT_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CERTFILE = os.path.join(ROOT_DIR, "Lib/test/certdata/keycert.pem")
BODY = b"x" * 407_676

# TLS record body sizes observed from https://crates.io/api/v1/crates/tokio.
TLS_RECORD_BODY_SIZES = [
    2855,
    281,
    53,
    218,
    1095,
    1395,
    1395,
    483,
    1395,
    1395,
    1395,
    1395,
    48,
    1360,
    1354,
    1395,
    1395,
    1395,
    1367,
    1395,
    1395,
    1395,
    1395,
    1326,
    1395,
    1395,
    1395,
    47,
    1395,
    1395,
    1395,
    1395,
    95,
    1395,
    1332,
    1287,
    1388,
    1395,
    1395,
    1374,
    1395,
    1380,
    794,
    791,
    1395,
    1381,
    1395,
    1395,
    1395,
    1333,
    1395,
    1395,
    1395,
    1395,
    1395,
    1395,
    965,
    16401,
    3914,
    2526,
    1041,
    8209,
    9233,
    16401,
    11650,
    10262,
    7486,
    3468,
    692,
    1041,
    16401,
    12242,
    9466,
    1041,
    8209,
    9233,
    8209,
    9233,
    16401,
    1041,
    8209,
    9233,
    6161,
    2065,
    9233,
    16401,
    16358,
    10806,
    1041,
    8209,
    16401,
    3914,
    16401,
    16401,
    3089,
    9233,
    4642,
    478,
    8209,
    3140,
    1752,
    9233,
    8209,
    8209,
    16401,
    16064,
    14676,
    13288,
    2065,
    16401,
    1041,
    8209,
    16401,
    1041,
    6374,
    1007,
]

server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
server_context.load_cert_chain(CERTFILE)
listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(1)
addr, port = listener.getsockname()
server_errors = []
finished = False


def guard_timeout():
    time.sleep(20)
    if not finished:
        print(
            "stdlib_urllib_https_misaligned_recv.py timed out",
            file=sys.stderr,
            flush=True,
        )
        os.abort()


threading.Thread(target=guard_timeout, daemon=True).start()


def drain_outgoing(outgoing, conn):
    while True:
        try:
            data = outgoing.read()
        except ssl.SSLWantReadError:
            return
        if not data:
            return
        conn.sendall(data)


def run_server():
    try:
        conn, _ = listener.accept()
        conn.settimeout(5.0)
        conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

        incoming = ssl.MemoryBIO()
        outgoing = ssl.MemoryBIO()
        tls = server_context.wrap_bio(incoming, outgoing, server_side=True)

        while True:
            try:
                tls.do_handshake()
                break
            except ssl.SSLWantReadError:
                drain_outgoing(outgoing, conn)
                incoming.write(conn.recv(65536))
            except ssl.SSLWantWriteError:
                pass
            drain_outgoing(outgoing, conn)

        request = b""
        while b"\r\n\r\n" not in request:
            try:
                request += tls.read(65536)
            except ssl.SSLWantReadError:
                drain_outgoing(outgoing, conn)
                incoming.write(conn.recv(65536))
            drain_outgoing(outgoing, conn)

        response = (
            b"HTTP/1.1 200 OK\r\n"
            b"Connection: close\r\n"
            + b"Content-Length: "
            + str(len(BODY)).encode()
            + b"\r\n"
            + b"Content-Type: application/json\r\n"
            + b"\r\n"
            + BODY
        )
        plaintext_sizes = [max(1, n - 17) for n in TLS_RECORD_BODY_SIZES]
        pos = 0
        while pos < len(response):
            size = plaintext_sizes.pop(0) if plaintext_sizes else 16384
            end = min(len(response), pos + size)
            while pos < end:
                try:
                    pos += tls.write(response[pos:end])
                except ssl.SSLWantWriteError:
                    pass
                drain_outgoing(outgoing, conn)
        conn.close()
    except BaseException as exc:
        server_errors.append(exc)
    finally:
        listener.close()


thread = threading.Thread(target=run_server)
thread.start()

client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
client_context.check_hostname = False
client_context.verify_mode = ssl.CERT_NONE
opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({}),
    urllib.request.HTTPSHandler(context=client_context),
)
try:
    with opener.open(f"https://{addr}:{port}/", timeout=5.0) as response:
        body = response.read()

    thread.join(10.0)
    assert not thread.is_alive(), "server thread did not stop"
    assert not server_errors, server_errors
    assert body == BODY
finally:
    finished = True
