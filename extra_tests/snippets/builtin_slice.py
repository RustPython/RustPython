from testutils import assert_raises

a = slice(10)
assert a.start == None
assert a.stop == 10
assert a.step == None

a = slice(0, 10, 1)
assert a.start == 0
assert a.stop == 10
assert a.step == 1

assert slice(10).__repr__() == 'slice(None, 10, None)'
assert slice(None).__repr__() == 'slice(None, None, None)'
assert slice(0, 10, 13).__repr__() == 'slice(0, 10, 13)'
assert slice('0', 1.1, 2+3j).__repr__() == "slice('0', 1.1, (2+3j))"

assert slice(10) == slice(10)
assert slice(-1) != slice(1)
assert slice(0, 10, 3) != slice(0, 11, 3)
assert slice(0, None, 3) != slice(0, 'a', 3)
assert slice(0, 'a', 3) == slice(0, 'a', 3)

assert slice(0, 0, 0).__eq__(slice(0, 0, 0))
assert not slice(0, 0, 1).__eq__(slice(0, 0, 0))
assert not slice(0, 1, 0).__eq__(slice(0, 0, 0))
assert not slice(1, 0, 0).__eq__(slice(0, 0, 0))
assert slice(1, 0, 0).__ne__(slice(0, 0, 0))
assert slice(0, 1, 0).__ne__(slice(0, 0, 0))
assert slice(0, 0, 1).__ne__(slice(0, 0, 0))

assert slice(0).__eq__(0) == NotImplemented
assert slice(0).__ne__(0) == NotImplemented
assert slice(None).__ne__(slice(0))

# slice gt, ge, lt, le
assert_raises(TypeError, lambda: slice(0, slice(), 0) < slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, slice(), 0) <= slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, slice(), 0) > slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, slice(), 0) >= slice(0, 0, 0))

assert_raises(TypeError, lambda: slice(0, 0, 0) < slice(0, 0, slice()))
assert_raises(TypeError, lambda: slice(0, 0, 0) <= slice(0, 0, slice()))
assert_raises(TypeError, lambda: slice(0, 0, 0) > slice(0, 0, slice()))
assert_raises(TypeError, lambda: slice(0, 0, 0) >= slice(0, 0, slice()))

assert_raises(TypeError, lambda: slice(0, 0) >= slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, 0) <= slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, 0) < slice(0, 0, 0))
assert_raises(TypeError, lambda: slice(0, 0) > slice(0, 0, 0))

assert slice(0, 0, 0) < slice(0, 1, -1)
assert slice(0, 0, 0) < slice(0, 0, 1)
assert slice(0, 0, 0) > slice(0, 0, -1)
assert slice(0, 0, 0) >= slice(0, 0, -1)
assert not slice(0, 0, 0) <= slice(0, 0, -1)

assert slice(0, 0, 0) > slice(0, -1, 1)
assert slice(0, 0, 0) >= slice(0, -1, 1)
assert slice(0, 0, 0) >= slice(0, -1, 1)

assert slice(0, 0, 0) <= slice(0, 0, 1)
assert slice(0, 0, 0) <= slice(0, 0, 0)
assert slice(0, 0, 0) <= slice(0, 0, 0)
assert not slice(0, 0, 0) > slice(0, 0, 0)
assert not slice(0, 0, 0) < slice(0, 0, 0)

assert not slice(0, float('nan'), float('nan')) <= slice(0, float('nan'), 1)
assert not slice(0, float('nan'), float('nan')) <= slice(0, float('nan'), float('nan'))
assert not slice(0, float('nan'), float('nan')) >= slice(0, float('nan'), float('nan'))
assert not slice(0, float('nan'), float('nan')) < slice(0, float('nan'), float('nan'))
assert not slice(0, float('nan'), float('nan')) > slice(0, float('nan'), float('nan'))

assert slice(0, float('inf'), float('inf')) >= slice(0, float('inf'), 1)
assert slice(0, float('inf'), float('inf')) <= slice(0, float('inf'), float('inf'))
assert slice(0, float('inf'), float('inf')) >= slice(0, float('inf'), float('inf'))
assert not slice(0, float('inf'), float('inf')) < slice(0, float('inf'), float('inf'))
assert not slice(0, float('inf'), float('inf')) > slice(0, float('inf'), float('inf'))

