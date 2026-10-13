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

try:
    import gc

    import _interpreters
except ImportError:
    _interpreters = None

if _interpreters is not None:
    child = _interpreters.create()
    try:
        assert (
            _interpreters.run_string(
                child,
                """
import contextvars
contextvars.ContextVar('child_marker').set(42)
""",
            )
            is None
        )
    finally:
        _interpreters.destroy(child)

    gc.collect()
    assert [
        (var.name, value)
        for var, value in copy_context().items()
        if var.name == "child_marker"
    ] == []


def context_cache_does_not_outlive_its_context():
    import contextvars

    for factory_name in ("new", "copy"):
        for _ in range(100):
            var = contextvars.ContextVar("var", default="default")
            empty = contextvars.Context()
            ctx = contextvars.Context() if factory_name == "new" else empty.copy()
            ctx.run(var.set, "cached")
            del ctx
            ctx = contextvars.Context() if factory_name == "new" else empty.copy()
            assert list(ctx.items()) == []
            assert ctx.run(var.get) == "default"
            assert ctx.run(var.get, "explicit default") == "explicit default"

            missing = contextvars.ContextVar("missing")
            ctx.run(missing.set, "cached")
            del ctx
            ctx = contextvars.Context() if factory_name == "new" else empty.copy()
            try:
                ctx.run(missing.get)
            except LookupError:
                pass
            else:
                raise AssertionError(
                    "an empty context returned a destroyed context value"
                )


context_cache_does_not_outlive_its_context()
