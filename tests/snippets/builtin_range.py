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

assert range(2**63+1)[2**63] == 9223372036854775808

# len tests
assert len(range(10, 5)) == 0, 'Range with no elements should have length = 0'
assert len(range(10, 5, -2)) == 3, 'Expected length 3, for elements: 10, 8, 6'
assert len(range(5, 10, 2)) == 3, 'Expected length 3, for elements: 5, 7, 9'

# index tests
assert range(10).index(6) == 6
assert range(4, 10).index(6) == 2
assert range(4, 10, 2).index(6) == 1

# index raises value error on out of bounds
assert_raises(lambda _: range(10).index(-1), ValueError)
assert_raises(lambda _: range(10).index(10), ValueError)

# index raises value error if out of step
assert_raises(lambda _: range(4, 10, 2).index(5), ValueError)
