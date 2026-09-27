import gc
import weakref
from functools import cache, lru_cache, partial, reduce

from testutils import assert_raises


class Squares:
    def __init__(self, max):
        self.max = max
        self.sofar = []

    def __len__(self):
        return len(self.sofar)

    def __getitem__(self, i):
        if not 0 <= i < self.max:
            raise IndexError
        n = len(self.sofar)
        while n <= i:
            self.sofar.append(n * n)
            n += 1
        return self.sofar[i]


def add(a, b):
    return a + b


assert reduce(add, ["a", "b", "c"]) == "abc"
assert reduce(add, ["a", "b", "c"], str(42)) == "42abc"
assert reduce(add, [["a", "c"], [], ["d", "w"]], []) == ["a", "c", "d", "w"]
assert reduce(add, [["a", "c"], [], ["d", "w"]], []) == ["a", "c", "d", "w"]
assert reduce(lambda x, y: x * y, range(2, 21), 1) == 2432902008176640000
assert reduce(add, Squares(10)) == 285
assert reduce(add, Squares(10), 0) == 285
assert reduce(add, Squares(0), 0) == 0
assert reduce(42, "1") == "1"
assert reduce(42, "", "1") == "1"

with assert_raises(TypeError):
    reduce()

with assert_raises(TypeError):
    reduce(42, 42)

with assert_raises(TypeError):
    reduce(42, 42, 42)


class TestFailingIter:
    def __iter__(self):
        raise RuntimeError


with assert_raises(RuntimeError):
    reduce(add, TestFailingIter())

assert reduce(add, [], None) == None
assert reduce(add, [], 42) == 42


class BadSeq:
    def __getitem__(self, index):
        raise ValueError


with assert_raises(ValueError):
    reduce(42, BadSeq())


# Test reduce()'s use of iterators.
class SequenceClass:
    def __init__(self, n):
        self.n = n

    def __getitem__(self, i):
        if 0 <= i < self.n:
            return i
        else:
            raise IndexError


assert reduce(add, SequenceClass(5)) == 10
assert reduce(add, SequenceClass(5), 42) == 52
with assert_raises(TypeError):
    reduce(add, SequenceClass(0))

assert reduce(add, SequenceClass(0), 42) == 42
assert reduce(add, SequenceClass(1)) == 0
assert reduce(add, SequenceClass(1), 42) == 42

d = {"one": 1, "two": 2, "three": 3}
assert reduce(add, d) == "".join(d.keys())

p = partial(add)
try:
    del p.__dict__
    assert False, "TypeError expected for partial dict deletion"
except TypeError:
    pass


class CallbackOwner:
    def handle(self, value):
        return value + 1


def check_callback_cycle(make_callback):
    owner = CallbackOwner()
    owner.callback = make_callback(owner)
    owner_ref = weakref.ref(owner)
    callback = owner.callback
    del owner
    gc.collect()
    assert owner_ref() is not None
    assert callback() == 2
    del callback
    gc.collect()
    assert owner_ref() is None


# Each partial field can retain the callback owner.
check_callback_cycle(lambda owner: partial(owner.handle, 1))
check_callback_cycle(lambda owner: partial(CallbackOwner.handle, owner, 1))
check_callback_cycle(lambda owner: partial(CallbackOwner.handle, self=owner, value=1))

part = partial(int)
part.__setstate__((part, (), {}, None))
part_ref = weakref.ref(part)
del part
gc.collect()
assert part_ref() is None


def check_cached_cycles(decorate):
    owner = CallbackOwner()
    owner.callback = decorate(owner.handle)
    owner_ref = weakref.ref(owner)
    assert owner.callback(1) == 2
    gc.collect()
    assert owner.callback(1) == 2
    del owner
    gc.collect()
    assert owner_ref() is None

    # Keep only the cached argument connected back to the wrapper.
    def consume(value):
        return None

    cached = decorate(consume)
    owner = CallbackOwner()
    owner.callback = cached
    owner_ref = weakref.ref(owner)
    cached(owner)
    del owner
    gc.collect()
    assert owner_ref() is not None
    del cached
    gc.collect()
    assert owner_ref() is None

    # Cached results can point back to the wrapper.
    def produce():
        return CallbackOwner()

    cached = decorate(produce)
    owner = cached()
    owner.callback = cached
    owner_ref = weakref.ref(owner)
    del owner
    gc.collect()
    assert owner_ref() is cached()
    del cached
    gc.collect()
    assert owner_ref() is None


for decorate in (cache, lru_cache(maxsize=8)):
    check_cached_cycles(decorate)

check_callback_cycle(lambda owner: partial(lru_cache(maxsize=0)(owner.handle), 1))


def check_partial_setstate_release():
    released = []

    class Finalizer:
        def __call__(self):
            pass

        def __del__(self):
            released.append(isinstance(part.args, tuple))

    part = partial(Finalizer(), Finalizer(), old=Finalizer())
    part.__setstate__((int, (), {}, None))
    assert released == [True, True, True]


check_partial_setstate_release()


def check_cache_clear_release(decorate):
    released = []

    class Result:
        def __del__(self):
            info = cached.cache_info()
            released.append((info.hits, info.misses, info.currsize))

    cached = decorate(Result)
    cached()
    cached()
    cached.cache_clear()
    assert released == [(0, 0, 0)]


for decorate in (cache, lru_cache(maxsize=8)):
    check_cache_clear_release(decorate)
