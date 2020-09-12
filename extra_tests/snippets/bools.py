from testutils import assert_raises

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

assert bool() is False
assert bool(1) is True
assert bool({}) is False

assert bool(NotImplemented) is True
assert bool(...) is True

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

assert (True or fake)  # noqa: F821
assert (False or True)
assert not (False or False)
assert ("thing" or 0) == "thing"

assert (True and True)
assert not (False and fake)  # noqa: F821
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

assert True.real == 1
assert True.imag == 0
assert type(True.real) is int
assert type(True.imag) is int

# Check work for sequence and map
assert bool({}) is False
assert bool([]) is False
assert bool(set()) is False

assert bool({"key": "value"}) is True
assert bool([1]) is True
assert bool(set([1,2])) is True

assert repr(True) == "True"

# Check __len__ work
class TestMagicMethodLenZero:
    def __len__(self):
        return 0

class TestMagicMethodLenOne:
    def __len__(self):
        return 1


assert bool(TestMagicMethodLenZero()) is False
assert bool(TestMagicMethodLenOne()) is True


# check __len__ and __bool__
class TestMagicMethodBoolTrueLenFalse:
    def __bool__(self):
        return True

    def __len__(self):
        return 0

class TestMagicMethodBoolFalseLenTrue:
    def __bool__(self):
        return False

    def __len__(self):
        return 1

assert bool(TestMagicMethodBoolTrueLenFalse()) is True
assert bool(TestMagicMethodBoolFalseLenTrue()) is False


# Test magic method throw error
class TestBoolThrowError:
    def __bool__(self):
        return object()

with assert_raises(TypeError):
    bool(TestBoolThrowError())

class TestLenThrowError:
    def __len__(self):
        return object()


with assert_raises(TypeError):
    bool(TestLenThrowError())

# Verify that TypeError occurs when bad things are returned
# from __bool__().  This isn't really a bool test, but
# it's related.
def check(o):
    with assert_raises(TypeError):
        bool(o)

class Foo(object):
    def __bool__(self):
        return self
check(Foo())

class Bar(object):
    def __bool__(self):
        return "Yes"
check(Bar())

class Baz(int):
    def __bool__(self):
        return self
check(Baz())

# __bool__() must return a bool not an int
class Spam(int):
    def __bool__(self):
        return 1
check(Spam())

class Eggs:
    def __len__(self):
        return -1

with assert_raises(ValueError):
    bool(Eggs())

with assert_raises(TypeError):
    bool(TestLenThrowError())
