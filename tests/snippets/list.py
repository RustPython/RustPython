from testutils import assert_raises

x = [1, 2, 3]
assert x[0] == 1
assert x[1] == 2
# assert x[7]

y = [2, *x]
assert y == [2, 1, 2, 3]

y.extend(x)
assert y == [2, 1, 2, 3, 1, 2, 3]

a = []
a.extend((1,2,3,4))
assert a == [1, 2, 3, 4]

a.extend('abcdefg')
assert a == [1, 2, 3, 4, 'a', 'b', 'c', 'd', 'e', 'f', 'g']

a.extend(range(10))
assert a == [1, 2, 3, 4, 'a', 'b', 'c', 'd', 'e', 'f', 'g', 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

a = []
a.extend({1,2,3,4})
assert a == [1, 2, 3, 4]

a.extend({'a': 1, 'b': 2, 'z': 51})
assert a == [1, 2, 3, 4, 'a', 'b', 'z']

class Iter:
    def __iter__(self):
        yield 12
        yield 28

a.extend(Iter())
assert a == [1, 2, 3, 4, 'a', 'b', 'z', 12, 28]

a.extend(bytes(b'hello world'))
assert a == [1, 2, 3, 4, 'a', 'b', 'z', 12, 28, 104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]

class Next:
    def __next__(self):
        yield 12
        yield 28

assert_raises(TypeError, lambda: [].extend(3))
assert_raises(TypeError, lambda: [].extend(slice(0, 10, 1)))

assert x * 0 == [], "list __mul__ by 0 failed"
assert x * -1 == [], "list __mul__ by -1 failed"
assert x * 2 == [1, 2, 3, 1, 2, 3], "list __mul__ by 2 failed"
y = x
x *= 2
assert y is x
assert x == [1, 2, 3] * 2

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

assert_raises(ValueError, lambda: a.remove(10), _msg='Remove not exist element')

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
   assert_raises(ZeroDivisionError, sorted, lst, key=lambda x: 1/x)
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

# __setitem__

# simple index
x = [1, 2, 3, 4, 5]
x[0] = 'a'
assert x == ['a', 2, 3, 4, 5]
x[-1] = 'b'
assert x == ['a', 2, 3, 4, 'b']
# make sure refrences are assigned correctly
y = []
x[1] = y
y.append(100)
assert x[1] == y
assert x[1] == [100]

#index bounds
def set_index_out_of_bounds_high():
  x = [0, 1, 2, 3, 4]
  x[5] = 'a'

def set_index_out_of_bounds_low():
  x = [0, 1, 2, 3, 4]
  x[-6] = 'a'

assert_raises(IndexError, set_index_out_of_bounds_high)
assert_raises(IndexError, set_index_out_of_bounds_low)

# non stepped slice index
a = list(range(10))
x = a[:]
y = a[:]
assert x == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
# replace whole list
x[:] = ['a', 'b', 'c']
y[::1] = ['a', 'b', 'c']
assert x == ['a', 'b', 'c']
assert x == y
# splice list start
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
x[:1] = ['a', 'b', 'c']
y[0:1] = ['a', 'b', 'c']
z[:1:1] = ['a', 'b', 'c']
zz[0:1:1] = ['a', 'b', 'c']
assert x == ['a', 'b', 'c', 1, 2, 3, 4, 5, 6, 7, 8, 9]
assert x == y
assert x == z
assert x == zz
# splice list end
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
x[5:] = ['a', 'b', 'c']
y[5::1] = ['a', 'b', 'c']
z[5:10] = ['a', 'b', 'c']
zz[5:10:1] = ['a', 'b', 'c']
assert x == [0, 1, 2, 3, 4, 'a', 'b', 'c']
assert x == y
assert x == z
assert x == zz
# insert sec
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
x[1:1] = ['a', 'b', 'c']
y[1:0] = ['a', 'b', 'c']
z[1:1:1] = ['a', 'b', 'c']
zz[1:0:1] = ['a', 'b', 'c']
assert x == [0, 'a', 'b', 'c', 1, 2, 3, 4, 5, 6, 7, 8, 9]
assert x == y
assert x == z
assert x == zz
# same but negative indexes?
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
x[-1:-1] = ['a', 'b', 'c']
y[-1:9] = ['a', 'b', 'c']
z[-1:-1:1] = ['a', 'b', 'c']
zz[-1:9:1] = ['a', 'b', 'c']
assert x == [0, 1, 2, 3, 4, 5, 6, 7, 8, 'a', 'b', 'c', 9]
assert x == y
assert x == z
assert x == zz
# splice mid
x = a[:]
y = a[:]
x[3:5] = ['a', 'b', 'c', 'd', 'e']
y[3:5:1] = ['a', 'b', 'c', 'd', 'e']
assert x == [0, 1, 2, 'a', 'b', 'c', 'd', 'e', 5, 6, 7, 8, 9]
assert x == y
x = a[:]
x[3:5] = ['a']
assert x == [0, 1, 2, 'a', 5, 6, 7, 8, 9]
# assign empty to non stepped empty slice does nothing
x = a[:]
y = a[:]
x[5:2] = []
y[5:2:1] = []
assert x == a
assert y == a
# assign empty to non stepped slice removes elems
x = a[:]
y = a[:]
x[2:8] = []
y[2:8:1] = []
assert x == [0, 1, 8, 9]
assert x == y
# make sure refrences are assigned correctly
yy = []
x = a[:]
y = a[:]
x[3:5] = ['a', 'b', 'c', 'd', yy]
y[3:5:1] = ['a', 'b', 'c', 'd', yy]
assert x == [0, 1, 2, 'a', 'b', 'c', 'd', [], 5, 6, 7, 8, 9]
assert x == y
yy.append(100)
assert x == [0, 1, 2, 'a', 'b', 'c', 'd', [100], 5, 6, 7, 8, 9]
assert x == y
assert x[7] == yy
assert x[7] == [100]
assert y[7] == yy
assert y[7] == [100]

# no zero step
def no_zero_step_set():
  x = [1, 2, 3, 4, 5]
  x[0:4:0] = [11, 12, 13, 14, 15]
assert_raises(ValueError, no_zero_step_set)

# stepped slice index
# forward slice
x = a[:]
x[2:8:2] = ['a', 'b', 'c']
assert x == [0, 1, 'a', 3, 'b', 5, 'c', 7, 8, 9]
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
c = ['a', 'b', 'c', 'd', 'e']
x[::2] = c
y[-10::2] = c
z[0:10:2] = c
zz[-13:13:2] = c # slice indexes will be truncated to bounds
assert x == ['a', 1, 'b', 3, 'c', 5, 'd', 7, 'e', 9]
assert x == y
assert x == z
assert x == zz
# backward slice
x = a[:]
x[8:2:-2] = ['a', 'b', 'c']
assert x == [0, 1, 2, 3, 'c', 5, 'b', 7, 'a', 9]
x = a[:]
y = a[:]
z = a[:]
zz = a[:]
c =  ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j']
x[::-1] = c
y[9:-11:-1] = c
z[9::-1] = c
zz[11:-13:-1] = c # slice indexes will be truncated to bounds
assert x == ['j', 'i', 'h', 'g', 'f', 'e', 'd', 'c', 'b', 'a']
assert x == y
assert x == z
assert x == zz
# step size bigger than len
x = a[:]
x[::200] = ['a']
assert x == ['a', 1, 2, 3, 4, 5, 6, 7, 8, 9]
x = a[:]
x[5::200] = ['a']
assert x == [0, 1, 2, 3, 4, 'a', 6, 7, 8, 9]

# bad stepped slices
def stepped_slice_assign_too_big():
  x = [0, 1, 2, 3, 4]
  x[::2] = ['a', 'b', 'c', 'd']

assert_raises(ValueError, stepped_slice_assign_too_big)

def stepped_slice_assign_too_small():
  x = [0, 1, 2, 3, 4]
  x[::2] = ['a', 'b']

assert_raises(ValueError, stepped_slice_assign_too_small)

# must assign iter t0 slice
def must_assign_iter_to_slice():
  x = [0, 1, 2, 3, 4]
  x[::2] = 42

assert_raises(TypeError, must_assign_iter_to_slice)

# other iterables?
a = list(range(10))

# string
x = a[:]
x[3:8] = "abcdefghi"
assert x == [0, 1, 2, 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 8, 9]

# tuple
x = a[:]
x[3:8] = (11, 12, 13, 14, 15)
assert x == [0, 1, 2, 11, 12, 13, 14, 15, 8, 9]

# class
# __next__
class CIterNext:
  def __init__(self, sec=(1, 2, 3)):
    self.sec = sec
    self.index = 0
  def __iter__(self):
    return self
  def __next__(self):
    if self.index >= len(self.sec):
      raise StopIteration
    v = self.sec[self.index]
    self.index += 1
    return v

x = list(range(10))
x[3:8] = CIterNext()
assert x == [0, 1, 2, 1, 2, 3, 8, 9]

# __iter__ yield
class CIter:
  def __init__(self, sec=(1, 2, 3)):
    self.sec = sec
  def __iter__(self):
    for n in self.sec:
      yield n

x = list(range(10))
x[3:8] = CIter()
assert x == [0, 1, 2, 1, 2, 3, 8, 9]

# __getitem but no __iter__ sequence
class CGetItem:
  def __init__(self, sec=(1, 2, 3)):
    self.sec = sec
  def __getitem__(self, sub):
    return self.sec[sub]

x = list(range(10))
x[3:8] = CGetItem()
assert x == [0, 1, 2, 1, 2, 3, 8, 9]

# iter raises error
class CIterError:
  def __iter__(self):
    for i in range(10):
      if i > 5:
        raise RuntimeError
      yield i

def bad_iter_assign():
  x = list(range(10))
  x[3:8] = CIterError()

assert_raises(RuntimeError, bad_iter_assign)

# slice assign when step or stop is -1
a = list(range(10))
x = a[:]
x[-1:-5:-1] = ['a', 'b', 'c', 'd']
assert x == [0, 1, 2, 3, 4, 5, 'd', 'c', 'b', 'a']
x = a[:]
x[-5:-1:-1] = []
assert x == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

# is step != 1 and start or stop of slice == -1
x = list(range(10))
del x[-1:-5:-1]
assert x == [0, 1, 2, 3, 4, 5]
x = list(range(10))
del x[-5:-1:-1]

assert [1, 2].__ne__([])
assert [2, 1].__ne__([1, 2])
assert not [1, 2].__ne__([1, 2])
assert [1, 2].__ne__(1) == NotImplemented

# list gt, ge, lt, le
assert_raises(TypeError, lambda: [0, []] < [0, 0])
assert_raises(TypeError, lambda: [0, []] <= [0, 0])
assert_raises(TypeError, lambda: [0, []] > [0, 0])
assert_raises(TypeError, lambda: [0, []] >= [0, 0])

assert_raises(TypeError, lambda: [0, 0] < [0, []])
assert_raises(TypeError, lambda: [0, 0] <= [0, []])
assert_raises(TypeError, lambda: [0, 0] > [0, []])
assert_raises(TypeError, lambda: [0, 0] >= [0, []])

assert [0, 0] < [1, -1]
assert [0, 0] < [0, 0, 1]
assert [0, 0] < [0, 0, -1]
assert [0, 0] <= [0, 0, -1]
assert not [0, 0, 0, 0] <= [0, -1]

assert [0, 0] > [-1, 1]
assert [0, 0] >= [-1, 1]
assert [0, 0, 0] >= [-1, 1]

assert [0, 0] <= [0, 1]
assert [0, 0] <= [0, 0]
assert [0, 0] <= [0, 0]
assert not [0, 0] > [0, 0]
assert not [0, 0] < [0, 0]

assert not [float('nan'), float('nan')] <= [float('nan'), 1]
assert not [float('nan'), float('nan')] <= [float('nan'), float('nan')]
assert not [float('nan'), float('nan')] >= [float('nan'), float('nan')]
assert not [float('nan'), float('nan')] < [float('nan'), float('nan')]
assert not [float('nan'), float('nan')] > [float('nan'), float('nan')]

assert [float('inf'), float('inf')] >= [float('inf'), 1]
assert [float('inf'), float('inf')] <= [float('inf'), float('inf')]
assert [float('inf'), float('inf')] >= [float('inf'), float('inf')]
assert not [float('inf'), float('inf')] < [float('inf'), float('inf')]
assert not [float('inf'), float('inf')] > [float('inf'), float('inf')]

# list __iadd__
a = []
a += [1, 2, 3]
assert a == [1, 2, 3]

a = []
a += (1,2,3,4)
assert a == [1, 2, 3, 4]

a += 'abcdefg'
assert a == [1, 2, 3, 4, 'a', 'b', 'c', 'd', 'e', 'f', 'g']

a += range(10)
assert a == [1, 2, 3, 4, 'a', 'b', 'c', 'd', 'e', 'f', 'g', 0, 1, 2, 3, 4, 5, 6, 7, 8, 9]

a = []
a += {1,2,3,4}
assert a == [1, 2, 3, 4]

a += {'a': 1, 'b': 2, 'z': 51}
assert a == [1, 2, 3, 4, 'a', 'b', 'z']

class Iter:
    def __iter__(self):
        yield 12
        yield 28

a += Iter()
assert a == [1, 2, 3, 4, 'a', 'b', 'z', 12, 28]

a += bytes(b'hello world')
assert a == [1, 2, 3, 4, 'a', 'b', 'z', 12, 28, 104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]

class Next:
    def __next__(self):
        yield 12
        yield 28

def iadd_int():
    a = []
    a += 3

def iadd_slice():
    a = []
    a += slice(0, 10, 1)

assert_raises(TypeError, iadd_int)
assert_raises(TypeError, iadd_slice)


it = iter([1,2,3,4])
assert it.__length_hint__() == 4
assert next(it) == 1
assert it.__length_hint__() == 3
assert list(it) == [2,3,4]
assert it.__length_hint__() == 0

it = reversed([1,2,3,4])
assert it.__length_hint__() == 4
assert next(it) == 4
assert it.__length_hint__() == 3
assert list(it) == [3,2,1]
assert it.__length_hint__() == 0

a = [*[1, 2], 3, *[4, 5]]
assert a == [1, 2, 3, 4, 5]