assert_raises(TypeError, lambda: slice(0) < 3)
assert_raises(TypeError, lambda: slice(0) > 3)
assert_raises(TypeError, lambda: slice(0) <= 3)
assert_raises(TypeError, lambda: slice(0) >= 3)

assert_raises(TypeError, hash, slice(0))
assert_raises(TypeError, hash, slice(None))

def dict_slice():
    d = {}
    d[slice(0)] = 3

assert_raises(TypeError, dict_slice)

assert slice(None           ).indices(10) == (0, 10,  1)
assert slice(None,  None,  2).indices(10) == (0, 10,  2)
assert slice(1,     None,  2).indices(10) == (1, 10,  2)
assert slice(None,  None, -1).indices(10) == (9, -1, -1)
assert slice(None,  None, -2).indices(10) == (9, -1, -2)
assert slice(3,     None, -2).indices(10) == (3, -1, -2)

# issue 3004 tests
assert slice(None, -9).indices(10) == (0, 1, 1)
assert slice(None, -10).indices(10) == (0, 0, 1)
assert slice(None, -11).indices(10) == (0, 0, 1)
assert slice(None, -10, -1).indices(10) == (9, 0, -1)
assert slice(None, -11, -1).indices(10) == (9, -1, -1)
assert slice(None, -12, -1).indices(10) == (9, -1, -1)
assert slice(None, 9).indices(10) == (0, 9, 1)
assert slice(None, 10).indices(10) == (0, 10, 1)
assert slice(None, 11).indices(10) == (0, 10, 1)
assert slice(None, 8, -1).indices(10) == (9, 8, -1)
assert slice(None, 9, -1).indices(10) == (9, 9, -1)
assert slice(None, 10, -1).indices(10) == (9, 9, -1)

assert \
    slice(-100,  100).indices(10) == \
    slice(None      ).indices(10)

assert \
    slice(100,  -100,  -1).indices(10) == \
    slice(None, None, -1).indices(10)

assert slice(-100, 100, 2).indices(10) == (0, 10,  2)

try:
	slice(None, None, 0)
	assert "zero step" == "throws an exception"
except:
	pass

a = []
b = [1, 2]
c = list(range(10))
d = "123456"

slice_a = slice(5)
assert slice_a.start is None
assert slice_a.stop == 5
assert slice_a.step is None

slice_b = slice(1, 5)
assert slice_b.start == 1
assert slice_b.stop == 5
assert slice_b.step is None

slice_c = slice(1, 5, 2)
assert slice_c.start == 1
assert slice_c.stop == 5
assert slice_c.step == 2

a = object()
slice_d = slice(a, "v", 1.0)
assert slice_d.start is a
assert slice_d.stop == "v"
assert slice_d.step == 1.0


class SubScript(object):
    def __getitem__(self, item):
        assert type(item) == slice

    def __setitem__(self, key, value):
        assert type(key) == slice


ss = SubScript()
_ = ss[:]
ss[:1] = 1


class CustomIndex:
    def __init__(self, x):
        self.x = x

    def __index__(self):
        return self.x


assert c[CustomIndex(1):CustomIndex(3)] == [1, 2]
assert d[CustomIndex(1):CustomIndex(3)] == "23"


def test_all_slices():
    """
    test all possible slices except big number
    """

    mod = __import__('cpython_generated_slices')

    ll = mod.LL
    start = mod.START
    end = mod.END
    step = mod.STEP
    slices_res = mod.SLICES_RES

    count = 0
    failures = []
    for s in start:
        for e in end:
            for t in step:
                lhs = ll[s:e:t]
                try:
                    assert lhs == slices_res[count]
                except AssertionError:
                    failures.append(
                        "start: {} ,stop: {}, step {}. Expected: {}, found: {}".format(
                            s, e, t, lhs, slices_res[count]
                        )
                    )
                count += 1

    if failures:
        for f in failures:
            print(f)
        print(len(failures), "slices failed")


test_all_slices()
