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

def test_slice():
    b = b'123456789'
    m = memoryview(b)
    m2 = memoryview(b)
    assert m == m
    assert m == m2
    assert m.tobytes() == b'123456789'
    assert m == b
    assert m[::2].tobytes() == b'13579'
    assert m[::2] == b'13579'
    assert m[1::2].tobytes() == b'2468'
    assert m[::2][1:].tobytes() == b'3579'
    assert m[::2][1:-1].tobytes() == b'357'
    assert m[::2][::2].tobytes() == b'159'
    assert m[::2][1::2].tobytes() == b'37'

test_slice()

def test_resizable():
    b = bytearray(b'123')
    b.append(4)
    m = memoryview(b)
    assert_raises(BufferError, lambda: b.append(5))
    m.release()
    b.append(6)
    m2 = memoryview(b)
    m4 = memoryview(b)
    assert_raises(BufferError, lambda: b.append(5))
    m3 = memoryview(b)
    assert_raises(BufferError, lambda: b.append(5))
    m2.release()
    assert_raises(BufferError, lambda: b.append(5))
    m3.release()
    m4.release()
    b.append(7)

test_resizable()
