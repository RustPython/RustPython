
import testutils

def test_dunion_ior0():
    a={1:2,2:3}
    b={3:4,5:6}
    a|=b

    assert a == {1:2,2:3,3:4,5:6}, f"wrong value assigned {a=}"
    assert b == {3:4,5:6}, f"right hand side modified, {b=}"

def test_dunion_or0():
    a={1:2,2:3}
    b={3:4,5:6}
    c=a|b

    assert a == {1:2,2:3}, f"left hand side of non-assignment operator modified {a=}"
    assert b == {3:4,5:6}, f"right hand side of non-assignment operator modified, {b=}"
    assert c == {1:2,2:3, 3:4, 5:6}, f"unexpected result of dict union {c=}"


def test_dunion_or1():
    a={1:2,2:3}
    b={3:4,5:6}
    c=a.__or__(b)

    assert a == {1:2,2:3}, f"left hand side of non-assignment operator modified {a=}"
    assert b == {3:4,5:6}, f"right hand side of non-assignment operator modified, {b=}"
    assert c == {1:2,2:3, 3:4, 5:6}, f"unexpected result of dict union {c=}"


def test_dunion_ror0():
    a={1:2,2:3}
    b={3:4,5:6}
    c=b.__ror__(a)

    assert a == {1:2,2:3}, f"left hand side of non-assignment operator modified {a=}"
    assert b == {3:4,5:6}, f"right hand side of non-assignment operator modified, {b=}"
    assert c == {1:2,2:3, 3:4, 5:6}, f"unexpected result of dict union {c=}"


def test_dunion_other_types():
    def perf_test_or(other_obj):
        d={1:2}
        try:
            d.__or__(other_obj)
        except:
            return True
        return False

    def perf_test_ior(other_obj):
        d={1:2}
        try:
            d.__ior__(other_obj)
        except:
            return True
        return False

    def perf_test_ror(other_obj):
        d={1:2}
        try:
            d.__ror__(other_obj)
        except:
            return True
        return False

    test_fct={'__or__':perf_test_or, '__ror__':perf_test_ror, '__ior__':perf_test_ior}
    others=['FooBar', 42, [36], set([19]), ['aa'], None]
    for tfn,tf in test_fct.items():
        for other in others:
            assert tf(other), f"Failed: dict {tfn}, accepted {other}"




testutils.skip_if_unsupported(3,9,test_dunion_ior0)
testutils.skip_if_unsupported(3,9,test_dunion_or0)
testutils.skip_if_unsupported(3,9,test_dunion_or1)
testutils.skip_if_unsupported(3,9,test_dunion_ror0)
testutils.skip_if_unsupported(3,9,test_dunion_other_types)



