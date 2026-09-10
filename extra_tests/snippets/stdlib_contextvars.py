from contextvars import Context, ContextVar, copy_context

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

ctx1 = Context()
ctx2 = Context()
nested = ContextVar("var")


def func2():
    assert nested.get(None) is None


def func1():
    assert nested.get(None) is None
    nested.set("spam")
    ctx2.run(func2)
    assert nested.get(None) == "spam"
    cur = copy_context()
    assert len(cur) == 1
    assert cur[nested] == "spam"
    return cur


returned_ctx = ctx1.run(func1)
assert ctx1 == returned_ctx
assert returned_ctx[nested] == "spam"
assert nested in returned_ctx
assert_raises(TypeError, hash, ctx1)


class ReentrantEq:
    def __eq__(self, other):
        ctx1.run(lambda: nested.set(object()))
        return True


ctx1.run(nested.set, ReentrantEq())
ctx2.run(nested.set, object())
assert ctx1 == ctx2
