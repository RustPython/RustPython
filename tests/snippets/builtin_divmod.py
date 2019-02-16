from testutils import assert_raises

assert divmod(11, 3) == (3, 2)
assert divmod(8,11) == (0, 8)
assert divmod(0.873, 0.252) == (3.0, 0.11699999999999999)

assert_raises(ZeroDivisionError, lambda: divmod(5, 0), 'divmod by zero')
assert_raises(ZeroDivisionError, lambda: divmod(5.0, 0.0), 'divmod by zero')
