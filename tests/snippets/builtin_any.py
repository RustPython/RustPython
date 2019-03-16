assert any([1]);
assert not any([]);

def anything(a):
    return a

class Test:
 def __iter__(self):
   while True:
    yield True

assert any(map(anything, Test()))
