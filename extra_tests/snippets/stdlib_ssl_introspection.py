"""Rustls must report unavailable negotiated signatures explicitly."""

import os
import pathlib
import ssl
import sys

CERT = pathlib.Path(
    os.environ.get(
        "RP_SSL_TEST_CERT",
        pathlib.Path(__file__).resolve().parents[2] / "Lib/test/certdata/keycert.pem",
    )
)


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
        for query in (endpoint.client_sigalg, endpoint.server_sigalg):
            try:
                query()
            except NotImplementedError as error:
                assert "Rustls does not expose" in str(error)
            else:
                raise AssertionError("Selected signatures must not be fabricated")


if sys.implementation.name == "rustpython":
    handshake(ssl.TLSVersion.TLSv1_2)
    handshake(ssl.TLSVersion.TLSv1_3)
