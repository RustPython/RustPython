from testutils import assert_raises

assert 1 < 2
assert 1 < 2 < 3
assert 5 == 5 == 5
assert (5 == 5) == True
assert 5 == 5 != 4 == 4 > 3 > 2 < 3 <= 3 != 0 == 0

assert not 1 > 2
assert not 5 == 5 == True
assert not 5 == 5 != 5 == 5
assert not 1 < 2 < 3 > 4
assert not 1 < 2 > 3 < 4
assert not 1 > 2 < 3 < 4

def test_type_error(x, y):
    assert_raises(TypeError, lambda: x < y)
    assert_raises(TypeError, lambda: x <= y)
    assert_raises(TypeError, lambda: x > y)
    assert_raises(TypeError, lambda: x >= y)

test_type_error([], 0)
test_type_error((), 0)
