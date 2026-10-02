import _imp
import builtins
import sys
import types

from testutils import assert_raises

if sys.version_info < (3, 15) and sys.implementation.name != "rustpython":
    sys.exit(0)


class Spec:
    def __init__(self, name):
        self.name = name


assert _imp.create_builtin(Spec("sys")) is sys
assert _imp.create_builtin(Spec("builtins")) is builtins

for name in (None, 42, b"sys"):
    with assert_raises(TypeError) as caught:
        _imp.create_builtin(Spec(name))
    assert str(caught.exception) == f"name must be string, not {type(name).__name__}"

with assert_raises(ValueError) as caught:
    _imp.create_builtin(Spec(""))
assert str(caught.exception) == "name must not be empty"

for name in ("_absent_builtin_for_probe", "os", "a.sys"):
    with assert_raises(ModuleNotFoundError) as caught:
        _imp.create_builtin(Spec(name))
    assert caught.exception.name is name
    assert str(caught.exception) == f"{name} module not found"

name = "_cached_nonbuiltin_for_probe"
sys.modules[name] = types.ModuleType(name)
try:
    with assert_raises(ModuleNotFoundError) as caught:
        _imp.create_builtin(Spec(name))
    assert caught.exception.name is name
finally:
    del sys.modules[name]

with assert_raises(ValueError) as caught:
    _imp.create_builtin(Spec("a\0b"))
assert str(caught.exception) == "embedded null character"

for name in ("é", "\ud800"):
    with assert_raises(UnicodeEncodeError) as caught:
        _imp.create_builtin(Spec(name))
    assert caught.exception.encoding == "ascii"
    assert caught.exception.object == name
    assert (caught.exception.start, caught.exception.end) == (0, 1)

with assert_raises(AttributeError):
    _imp.create_builtin(object())
