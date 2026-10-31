import _imp
import time as import_time

from testutils import assert_raises, skip_if_unsupported

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
with assert_raises(ModuleNotFoundError):
    _imp.create_builtin(B)

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


def test_lazy_attributes_eager_registry():
    # CPython accepts any first argument and a string name. In an eager-only
    # interpreter there are no pending lazy module names to remove.
    for modobj in (None, object(), {}, 42):
        assert _imp._set_lazy_attributes(modobj, "package.module") is None
        assert _imp._set_lazy_attributes(modobj, "\ud800") is None

    for name in (None, 1, b"package.module", [], {}):
        with assert_raises(TypeError):
            _imp._set_lazy_attributes(None, name)

    with assert_raises(TypeError):
        _imp._set_lazy_attributes(modobj=None, name="package.module")

    class ModuleName(str):
        hashes = 0

        def __hash__(self):
            self.hashes += 1
            return 42

    name = ModuleName("package.module")
    assert _imp._set_lazy_attributes(None, name) is None
    assert name.hashes == 1

    class UnhashableName(str):
        __hash__ = None

    with assert_raises(TypeError):
        _imp._set_lazy_attributes(None, UnhashableName("package.module"))

    class HashErrorName(str):
        def __hash__(self):
            raise RuntimeError("name hash failed")

    with assert_raises(RuntimeError):
        _imp._set_lazy_attributes(None, HashErrorName("package.module"))


skip_if_unsupported(3, 15, test_lazy_attributes_eager_registry)
