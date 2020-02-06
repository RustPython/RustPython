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
