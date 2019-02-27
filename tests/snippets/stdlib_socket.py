import socket
from testutils import assertRaises


listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.bind(("127.0.0.1", 0))
listener.listen(1)

connector = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
connector.connect(("127.0.0.1", listener.getsockname()[1]))
connection = listener.accept()[0]

message_a = b'aaaa'
message_b = b'bbbbb'

connector.send(message_a)
connection.send(message_b)
recv_a = connection.recv(len(message_a))
recv_b = connector.recv(len(message_b))
assert recv_a == message_a
assert recv_b == message_b

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
