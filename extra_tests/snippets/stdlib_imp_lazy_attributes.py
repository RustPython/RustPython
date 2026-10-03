import _imp

from testutils import assert_raises, skip_if_unsupported


def test_lazy_attributes_name_hash():
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


skip_if_unsupported(3, 15, test_lazy_attributes_name_hash)
