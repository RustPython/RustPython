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
