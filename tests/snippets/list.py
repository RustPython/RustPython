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


x = [0, 1, 2]
assert x.pop() == 2
assert x == [0, 1]

def test_pop(lst, idx, value, new_lst):
    assert lst.pop(idx) == value
    assert lst == new_lst
test_pop([0, 1, 2], -1, 2, [0, 1])
test_pop([0, 1, 2], 0, 0, [1, 2])
test_pop([0, 1, 2], 1, 1, [0, 2])
test_pop([0, 1, 2], 2, 2, [0, 1])
assert_raises(IndexError, lambda: [].pop())
assert_raises(IndexError, lambda: [].pop(0))
assert_raises(IndexError, lambda: [].pop(-1))
assert_raises(IndexError, lambda: [0].pop(1))
assert_raises(IndexError, lambda: [0].pop(-2))

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

for size in [1, 2, 3, 4, 5, 8, 10, 100, 1000]:
   lst = list(range(size))
   orig = lst[:]
   lst.sort()
   assert lst == orig
   assert sorted(lst) == orig
   assert_raises(ZeroDivisionError, lambda: sorted(lst, key=lambda x: 1/x))
   lst.reverse()
   assert sorted(lst) == orig
   assert sorted(lst, reverse=True) == lst
   assert sorted(lst, key=lambda x: -x) == lst
   assert sorted(lst, key=lambda x: -x, reverse=True) == orig

assert sorted([(1, 2, 3), (0, 3, 6)]) == [(0, 3, 6), (1, 2, 3)]
assert sorted([(1, 2, 3), (0, 3, 6)], key=lambda x: x[0]) == [(0, 3, 6), (1, 2, 3)]
assert sorted([(1, 2, 3), (0, 3, 6)], key=lambda x: x[1]) == [(1, 2, 3), (0, 3, 6)]
assert sorted([(1, 2), (), (5,)], key=len) == [(), (5,), (1, 2)]

lst = [3, 1, 5, 2, 4]
class C:
  def __init__(self, x): self.x = x
  def __lt__(self, other): return self.x < other.x
lst.sort(key=C)
assert lst == [1, 2, 3, 4, 5]

lst = [3, 1, 5, 2, 4]
class C:
  def __init__(self, x): self.x = x
  def __gt__(self, other): return self.x > other.x
lst.sort(key=C)
assert lst == [1, 2, 3, 4, 5]

lst = [5, 1, 2, 3, 4]
def f(x):
    lst.append(1)
    return x
assert_raises(ValueError, lambda: lst.sort(key=f)) # "list modified during sort"
assert lst == [1, 2, 3, 4, 5]

# __delitem__
x = ['a', 'b', 'c']
del x[0]
assert x == ['b', 'c']

x = ['a', 'b', 'c']
del x[-1]
assert x == ['a', 'b']

x = y = [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15]
del x[2:14:3]
assert x == y
assert x == [1, 2, 4, 5, 7, 8, 11, 12, 14, 15]
assert y == [1, 2, 4, 5, 7, 8, 11, 12, 14, 15]

x = [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15]
del x[-5:]
assert x == [1, 2, 3, 4, 5, 6, 7, 8, 10]

x = list(range(12))
del x[10:2:-2]
assert x == [0,1,2,3,5,7,9,11]

def bad_del_1():
  del ['a', 'b']['a']
assert_raises(TypeError, bad_del_1)

def bad_del_2():
  del ['a', 'b'][2]
assert_raises(IndexError, bad_del_2)
