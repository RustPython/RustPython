empty_exc = KeyError()
assert str(empty_exc) == ''
assert repr(empty_exc) == 'KeyError()'
assert len(empty_exc.args) == 0
assert type(empty_exc.args) == tuple

exc = KeyError('message')
assert str(exc) == "'message'"
assert repr(exc) == "KeyError('message',)"

exc = KeyError('message', 'another message')
assert str(exc) == "('message', 'another message')"
assert repr(exc) == "KeyError('message', 'another message')"
assert exc.args[0] == 'message'
assert exc.args[1] == 'another message'

class A:
    def __repr__(self):
        return 'repr'
    def __str__(self):
        return 'str'

exc = KeyError(A())
assert str(exc) == 'repr'
assert repr(exc) == 'KeyError(repr,)'
