assert isinstance(dir(), list)
assert '__builtins__' in dir()

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

# eval() takes any mapping-like type, so dir() must support them
# TODO: eval() should take any mapping as locals, not just dict-derived types
class A(dict):
	def __getitem__(self, x):
		return dir
	def keys(self):
		yield 6
		yield 5
assert eval("dir()", {}, A()) == [5, 6]

import socket

assert "AF_INET" in dir(socket)
