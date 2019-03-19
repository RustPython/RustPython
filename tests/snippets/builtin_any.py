assert any([1]);
assert not any([]);
assert not any([0,0,0,0]);
assert any([0,0,1,0,0]);
def anything(a):
    return a

class Test:
 def __iter__(self):
   while True:
    yield True

assert any(map(anything, Test()))
