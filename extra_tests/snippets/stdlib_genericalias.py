from types import GenericAlias

from testutils import assert_raises


class Parameter:
    def __typing_subst__(self, value):
        return value


parameter = Parameter()


# Construction does not inspect arguments. Both entry points populate the cache
# using the attributes present at the first lookup, rather than construction.
class LateParameter:
    __parameters__ = (parameter,)

    def __repr__(self):
        return "late_parameter"


for materialize in (False, True):
    late = LateParameter()
    alias = GenericAlias(list, late)
    if materialize:
        cached = alias.__parameters__
        assert cached == (parameter,)
    late.__typing_subst__ = lambda value: value
    if materialize:
        with assert_raises(TypeError) as caught:
            alias[int]
        assert str(caught.exception) == (
            "argument late_parameter with __typing_subst__ was not found in __parameters__"
        )
        assert alias.__parameters__ is cached
    else:
        assert alias[int] == list[int]
        cached = alias.__parameters__
        assert cached == (late,)
        assert alias.__parameters__ is cached

late = LateParameter()
alias = GenericAlias(list, late)
late.__typing_subst__ = lambda value: value
assert alias.__parameters__ == (late,)
assert alias[int] == list[int]

# Empty tuples are cached too, even if parameters appear later.
empty = LateParameter()
empty.__parameters__ = ()
alias = GenericAlias(list, empty)
cached = alias.__parameters__
empty.__typing_subst__ = lambda value: value
assert alias.__parameters__ is cached
assert cached == ()
with assert_raises(TypeError):
    alias[int]


# Lookup errors happen lazily and leave the cache uninitialized for a retry.
class FailingParameter:
    def __init__(self, failing_attribute):
        self.failing_attribute = failing_attribute
        self.fail = True
        self.lookups = []

    def __getattribute__(self, name):
        if name in ("__typing_subst__", "__parameters__"):
            self.lookups.append(name)
            if name == self.failing_attribute and self.fail:
                raise RuntimeError("parameter lookup failed")
        return object.__getattribute__(self, name)

    __parameters__ = (parameter,)


for attribute in ("__typing_subst__", "__parameters__"):
    for subscribe in (False, True):
        failing = FailingParameter(attribute)
        alias = GenericAlias(list, failing)
        assert failing.lookups == []
        with assert_raises(RuntimeError) as caught:
            if subscribe:
                alias[int]
            else:
                alias.__parameters__
        assert str(caught.exception) == "parameter lookup failed"
        failing.fail = False
        cached = alias.__parameters__
        assert cached == (parameter,)
        count = len(failing.lookups)
        assert alias.__parameters__ is cached
        assert len(failing.lookups) == count


# A nested lookup may populate the cache, but the outer calculation replaces it.
inner_parameter = Parameter()
outer_parameter = Parameter()


class ReentrantParameter:
    entered = False

    @property
    def __parameters__(self):
        if not self.entered:
            self.entered = True
            self.inner = self.alias.__parameters__
            return (outer_parameter,)
        return (inner_parameter,)


reentrant = ReentrantParameter()
reentrant.alias = GenericAlias(list, reentrant)
cached = reentrant.alias.__parameters__
assert reentrant.inner == (inner_parameter,)
assert cached == (outer_parameter,)
assert cached is reentrant.alias.__parameters__
assert cached is not reentrant.inner


# A cyclic parameter lookup raises rather than deadlocking or overflowing Rust.
class RecursiveParameter:
    recurse = True

    @property
    def __parameters__(self):
        if self.recurse:
            return self.alias.__parameters__
        return (parameter,)


recursive = RecursiveParameter()
recursive.alias = GenericAlias(list, recursive)
with assert_raises(RecursionError):
    recursive.alias.__parameters__
recursive.recurse = False
assert recursive.alias.__parameters__ == (parameter,)

# Substitution and unpacking construct aliases with their own lazy caches.
failing = FailingParameter("__parameters__")
result = GenericAlias(list, parameter)[failing]
assert failing.lookups == []
with assert_raises(RuntimeError):
    result.__parameters__

failing = FailingParameter("__parameters__")
original = GenericAlias(tuple, failing)
starred = next(iter(original))
assert failing.lookups == []
with assert_raises(RuntimeError):
    starred.__parameters__
failing.fail = False
assert starred.__parameters__ == (parameter,)
failing.__parameters__ = ()
assert original.__parameters__ == ()
assert starred.__parameters__ == (parameter,)

# Concurrent descriptor callbacks can overlap without holding the cache lock.
import threading

barrier = threading.Barrier(2)


class ConcurrentParameter:
    @property
    def __parameters__(self):
        barrier.wait(timeout=10)
        return (parameter,)


concurrent = GenericAlias(list, ConcurrentParameter())
results = []
errors = []


def read_parameters():
    try:
        results.append(concurrent.__parameters__)
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
cached = concurrent.__parameters__
assert any(cached is result for result in results)
assert concurrent.__parameters__ is cached


# Failure in the outer calculation clears an inner successful cache as well.
class ReentrantFailure:
    entered = False
    fail = True

    @property
    def __parameters__(self):
        if not self.entered:
            self.entered = True
            self.inner = self.alias.__parameters__
            if self.fail:
                raise RuntimeError("outer lookup failed")
        return (inner_parameter,) if self.fail else (outer_parameter,)


for subscribe in (False, True):
    failure = ReentrantFailure()
    failure.alias = GenericAlias(list, failure)
    with assert_raises(RuntimeError) as caught:
        if subscribe:
            failure.alias[int]
        else:
            failure.alias.__parameters__
    assert str(caught.exception) == "outer lookup failed"
    assert failure.inner == (inner_parameter,)
    failure.fail = False
    cached = failure.alias.__parameters__
    assert cached == (outer_parameter,)
    assert failure.alias.__parameters__ is cached
