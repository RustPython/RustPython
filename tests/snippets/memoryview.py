
obj = b"abcde"
a = memoryview(obj)
assert a.obj == obj

assert a[2:3] == b"c"

assert hash(obj) == hash(a)
