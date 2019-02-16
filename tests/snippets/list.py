x = [1, 2, 3]
assert x[0] == 1
assert x[1] == 2
# assert x[7]

y = [2, *x]
assert y == [2, 1, 2, 3]

y.extend(x)
assert y == [2, 1, 2, 3, 1, 2, 3]

assert x * 0 == [], "list __mul__ by 0 failed"
assert x * -1 == [], "list __mul__ by -1 failed"
assert x * 2 == [1, 2, 3, 1, 2, 3], "list __mul__ by 2 failed"

# index()
assert ['a', 'b', 'c'].index('b') == 1
assert [5, 6, 7].index(7) == 2
try:
    ['a', 'b', 'c'].index('z')
except ValueError:
    pass
else:
    assert False, "ValueError was not raised"

x = [[1,0,-3], 'a', 1]
y = [[3,2,1], 'z', 2]
assert x < y, "list __lt__ failed"

x = [5, 13, 31]
y = [1, 10, 29]
assert x > y, "list __gt__ failed"


assert [1,2,'a'].pop() == 'a', "list pop failed"
try:
    [].pop()
except IndexError:
    pass
else:
    assert False, "IndexError was not raised"

recursive = []
recursive.append(recursive)
assert repr(recursive) == "[[...]]"

# insert()
x = ['a', 'b', 'c']
x.insert(0, 'z') # insert is in-place, no return value
assert x == ['z', 'a', 'b', 'c']

x = ['a', 'b', 'c']
x.insert(100, 'z')
assert x == ['a', 'b', 'c', 'z']

x = ['a', 'b', 'c']
x.insert(-1, 'z')
assert x == ['a', 'b', 'z', 'c']

x = ['a', 'b', 'c']
x.insert(-100, 'z')
assert x == ['z', 'a', 'b', 'c']

try:
    x.insert(100000000000000000000, 'z')
except OverflowError:
    pass
else:
    assert False, "OverflowError was not raised"

x = [[], 2, {}]
y = x.copy()
assert x is not y
assert x == y
assert all(a is b for a, b in zip(x, y))
y.append(4)
assert x != y

a = [1, 2, 3]
assert len(a) == 3
a.remove(1)
assert len(a) == 2
assert not 1 in a

try:
    a.remove(10)
except ValueError:
    pass
else:
    assert False, "Remove not exist element should raise ValueError"

x = [1]
x.append(x)
assert x in x
assert x.index(x) == 1
assert x.count(x) == 1

class Foo(object):
    def __eq__(self, x):
        return False

foo = Foo()
foo1 = Foo()
x = [1, foo, 2, foo, []]
assert foo in x
assert 2 in x
assert x.index(foo) == 1
assert x.count(foo) == 2
assert x.index(2) == 2
assert [] in x
assert x.index([]) == 4
assert foo1 not in x
