from testutils import assert_raises
import itertools

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
