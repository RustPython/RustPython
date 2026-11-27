import gc
import weakref

# Objects of exactly int/float/complex/range are recycled through per-type
# freelists; everything else about deallocation must behave as before.


## Recycled objects carry the right value and type


def churn_ints(n):
    total = 0
    for i in range(1000, 1000 + n):
        total += i
    return total


assert churn_ints(5000) == sum(range(1000, 6000))
seen = [i for i in range(250, 270)]
assert seen == list(range(250, 270))
for i in range(-5, 257):
    assert i is int(str(i)), i

floats = [x * 1.5 for x in range(2000)]
assert floats[-1] == 1999 * 1.5
assert all(type(f) is float for f in floats)
complexes = [complex(i, -i) for i in range(2000)]
assert complexes[123] == complex(123, -123)
ranges = [range(i, i + 3) for i in range(2000)]
assert ranges[77] == range(77, 80) and list(ranges[5]) == [5, 6, 7]
del floats, complexes, ranges


## Subclass instances take the full path: __del__ and weakrefs still work


class DelInt(int):
    deleted = 0

    def __del__(self):
        DelInt.deleted += 1


for i in range(100):
    DelInt(i + 1000)
gc.collect()
assert DelInt.deleted == 100, DelInt.deleted


class RefFloat(float):
    pass


callbacks = []
obj = RefFloat(12345.5)
ref = weakref.ref(obj, lambda r: callbacks.append(r))
del obj
gc.collect()
assert ref() is None
assert len(callbacks) == 1


class DelFloat(float):
    deleted = 0

    def __del__(self):
        DelFloat.deleted += 1


for i in range(50):
    DelFloat(i + 0.5)
gc.collect()
assert DelFloat.deleted == 50, DelFloat.deleted


## Subclass husks never come back as plain ints


class Plain(int):
    pass


for i in range(500):
    Plain(i + 1000)
for i in range(1000, 1500):
    x = i * 1
    assert type(x) is int, type(x)
    assert x == i

# bool shares int's payload; its singletons are untouched by the churn.
assert True + 0 == 1 and type(True + 0) is int
assert type(True) is bool and True is (1 == 1)
