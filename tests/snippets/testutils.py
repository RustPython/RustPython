def assert_raises(exc_type, expr, msg=None):
    """
    Helper function to assert `expr` raises an exception of type `exc_type`.
    Args:
        expr: Callable
        exec_type: Exception
    Returns:
        None
    Raises:
        Assertion error on failure
    """
    try:
        expr()
    except exc_type:
        pass
    else:
        failmsg = '{} was not raised'.format(exc_type.__name__)
        if msg is not None:
            failmsg += ': {}'.format(msg)
        assert False, failmsg


class assertRaises:
    def __init__(self, expected):
        self.expected = expected

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if exc_type is None:
            failmsg = '{} was not raised'.format(self.expected.__name__)
            assert False, failmsg
        if not issubclass(exc_type, self.expected):
            return False
        return True


class TestFailingBool:
    def __bool__(self):
        raise RuntimeError

class TestFailingIter:
    def __iter__(self):
        raise RuntimeError
