import socket

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.bind(("127.0.0.1", 8080))
listener.listen(1)

connector = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
connector.connect(("127.0.0.1", 8080))
connection = listener.accept()[0]

message_a = b'aaaa'
message_b = b'bbbbb'

connector.send(message_a)
connector.close()
recv_a = connection.recv(10)

connection.close()
listener.close()

