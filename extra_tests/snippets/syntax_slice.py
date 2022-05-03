from testutils import assert_raises

a = []
assert a[:] == []
assert a[: 2 ** 100] == []
assert a[-2 ** 100 :] == []
assert a[:: 2 ** 100] == []
assert a[10:20] == []
assert a[-20:-10] == []

b = [1, 2]

assert b[:] == [1, 2]
assert b[slice(None)] == [1, 2]
assert b[: 2 ** 100] == [1, 2]
assert b[-2 ** 100 :] == [1, 2]
assert b[2 ** 100 :] == []
assert b[:: 2 ** 100] == [1]
assert b[-10:1] == [1]
assert b[0:0] == []
assert b[1:0] == []

assert_raises(ValueError, lambda: b[::0], _msg='zero step slice')

assert b[::-1] == [2, 1]
assert b[1::-1] == [2, 1]
assert b[0::-1] == [1]
assert b[0:-5:-1] == [1]
assert b[:0:-1] == [2]
assert b[5:0:-1] == [2]

c = list(range(10))

assert c[9:6:-3] == [9]
assert c[9::-3] == [9, 6, 3, 0]
assert c[9::-4] == [9, 5, 1]
assert c[8 :: -2 ** 100] == [8]

assert c[7:7:-2] == []
assert c[7:8:-2] == []

d = "123456"

assert d[3::-1] == "4321"
assert d[4::-3] == "52"

assert [1, 2, 3, 5, 6][-1:-5:-1] == [6, 5, 3, 2]  # #746
