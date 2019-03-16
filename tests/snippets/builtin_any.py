def anything(a):
    return a

class Test:
 def __iter__(self):
   while True:
    yield True

assert True == any(map(anything, Test()))
