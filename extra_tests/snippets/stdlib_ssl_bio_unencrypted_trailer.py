import pathlib
import ssl

CERT = (
    pathlib.Path(__file__).resolve().parent.parent.parent
    / "Lib"
    / "test"
    / "certdata"
    / "keycert.pem"
)

TAIL = b"tail"

server_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
server_context.load_cert_chain(CERT)
client_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
client_context.check_hostname = False
client_context.verify_mode = ssl.CERT_NONE
server_context.maximum_version = client_context.maximum_version = ssl.TLSVersion.TLSv1_2

client_in, client_out = ssl.MemoryBIO(), ssl.MemoryBIO()
server_in, server_out = ssl.MemoryBIO(), ssl.MemoryBIO()
client = client_context.wrap_bio(client_in, client_out)
server = server_context.wrap_bio(server_in, server_out, server_side=True)

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

try:
    server.unwrap()
except ssl.SSLWantReadError:
    pass
client_in.write(server_out.read() + TAIL)
client.unwrap()
assert client_in.read() == TAIL
