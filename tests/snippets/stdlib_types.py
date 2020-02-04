import types

from testutils import assert_raises

ns = types.SimpleNamespace(a=2, b='Rust')

assert ns.a == 2
assert ns.b == "Rust"
with assert_raises(AttributeError):
    _ = ns.c
