import types

from testutils import assertRaises

ns = types.SimpleNamespace(a=2, b='Rust')

assert ns.a == 2
assert ns.b == "Rust"
with assertRaises(AttributeError):
    _ = ns.c
