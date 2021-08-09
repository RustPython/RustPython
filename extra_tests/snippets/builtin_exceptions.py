import platform
import sys


# Regression to:
# https://github.com/RustPython/RustPython/issues/2779

class MyError(Exception):
    pass


e = MyError('message')

try:
    raise e from e
except MyError as exc:
    # It was a segmentation fault before, will print info to stdout:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert isinstance(exc, MyError)
    assert exc.__cause__ is e
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'

try:
    raise ValueError('test') from e
except ValueError as exc:
    sys.excepthook(type(exc), exc, exc.__traceback__)  # ok, will print two excs
    assert isinstance(exc, ValueError)
    assert exc.__cause__ is e
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'


# New case:
# potential recursion on `__context__` field

e = MyError('message')

try:
    try:
        raise e
    except MyError as exc:
        raise e
    else:
        assert False, 'exception not raised'
except MyError as exc:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert exc.__cause__ is None
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'

e = MyError('message')

try:
    try:
        raise e
    except MyError as exc:
        raise exc
    else:
        assert False, 'exception not raised'
except MyError as exc:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert exc.__cause__ is None
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'

e = MyError('message')

try:
    try:
        raise e
    except MyError as exc:
        raise e from e
    else:
        assert False, 'exception not raised'
except MyError as exc:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert exc.__cause__ is e
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'

e = MyError('message')

try:
    try:
        raise e
    except MyError as exc:
        raise exc from e
    else:
        assert False, 'exception not raised'
except MyError as exc:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert exc.__cause__ is e
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'


# New case:
# two exception in a recursion loop

class SubError(MyError):
    pass

e = MyError('message')
d = SubError('sub')


try:
    raise e from d
except MyError as exc:
    # It was a segmentation fault before, will print info to stdout:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert isinstance(exc, MyError)
    assert exc.__cause__ is d
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'

e = MyError('message')

try:
    raise d from e
except SubError as exc:
    # It was a segmentation fault before, will print info to stdout:
    sys.excepthook(type(exc), exc, exc.__traceback__)
    assert isinstance(exc, SubError)
    assert exc.__cause__ is e
    assert exc.__context__ is None
else:
    assert False, 'exception not raised'


# New case:
# explicit `__context__` manipulation.

e = MyError('message')
e.__context__ = e

try:
    raise e
except MyError as exc:
    # It was a segmentation fault before, will print info to stdout:
    if platform.python_implementation() == 'RustPython':
        # For some reason `CPython` hangs on this code:
        sys.excepthook(type(exc), exc, exc.__traceback__)
        assert isinstance(exc, MyError)
        assert exc.__cause__ is None
        assert exc.__context__ is e
