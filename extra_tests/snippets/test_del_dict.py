# Test that del obj.__dict__ works and lazy creation happens
class C:
    pass


obj = C()
obj.x = 42

# Delete __dict__
del obj.__dict__

# After deletion, accessing __dict__ should return a new empty dict
d = obj.__dict__
assert isinstance(d, dict)
assert len(d) == 0

# Old attribute should be gone
try:
    obj.x
    assert False, "AttributeError expected"
except AttributeError:
    pass

# Function __dict__ deletion should fail
def f():
    pass

try:
    del f.__dict__
    assert False, "TypeError expected for function dict deletion"
except TypeError:
    pass

# functools.partial __dict__ deletion should fail
import functools

p = functools.partial(f)
try:
    del p.__dict__
    assert False, "TypeError expected for partial dict deletion"
except TypeError:
    pass

print("OK")
