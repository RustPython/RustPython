import socket
import os
from testutils import assertRaises

MESSAGE_A = b'aaaa'
MESSAGE_B= b'bbbbb'

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
with assertRaises(TypeError):
	s.connect(("127.0.0.1", 8888, 8888))

with assertRaises(OSError):
	# Lets hope nobody is listening on port 1
	s.connect(("127.0.0.1", 1))

with assertRaises(TypeError):
	s.bind(("127.0.0.1", 8888, 8888))

with assertRaises(OSError):
	# Lets hope nobody run this test on machine with ip 1.2.3.4
	s.bind(("1.2.3.4", 8888))

with assertRaises(TypeError):
	s.bind((888, 8888))

s.close()
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.bind(("127.0.0.1", 0))
with assertRaises(OSError):
	s.recv(100)

with assertRaises(OSError):
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
with assertRaises(OSError):
	s.bind(("1.2.3.4", 888))

s.close()
### Errors
with assertRaises(OSError):
	socket.socket(100, socket.SOCK_STREAM)

with assertRaises(OSError):
	socket.socket(socket.AF_INET, 1000)
