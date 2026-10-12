from collections.abc import Awaitable, Callable
from typing import TypeVar

import _typing
from testutils import assert_raises

T = TypeVar("T")


def abort_signal_handler(
    fn: Callable[[], Awaitable[T]], on_abort: Callable[[], None] | None = None
) -> T:
    pass


# Ensure PEP 604 unions work with typing.Callable aliases.
TracebackFilter = bool | Callable[[int], int]


# Test that Union/Optional in function parameter annotations work correctly.
# This tests that annotation scopes can access global implicit symbols (like Union)
# that are imported at module level but not explicitly bound in the function scope.
# Regression test for: rich
from typing import Optional, Union


def function_with_union_param(x: Optional[Union[int, str]] = None) -> None:
    pass


class ClassWithUnionParams:
    def __init__(
        self,
        color: Optional[Union[str, int]] = None,
        bold: Optional[bool] = None,
    ) -> None:
        pass

    def method(self, value: Union[int, float]) -> Union[str, bytes]:
        return str(value)


# _idfunc takes exactly one argument, checked before the argument is read.

assert _typing._idfunc(1) == 1
with assert_raises(TypeError):
    _typing._idfunc()


# ParamSpecArgs shows a non-ParamSpec origin by its repr, which is where the
# recursion guard lives; nesting them deeply must not walk the native stack.

from typing import ParamSpec, ParamSpecArgs

spec = ParamSpec("spec")
assert repr(spec.args) == "spec.args"
assert repr(spec.kwargs) == "spec.kwargs"

nested = object()
for _ in range(2000):
    nested = ParamSpecArgs(nested)
try:
    repr(nested)
except RecursionError:
    pass


# Python 3.14.8 retains ParamSpec bounds without converting or validating them.
assert ParamSpec("Unbounded").__bound__ is None
for bound in (None, "int", "not valid Python!", (int, str), object()):
    assert ParamSpec("BoundSpec", bound=bound).__bound__ is bound

# Eager aliases without a caller module never inspect their aliased value.
alias_module_lookups = []


class AliasModuleValue:
    @property
    def __module__(self):
        alias_module_lookups.append(True)
        return "unexpected_value_module"


for alias_value in (int, AliasModuleValue()):
    alias_namespace = {"value": alias_value}
    exec(
        "from typing import TypeAliasType; alias = TypeAliasType('Alias', value)",
        alias_namespace,
    )
    assert alias_namespace["alias"].__module__ is None
assert alias_module_lookups == []


# GenericAlias rejects parameters added after its parameter tuple was cached.
from types import GenericAlias


class LateParameter:
    def __repr__(self):
        return "late_parameter"


first = LateParameter()
first.__typing_subst__ = lambda value: value
late = LateParameter()
alias = GenericAlias(dict, (first, late))
assert alias.__parameters__ == (first,)
late.__typing_subst__ = lambda value: value
with assert_raises(TypeError) as caught:
    alias[0]
assert str(caught.exception) == (
    "argument late_parameter with __typing_subst__ was not found in __parameters__"
)


