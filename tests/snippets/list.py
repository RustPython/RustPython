from testutils import assert_raises

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
assert_raises(ValueError, lambda: ['a', 'b', 'c'].index('z'))

x = [[1,0,-3], 'a', 1]
y = [[3,2,1], 'z', 2]
assert x < y, "list __lt__ failed"

x = [5, 13, 31]
y = [1, 10, 29]
assert x > y, "list __gt__ failed"


assert [1,2,'a'].pop() == 'a', "list pop failed"
assert_raises(IndexError, lambda: [].pop())

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

assert_raises(OverflowError, lambda: x.insert(100000000000000000000, 'z'))

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

assert_raises(ValueError, lambda: a.remove(10), 'Remove not exist element')

foo = bar = [1]
foo += [2]
assert (foo, bar) == ([1, 2], [1, 2])


x = [1]
x.append(x)
assert x in x
assert x.index(x) == 1
assert x.count(x) == 1
x.remove(x)
assert x not in x

class Foo(object):
    def __eq__(self, x):
        return False

foo = Foo()
foo1 = Foo()
x = [1, foo, 2, foo, []]
assert x == x
assert foo in x
assert 2 in x
assert x.index(foo) == 1
assert x.count(foo) == 2
assert x.index(2) == 2
assert [] in x
assert x.index([]) == 4
assert foo1 not in x
x.remove(foo)
assert x.index(foo) == 2
assert x.count(foo) == 1

x = []
x.append(x)
assert x == x

a = [1, 2, 3]
b = [1, 2, 3]
c = [a, b]
a.append(c)
b.append(c)

assert a == b

assert [foo] == [foo]
