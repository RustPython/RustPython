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


def assertRaises(expected, *args, **kw):
    if not args:
        assert not kw
        return _assertRaises(expected)
    else:
        f, f_args = args[0], args[1:]
        with _assertRaises(expected):
            f(*f_args, **kw)


class _assertRaises:
    def __init__(self, expected):
        self.expected = expected
        self.exception = None

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if exc_type is None:
            failmsg = '{} was not raised'.format(self.expected.__name__)
            assert False, failmsg
        if not issubclass(exc_type, self.expected):
            return False

        self.exception = exc_val
        return True


class TestFailingBool:
    def __bool__(self):
        raise RuntimeError

class TestFailingIter:
    def __iter__(self):
        raise RuntimeError
