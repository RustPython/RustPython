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