def test_union_lazy_parameters():
    import threading
    from types import GenericAlias
    from typing import Union

    from testutils import assert_raises

    class Parameter:
        def __typing_subst__(self, value):
            return value

    parameter = Parameter()

    class LateParameter:
        __parameters__ = (parameter,)

        def __repr__(self):
            return "late_parameter"

    # Building a union must not freeze the nested alias's parameter cache.
    for materialize in (False, True):
        for subscribe_first in (False, True):
            late = LateParameter()
            alias = GenericAlias(list, late)
            union = alias | int
            if materialize:
                cached = union.__parameters__
                assert cached == (parameter,)
            late.__typing_subst__ = lambda value: value
            if materialize:
                with assert_raises(TypeError) as caught:
                    union[str]
                assert str(caught.exception) == (
                    "argument late_parameter with __typing_subst__ was not found in __parameters__"
                )
                assert union.__parameters__ is cached
            else:
                if not subscribe_first:
                    assert union.__parameters__ == (late,)
                assert union[str] == (list[str] | int)
                assert alias[str] == list[str]
                cached = union.__parameters__
                assert cached == (late,)
                assert union.__parameters__ is cached

    # typing.Union and flattening an existing union preserve the same lazy boundary.
    for make_union in (
        lambda alias: Union[alias, int],
        lambda alias: (alias | bytes) | int,
    ):
        late = LateParameter()
        alias = GenericAlias(list, late)
        union = make_union(alias)
        late.__typing_subst__ = lambda value: value
        assert union[str] == make_union(list[str])
        assert alias[str] == list[str]

    # Nested descriptor failures occur on first use and do not poison either cache.
    class FailingParameter:
        fail = True
        lookups = 0

        @property
        def __parameters__(self):
            self.lookups += 1
            if self.fail:
                raise RuntimeError("lazy union failure")
            return (parameter,)

    for subscribe in (False, True):
        failing = FailingParameter()
        alias = GenericAlias(list, failing)
        union = alias | int
        assert failing.lookups == 0
        with assert_raises(RuntimeError) as caught:
            if subscribe:
                union[str]
            else:
                union.__parameters__
        assert str(caught.exception) == "lazy union failure"
        failing.fail = False
        cached = union.__parameters__
        assert cached == (parameter,)
        count = failing.lookups
        assert union.__parameters__ is cached
        assert failing.lookups == count

    # Parameter collection uses identity, not union argument hash/equality rules.
    parameter_comparisons = []

    class EqualParameter(Parameter):
        def __hash__(self):
            parameter_comparisons.append("hash")
            return 0

        def __eq__(self, other):
            parameter_comparisons.append("eq")
            return True

    first = EqualParameter()
    second = EqualParameter()
    union = GenericAlias(tuple, (first, second)) | int
    parameter_comparisons.clear()
    cached = union.__parameters__
    assert len(cached) == 2
    assert cached[0] is first and cached[1] is second
    assert parameter_comparisons == []
    assert union.__parameters__ is cached

    # Empty caches are also stable when a nested alias's descriptor changes.
    state = {"parameters": ()}

    class ChangingAlias(GenericAlias):
        @property
        def __parameters__(self):
            return state["parameters"]

    union = ChangingAlias(list, int) | str
    cached = union.__parameters__
    assert cached == ()
    state["parameters"] = (parameter,)
    assert union.__parameters__ is cached
    with assert_raises(TypeError):
        union[bytes]

    # Reentrant callbacks may populate the cache. The outer calculation replaces
    # that cache on success and clears it on failure, allowing another attempt.
    inner_parameter = Parameter()
    outer_parameter = Parameter()

    for fail in (False, True):
        state = {"entered": False, "parameters": (inner_parameter,)}

        class ReentrantAlias(GenericAlias):
            @property
            def __parameters__(self):
                if not state["entered"]:
                    state["entered"] = True
                    state["inner"] = state["union"].__parameters__
                    if fail:
                        raise RuntimeError("outer union failure")
                    return (outer_parameter,)
                return state["parameters"]

        state["union"] = ReentrantAlias(list, int) | str
        union = state["union"]
        if fail:
            with assert_raises(RuntimeError) as caught:
                union.__parameters__
            assert str(caught.exception) == "outer union failure"
            state["parameters"] = (outer_parameter,)
        cached = union.__parameters__
        assert state["inner"] == (inner_parameter,)
        assert cached == (outer_parameter,)
        assert cached is not state["inner"]
        assert union.__parameters__ is cached

    # Concurrent callbacks must run without holding the union's parameter lock.
    barrier = threading.Barrier(2)

    class ConcurrentAlias(GenericAlias):
        @property
        def __parameters__(self):
            barrier.wait(timeout=10)
            return (parameter,)

    union = ConcurrentAlias(list, int) | str
    results = []
    errors = []

    def read_parameters():
        try:
            results.append(union.__parameters__)
        except BaseException as error:
            errors.append(error)

    threads = [threading.Thread(target=read_parameters) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=15)
    assert all(not thread.is_alive() for thread in threads)
    assert errors == []
    assert results == [(parameter,), (parameter,)]
    cached = union.__parameters__
    assert any(cached is result for result in results)
    assert union.__parameters__ is cached


test_union_lazy_parameters()
