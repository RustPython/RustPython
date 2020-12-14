import socket
import os
from testutils import assert_raises

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

fd = open('README.md', 'rb')
connector.sendfile(fd)
recv_readme = connection.recv(os.stat('README.md').st_size)
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

with assert_raises(OverflowError):
	socket.htonl(-1)

assert socket.htonl(0)==0
assert socket.htonl(10)==167772160

assert socket.inet_aton("127.0.0.1")==b"\x7f\x00\x00\x01"
assert socket.inet_aton("255.255.255.255")==b"\xff\xff\xff\xff"


assert socket.inet_ntoa(b"\x7f\x00\x00\x01")=="127.0.0.1"
assert socket.inet_ntoa(b"\xff\xff\xff\xff")=="255.255.255.255"

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
	with assert_raises(OSError): # TODO: check that it raises a socket.timeout
		# testing that it doesn't work with the timeout; that it stops blocking eventually
		connection.recv(len(MESSAGE_A))
