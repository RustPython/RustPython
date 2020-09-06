from array import array

a1 = array("b", [0, 1, 2, 3])

assert a1.tobytes() == b"\x00\x01\x02\x03"
assert a1[2] == 2

assert list(a1) == [0, 1, 2, 3]

a1.reverse()
assert a1 == array("B", [3, 2, 1, 0])

a1.extend([4, 5, 6, 7])

assert a1 == array("h", [3, 2, 1, 0, 4, 5, 6, 7])

# eq, ne
a = array("b", [0, 1, 2, 3])
b = a
assert a.__ne__(b) is False
b = array("B", [3, 2, 1, 0])
assert a.__ne__(b) is True

def test_float_with_integer_input():
    f = array("f", [0, 1, 2.0, 3.0])
    f.append(4)
    f.insert(0, -1)
    assert f.count(4) == 1
    f.remove(1)
    assert f.index(0) == 1
    f[0] = -2
    assert f == array("f", [-2, 0, 2, 3, 4])

test_float_with_integer_input()

# slice assignment step overflow behaviour test
T = 'I'
a = array(T, range(10))
b = array(T, [100])
a[::9999999999] = b
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 9])
a[::-9999999999] = b
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
c = array(T)
a[0:0:9999999999] = c
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
a[0:0:-9999999999] = c
assert a == array(T, [100, 1, 2, 3, 4, 5, 6, 7, 8, 100])
del a[::9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8, 100])
del a[::-9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])
del a[0:0:9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])
del a[0:0:-9999999999]
assert a == array(T, [1, 2, 3, 4, 5, 6, 7, 8])

def test_float_with_nan():
    f = float('nan')
    a = array('f')
    a.append(f)
    assert not (a == a)
    assert a != a
    assert not (a < a)
    assert not (a <= a)
    assert not (a > a)
    assert not (a >= a)

test_float_with_nan()

def test_different_type_cmp():
    a = array('i', [-1, -2, -3, -4])
    b = array('I', [1, 2, 3, 4])
    c = array('f', [1, 2, 3, 4])
    assert a < b
    assert b > a
    assert b == c
    assert a < c
    assert c > a

test_different_type_cmp()
