import _imp
import time as import_time

assert _imp.is_builtin("time") == True
assert _imp.is_builtin("os") == False
assert _imp.is_builtin("not existing module") == False

assert _imp.is_frozen("__hello__") == True
assert _imp.is_frozen("os") == False

class FakeSpec:
	def __init__(self, name):
		self.name = name

A = FakeSpec("time")

imp_time = _imp.create_builtin(A)
assert imp_time.sleep == import_time.sleep

B = FakeSpec("not existing module")
assert _imp.create_builtin(B) == None

_imp.exec_builtin(imp_time) == 0

_imp.get_frozen_object("__hello__")

hello = _imp.init_frozen("__hello__")
assert hello.initialized == True
