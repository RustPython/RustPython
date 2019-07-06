assert True
assert not False

assert 5

assert not 0
assert not []
assert not ()
assert not {}
assert not ""
assert not 0.0

assert not None

assert bool() == False
assert bool(1) == True
assert bool({}) == False

assert bool(NotImplemented) == True
assert bool(...) == True

if not 1:
    raise BaseException

if not {} and not [1]:
    raise BaseException

if not object():
    raise BaseException

class Falsey:
    def __bool__(self):
        return False

assert not Falsey()

assert (True or fake)
assert (False or True)
assert not (False or False)
assert ("thing" or 0) == "thing"

assert (True and True)
assert not (False and fake)
assert (True and 5) == 5

# Bools are also ints.
assert isinstance(True, int)
assert True + True == 2
assert False * 7 == 0
assert True > 0
assert int(True) == 1
assert True.conjugate() == 1
assert isinstance(True.conjugate(), int)

# Boolean operations on pairs of Bools should return Bools, not ints
assert (False | True) is True
assert (False & True) is False
assert (False ^ True) is True
# But only if both are Bools
assert (False | 1) is not True
assert (0 | True) is not True
assert (False & 1) is not False
assert (0 & True) is not False
assert (False ^ 1) is not True
assert (0 ^ True) is not True

# Check that the same works with __XXX__ methods
assert False.__or__(0) is not False
assert False.__or__(False) is False
assert False.__ror__(0) is not False
assert False.__ror__(False) is False
assert False.__and__(0) is not False
assert False.__and__(False) is False
assert False.__rand__(0) is not False
assert False.__rand__(False) is False
assert False.__xor__(0) is not False
assert False.__xor__(False) is False
assert False.__rxor__(0) is not False
assert False.__rxor__(False) is False
