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
        failmsg = f'{exc_type.__name__} was not raised'
        if msg is not None:
            failmsg += f': {msg}'
        assert False, failmsg
