import builtins
import platform
import sys

def exceptions_eq(e1, e2):
    return type(e1) is type(e2) and e1.args == e2.args

def round_trip_repr(e):
    return exceptions_eq(e, eval(repr(e)))

# KeyError
empty_exc = KeyError()
assert str(empty_exc) == ''
assert round_trip_repr(empty_exc)
assert len(empty_exc.args) == 0
assert type(empty_exc.args) == tuple

exc = KeyError('message')
assert str(exc) == "'message'"
assert round_trip_repr(exc)

assert LookupError.__str__(exc) == "message"

exc = KeyError('message', 'another message')
assert str(exc) == "('message', 'another message')"
assert round_trip_repr(exc)
assert exc.args[0] == 'message'
assert exc.args[1] == 'another message'

class A:
    def __repr__(self):
        return 'A()'
    def __str__(self):
        return 'str'
    def __eq__(self, other):
        return type(other) is A

exc = KeyError(A())
assert str(exc) == 'A()'
assert round_trip_repr(exc)

# ImportError / ModuleNotFoundError
exc = ImportError()
assert exc.name is None
assert exc.path is None
assert exc.msg is None
assert exc.args == ()

exc = ImportError('hello')
assert exc.name is None
assert exc.path is None
assert exc.msg == 'hello'
assert exc.args == ('hello',)

exc = ImportError('hello', name='name', path='path')
assert exc.name == 'name'
assert exc.path == 'path'
assert exc.msg == 'hello'
assert exc.args == ('hello',)


class NewException(Exception):

	def __init__(self, value):
		self.value = value


try:
	raise NewException("test")
except NewException as e:
	assert e.value == "test"


exc = SyntaxError('msg', 1, 2, 3, 4, 5)
assert exc.msg == 'msg'
assert exc.filename is None
assert exc.lineno is None
assert exc.offset is None
assert exc.text is None

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


# Regression to
# https://github.com/RustPython/RustPython/issues/2771

# `BaseException` and `Exception`:
assert BaseException.__new__.__qualname__ == 'BaseException.__new__'
assert BaseException.__init__.__qualname__ == 'BaseException.__init__'
assert BaseException().__dict__ == {}

assert Exception.__new__.__qualname__ == 'Exception.__new__'
assert Exception.__init__.__qualname__ == 'Exception.__init__'
assert Exception().__dict__ == {}


# Extends `BaseException`, simple:
assert KeyboardInterrupt.__new__.__qualname__ == 'KeyboardInterrupt.__new__'
assert KeyboardInterrupt.__init__.__qualname__ == 'KeyboardInterrupt.__init__'
assert KeyboardInterrupt().__dict__ == {}


# Extends `Exception`, simple:
assert TypeError.__new__.__qualname__ == 'TypeError.__new__'
assert TypeError.__init__.__qualname__ == 'TypeError.__init__'
assert TypeError().__dict__ == {}


# Extends `Exception`, complex:
assert OSError.__new__.__qualname__ == 'OSError.__new__'
assert OSError.__init__.__qualname__ == 'OSError.__init__'
assert OSError().__dict__ == {}
assert OSError.errno
assert OSError.strerror
assert OSError(1, 2).errno
assert OSError(1, 2).strerror


# OSError Unexpected number of arguments
w = OSError()
assert w.errno == None
assert not sys.platform.startswith("win") or w.winerror == None
assert w.strerror == None
assert w.filename == None
assert w.filename2 == None
assert str(w) == ""

w = OSError(0)
assert w.errno == None
assert not sys.platform.startswith("win") or w.winerror == None
assert w.strerror == None
assert w.filename == None
assert w.filename2 == None
assert str(w) == "0"

w = OSError('foo')
assert w.errno == None
assert not sys.platform.startswith("win") or w.winerror == None
assert w.strerror == None
assert w.filename == None
assert w.filename2 == None
assert str(w) == "foo"

w = OSError('a', 'b', 'c', 'd', 'e', 'f')
assert w.errno == None
assert not sys.platform.startswith("win") or w.winerror == None
assert w.strerror == None
assert w.filename == None
assert w.filename2 == None
assert str(w) == "('a', 'b', 'c', 'd', 'e', 'f')"

# Custom `__new__` and `__init__`:
assert ImportError.__init__.__qualname__ == 'ImportError.__init__'
assert ImportError(name='a').name == 'a'
assert (
    ModuleNotFoundError.__init__.__qualname__ == 'ModuleNotFoundError.__init__'
)
assert ModuleNotFoundError(name='a').name == 'a'


# Check that all exceptions have string `__doc__`:
for exc in filter(
    lambda obj: isinstance(obj, BaseException),
    vars(builtins).values(),
):
    assert isinstance(exc.__doc__, str)
