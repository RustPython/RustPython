import io
import os
import pathlib
import socket
import ssl
import sys
import threading

CERT = pathlib.Path(
    os.environ.get(
        "RP_SSL_TEST_CERT",
        pathlib.Path(__file__).resolve().parents[2] / "Lib/test/certdata/keycert.pem",
    )
)
IS_RUSTPYTHON = sys.implementation.name == "rustpython"

sigalgs = ssl.get_sigalgs()
assert isinstance(sigalgs, list)
assert sigalgs and all(isinstance(name, str) for name in sigalgs)
assert "rsa_pss_rsae_sha256" in sigalgs
assert "ecdsa_secp256r1_sha256" in sigalgs
assert "rsa_pss_sha256" not in sigalgs
assert "ecdsa_nistp256_sha256" not in sigalgs
original_sigalgs = sigalgs.copy()
sigalgs.clear()
assert ssl.get_sigalgs() == original_sigalgs
assert isinstance(ssl.HAS_PSK_TLS13, bool)
if IS_RUSTPYTHON:
    assert not ssl.HAS_PSK
    assert not ssl.HAS_PSK_TLS13


def handshake(version):
    client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    client_context.check_hostname = False
    client_context.verify_mode = ssl.CERT_NONE
    server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_context.load_cert_chain(str(CERT))
    for context in (client_context, server_context):
        context.minimum_version = context.maximum_version = version
        context.set_ecdh_curve("secp384r1")

    client_in, client_out = ssl.MemoryBIO(), ssl.MemoryBIO()
    server_in, server_out = ssl.MemoryBIO(), ssl.MemoryBIO()
    client = client_context.wrap_bio(client_in, client_out)
    server = server_context.wrap_bio(server_in, server_out, server_side=True)
    if IS_RUSTPYTHON:
        # OpenSSL 3.5.8 can crash when queried before negotiation.
        assert client.group() is None
        assert server.group() is None

    completed = [False, False]
    for _ in range(20):
        for index, endpoint, outgoing, incoming in (
            (0, client, client_out, server_in),
            (1, server, server_out, client_in),
        ):
            try:
                endpoint.do_handshake()
            except ssl.SSLWantReadError:
                pass
            else:
                completed[index] = True
            data = outgoing.read()
            if data:
                incoming.write(data)
        if all(completed):
            break
    assert all(completed), "MemoryBIO handshake did not complete"
    for endpoint in (client, server):
        assert endpoint.group() == "secp384r1"
        assert endpoint._sslobj.uses_ktls_for_send() is False
        assert endpoint._sslobj.uses_ktls_for_recv() is False
        if IS_RUSTPYTHON:
            for query in (endpoint.client_sigalg, endpoint.server_sigalg):
                try:
                    query()
                except NotImplementedError as error:
                    assert "Rustls does not expose" in str(error)
                else:
                    raise AssertionError("Selected signatures must not be fabricated")
        else:
            assert endpoint.client_sigalg() is None
            assert endpoint.server_sigalg() in original_sigalgs

    payload = b"TLS introspection preserves application data"
    assert client.write(payload) == len(payload)
    server_in.write(client_out.read())
    assert server.read(len(payload)) == payload


handshake(ssl.TLSVersion.TLSv1_2)
handshake(ssl.TLSVersion.TLSv1_3)


def sendfile_fallback():
    payload = b"encrypted sendfile fallback" * 32
    client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    client_context.check_hostname = False
    client_context.verify_mode = ssl.CERT_NONE
    server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    server_context.load_cert_chain(str(CERT))
    left, right = socket.socketpair()
    left.settimeout(5)
    right.settimeout(5)
    received = bytearray()
    errors = []

    def receive():
        try:
            with server_context.wrap_socket(right, server_side=True) as server:
                while len(received) < len(payload):
                    chunk = server.recv(len(payload) - len(received))
                    if not chunk:
                        break
                    received.extend(chunk)
        except Exception as error:
            errors.append(error)

    worker = threading.Thread(target=receive, daemon=True)
    worker.start()
    try:
        with client_context.wrap_socket(left) as client:
            assert client._sslobj.uses_ktls_for_send() is False
            file = io.BytesIO(b"pre" + payload + b"post")
            assert client.sendfile(file, offset=3, count=len(payload)) == len(payload)
            assert file.tell() == 3 + len(payload)
    finally:
        left.close()
        worker.join(6)
        right.close()
    assert not worker.is_alive(), "TLS sendfile receiver did not complete"
    assert not errors, errors
    assert received == payload


sendfile_fallback()
