from testutils import assert_raises

assert divmod(11, 3) == (3, 2)
assert divmod(8,11) == (0, 8)
assert divmod(0.873, 0.252) == (3.0, 0.11699999999999999)
assert divmod(-86340, 86400) == (-1, 60)

assert_raises(ZeroDivisionError, divmod, 5, 0, _msg='divmod by zero')
assert_raises(ZeroDivisionError, divmod, 5.0, 0.0, _msg='divmod by zero')
