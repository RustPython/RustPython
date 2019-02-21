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
        failmsg = '{!s} was not raised'.format(exc_type.__name__)
        if msg is not None:
            failmsg += ': {!s}'.formt(msg)
        assert False, failmsg
