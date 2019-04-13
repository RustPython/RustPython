
class A:
	def test():
		pass

a = A()

assert "test" in dir(a), "test not in a"
assert "test" in dir(A), "test not in A"

a.x = 3
assert "x" in dir(a), "x not in a"

class B(A):
	def __dir__(self):
		return ('q', 'h')

# Gets sorted and turned into a list
assert ['h', 'q'] == dir(B())

# This calls type.__dir__ so isn't changed (but inheritance works)!
assert 'test' in dir(A)

import socket

assert "AF_INET" in dir(socket)
