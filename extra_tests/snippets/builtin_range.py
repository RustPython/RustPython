from testutils import assert_raises

assert range(2**63+1)[2**63] == 9223372036854775808

# len tests
assert len(range(10, 5)) == 0, 'Range with no elements should have length = 0'
assert len(range(10, 5, -2)) == 3, 'Expected length 3, for elements: 10, 8, 6'
assert len(range(5, 10, 2)) == 3, 'Expected length 3, for elements: 5, 7, 9'

# index tests
assert range(10).index(6) == 6
assert range(4, 10).index(6) == 2
assert range(4, 10, 2).index(6) == 1
assert range(10, 4, -2).index(8) == 1

assert_raises(ValueError, lambda: range(10).index(-1), _msg='out of bounds')
assert_raises(ValueError, lambda: range(10).index(10), _msg='out of bounds')
assert_raises(ValueError, lambda: range(4, 10, 2).index(5), _msg='out of step')
assert_raises(ValueError, lambda: range(10).index('foo'), _msg='not an int')
assert_raises(ValueError, lambda: range(1, 10, 0), _msg='step is zero')

# get tests
assert range(10)[0] == 0
assert range(10)[9] == 9
assert range(10, 0, -1)[0] == 10
assert range(10, 0, -1)[9] == 1
assert_raises(IndexError, lambda: range(10)[10], _msg='out of bound')

# slice tests
assert range(10)[0:3] == range(3)
assert range(10)[-5:9] == range(5, 9)
assert range(10)[100:10] == range(10, 10)
assert range(10)[-15:3] == range(0, 3)
assert range(10, 100, 3)[4:1000:5] == range(22, 100, 15)
assert range(10)[:] == range(10)
assert range(10, 0, -2)[0:5:2] == range(10, 0, -4)
assert range(10)[10:11] == range(10,10)
assert range(0, 10, -1)[::-1] == range(1, 1)
assert range(0, 10)[::-1] == range(9, -1, -1)
assert range(0, -10)[::-1] == range(-1, -1, -1)
assert range(0, -10)[::-1][::-1] == range(0, 0)
assert_raises(ValueError, lambda: range(0, 10)[::0], _msg='slice step cannot be zero')
assert_raises(TypeError, lambda: range(0, 10)['a':], _msg='slice indices must be integers or None or have an __index__ method')

# count tests
assert range(10).count(2) == 1
assert range(10).count(11) == 0
assert range(10).count(-1) == 0
assert range(9, 12).count(10) == 1
assert range(4, 10, 2).count(4) == 1
assert range(4, 10, 2).count(7) == 0
assert range(10).count("foo") == 0

# __eq__
assert range(1, 2, 3) == range(1, 2, 3)
assert range(1, 2, 1) == range(1, 2)
assert range(2) == range(0, 2)
assert range(0, 10, 3) == range(0, 12, 3)
assert range(20, 10, 3) == range(20, 12, 3)

assert range(10).__eq__(range(0, 10, 1)) is True
assert range(10).__ne__(range(0, 10, 1)) is False
assert range(10).__eq__(range(0, 11, 1)) is False
assert range(10).__ne__(range(0, 11, 1)) is True
assert range(0, 10, 3).__eq__(range(0, 11, 3)) is True
assert range(0, 10, 3).__ne__(range(0, 11, 3)) is False
#__lt__
assert range(1, 2, 3).__lt__(range(1, 2, 3)) == NotImplemented
assert range(1, 2, 1).__lt__(range(1, 2)) == NotImplemented
assert range(2).__lt__(range(0, 2)) == NotImplemented

#__gt__
assert range(1, 2, 3).__gt__(range(1, 2, 3)) == NotImplemented
assert range(1, 2, 1).__gt__(range(1, 2)) == NotImplemented
assert range(2).__gt__(range(0, 2)) == NotImplemented

#__le__
assert range(1, 2, 3).__le__(range(1, 2, 3)) == NotImplemented
assert range(1, 2, 1).__le__(range(1, 2)) == NotImplemented
assert range(2).__le__(range(0, 2)) == NotImplemented

#__ge__
assert range(1, 2, 3).__ge__(range(1, 2, 3)) == NotImplemented
assert range(1, 2, 1).__ge__(range(1, 2)) == NotImplemented
assert range(2).__ge__(range(0, 2)) == NotImplemented

# __bool__
assert bool(range(1))
assert bool(range(1, 2))

assert not bool(range(0))
assert not bool(range(1, 1))

# __contains__
assert 6 in range(10)
assert 6 in range(4, 10)
assert 6 in range(4, 10, 2)
assert 10 in range(10, 4, -2)
assert 8 in range(10, 4, -2)

assert -1 not in range(10)
assert 9 not in range(10, 4, -2)
assert 4 not in range(10, 4, -2)
assert 'foo' not in range(10)

# __reversed__
assert list(reversed(range(5))) == [4, 3, 2, 1, 0]
assert list(reversed(range(5, 0, -1))) == [1, 2, 3, 4, 5]
assert list(reversed(range(1,10,5))) == [6, 1]

# __reduce__
assert range(10).__reduce__()[0] == range
assert range(10).__reduce__()[1] == (0, 10, 1)
assert range(10, 1, -2).__reduce__()[0] == range
assert range(10, 1, -2).__reduce__()[1] == (10, 1, -2)

# range retains the original int refs
i = 2**64
assert range(i).stop is i

# negative index
assert range(10)[-1] == 9
assert_raises(IndexError, lambda: range(10)[-11], _msg='out of bound')
assert range(10)[-2:4] == range(8, 4)
assert range(10)[-6:-2] == range(4, 8)
assert range(50, 0, -2)[-5] == 10
assert range(50, 0, -2)[-5:3:5] == range(10, 44, -10)

assert hash(range(10)) == hash((10, 0, 1))
assert hash(range(10)) == hash(range(10))
assert hash(range(100)[20:30]) == hash(range(20, 30))
assert hash(range(10, 10)) == hash(range(0, 0))
assert hash(range(1, 2, 100)) == hash(range(1, 6, 100))

a = {}
for i in range(100):
    a[range(10)] = 1

assert len(a.keys()) == 1
