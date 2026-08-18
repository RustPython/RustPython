import _imp
import time as import_time

from testutils import assert_raises

assert _imp.is_builtin("time") == True
assert _imp.is_builtin("os") == False
assert _imp.is_builtin("not existing module") == False

assert _imp.is_frozen("__hello__") == True
assert _imp.is_frozen("math") == False


class FakeSpec:
    def __init__(self, name):
        self.name = name


A = FakeSpec("time")

imp_time = _imp.create_builtin(A)
# FIXME: cpython3.9 fail
# assert imp_time.sleep == import_time.sleep

B = FakeSpec("not existing module")
assert _imp.create_builtin(B) == None

_imp.exec_builtin(imp_time) == 0

_imp.get_frozen_object("__hello__")

hello = _imp.init_frozen("__hello__")
assert hello.initialized == True

# withdata is keyword-only
with assert_raises(TypeError):
    _imp.find_frozen("x", True)
assert _imp.find_frozen("_this_module_does_not_exist_") is None

# and it hands back the marshalled code that get_frozen_object() takes
data, ispkg, origname = _imp.find_frozen("__hello__", withdata=True)
assert ispkg is False
assert origname == "__hello__"
assert _imp.get_frozen_object("__hello__", data).co_name == "<module>"
