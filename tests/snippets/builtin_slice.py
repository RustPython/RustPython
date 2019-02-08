
a = []
assert a[:] == []
assert a[:2**100] == []
assert a[-2**100:] == []
assert a[::2**100] == []
assert a[10:20] == []
assert a[-20:-10] == []

b = [1, 2]

assert b[:] == [1, 2]
assert b[:2**100] == [1, 2]
assert b[-2**100:] == [1, 2]
assert b[2**100:] == []
assert b[::2**100] == [1]
assert b[-10:1] == [1]

slice_a = slice(5)
assert slice_a.start is None
assert slice_a.stop == 5
assert slice_a.step is None

slice_b = slice(1, 5)
assert slice_b.start == 1
assert slice_b.stop == 5
assert slice_b.step is None

slice_c = slice(1, 5, 2)
assert slice_c.start == 1
assert slice_c.stop == 5
assert slice_c.step == 2


class SubScript(object):
    def __getitem__(self, item):
        assert type(item) == slice

    def __setitem__(self, key, value):
        assert type(key) == slice


ss = SubScript()
_ = ss[:]
ss[:1] = 1
