from testutils import assert_raises

assert_raises(ZeroDivisionError, lambda: 5 / 0)
assert_raises(ZeroDivisionError, lambda: 5 / -0.0)
assert_raises(ZeroDivisionError, lambda: 5 / (2-2))
assert_raises(ZeroDivisionError, lambda: 5 % 0)
assert_raises(ZeroDivisionError, lambda: 5 // 0)
assert_raises(ZeroDivisionError, lambda: 5.3 // (-0.0))
assert_raises(ZeroDivisionError, divmod, 5, 0)

assert issubclass(ZeroDivisionError, ArithmeticError)
