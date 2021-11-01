def _test_termios():
    # These tests are in a function so we can only run them if termios is available
    assert termios.error.__module__ == "termios"
    assert termios.error.__name__ == "error"


try:
    import termios
except ImportError:
    # Not all platforms have termios, noop
    pass
else:
    _test_termios()
