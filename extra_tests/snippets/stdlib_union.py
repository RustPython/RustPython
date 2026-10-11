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
