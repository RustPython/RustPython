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

a = object()
slice_d = slice(a, "v", 1.0)
assert slice_d.start is a
assert slice_d.stop == "v"
assert slice_d.step == 1.0


class SubScript(object):
    def __getitem__(self, item):
        assert type(item) == slice

    def __setitem__(self, key, value):
        assert type(key) == slice


ss = SubScript()
_ = ss[:]
ss[:1] = 1


class CustomIndex:
    def __init__(self, x):
        self.x = x

    def __index__(self):
        return self.x


assert c[CustomIndex(1):CustomIndex(3)] == [1, 2]
assert d[CustomIndex(1):CustomIndex(3)] == "23"


def test_all_slices():
    """
    test all possible slices except big number
    """

    mod = __import__('cpython_generated_slices')

    ll = mod.LL
    start = mod.START
    end = mod.END
    step = mod.STEP
    slices_res = mod.SLICES_RES

    count = 0
    failures = []
    for s in start:
        for e in end:
            for t in step:
                lhs = ll[s:e:t]
                try:
                    assert lhs == slices_res[count]
                except AssertionError:
                    failures.append(
                        "start: {} ,stop: {}, step {}. Expected: {}, found: {}".format(
                            s, e, t, lhs, slices_res[count]
                        )
                    )
                count += 1

    if failures:
        for f in failures:
            print(f)
        print(len(failures), "slices failed")


test_all_slices()
