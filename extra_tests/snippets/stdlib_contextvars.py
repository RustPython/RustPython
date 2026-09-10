from contextvars import ContextVar

from testutils import assert_raises

name = "\ud800"
var = ContextVar(name)
assert var.name == name
assert var.name is name


class StrSub(str):
    pass


sub = StrSub("foo")
var_sub = ContextVar(sub)
assert var_sub.name is sub

assert_raises(TypeError, ContextVar, 1)
