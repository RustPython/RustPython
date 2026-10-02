import _imp

from testutils import assert_raises, skip_if_unsupported


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
