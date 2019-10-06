import array

from testutils import assert_raises

obj = b"abcde"
a = memoryview(obj)
assert a.obj == obj

assert a[2:3] == b"c"

assert hash(obj) == hash(a)

class A(array.array):
    ...

class B(bytes):
    ...

class C():
    ...

memoryview(bytearray('abcde', encoding='utf-8'))
memoryview(array.array('i', [1, 2, 3]))
memoryview(A('b', [0]))
memoryview(B('abcde', encoding='utf-8'))

assert_raises(TypeError, lambda: memoryview([1, 2, 3]))
assert_raises(TypeError, lambda: memoryview((1, 2, 3)))
assert_raises(TypeError, lambda: memoryview({}))
assert_raises(TypeError, lambda: memoryview('string'))
assert_raises(TypeError, lambda: memoryview(C()))
