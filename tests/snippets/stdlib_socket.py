import socket
from testutils import assertRaises

MESSAGE_A = b'aaaa'
MESSAGE_B= b'bbbbb'

# TCP

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.bind(("127.0.0.1", 0))
listener.listen(1)

connector = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
connector.connect(("127.0.0.1", listener.getsockname()[1]))
connection = listener.accept()[0]

connector.send(MESSAGE_A)
connection.send(MESSAGE_B)
recv_a = connection.recv(len(MESSAGE_A))
recv_b = connector.recv(len(MESSAGE_B))
assert recv_a == MESSAGE_A
assert recv_b == MESSAGE_B
connection.close()
connector.close()
listener.close()

s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
with assertRaises(TypeError):
	s.connect(("127.0.0.1", 8888, 8888))

with assertRaises(TypeError):
	s.bind(("127.0.0.1", 8888, 8888))

with assertRaises(TypeError):
	s.bind((888, 8888))

s.close()

# UDP
sock1 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock1.bind(("127.0.0.1", 0))

sock2 = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

sock2.sendto(MESSAGE_A, sock1.getsockname())
(recv_a, addr) = sock1.recvfrom(len(MESSAGE_A))
assert recv_a == MESSAGE_A

sock2.bind(("127.0.0.1", 0))
sock1.connect(("127.0.0.1", sock2.getsockname()[1]))
sock2.connect(("127.0.0.1", sock1.getsockname()[1]))

sock1.send(MESSAGE_A)
sock2.send(MESSAGE_B)
recv_a = sock2.recv(len(MESSAGE_A))
recv_b = sock1.recv(len(MESSAGE_B))
assert recv_a == MESSAGE_A
assert recv_b == MESSAGE_B
sock1.close()
sock2.close()
