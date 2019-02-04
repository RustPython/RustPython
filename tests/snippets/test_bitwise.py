def assert_raises(expr, exc_type):
    """
    Helper function to assert `expr` raises an exception of type `exc_type`
    Args:
        expr: Callable
        exec_type: Exception
    Returns:
        None
    Raises:
        Assertion error on failure
    """
    try:
        expr(None)
    except exc_type:
        assert True
    else:
        assert False

#
# Tests
#
assert 8 >> 3 == 1
assert 8 << 3 == 64

# Left shift raises type error
assert_raises(lambda _: 1 << 0.1, TypeError)
assert_raises(lambda _: 1 << "abc", TypeError)

# Right shift raises type error
assert_raises(lambda _: 1 >> 0.1, TypeError)
assert_raises(lambda _: 1 >> "abc", TypeError)

# Left shift raises value error on negative
assert_raises(lambda _: 1 << -1, ValueError)

# Right shift raises value error on negative
assert_raises(lambda _: 1 >> -1, ValueError)
