
class A:
	def test():
		pass

a = A()

assert "test" in dir(a)

import socket

assert "AF_INET" in dir(socket)
