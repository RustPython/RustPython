
from testutils import assert_raises, skip_if_unsupported

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
        return d.__or__(other_obj) is NotImplemented

    def perf_test_ror(other_obj):
        d={1:2}
        return d.__ror__(other_obj) is NotImplemented

    test_fct={'__or__':perf_test_or, '__ror__':perf_test_ror}
    others=['FooBar', 42, [36], set([19]), ['aa'], None]
    for tfn,tf in test_fct.items():
        for other in others:
            assert tf(other), f"Failed: dict {tfn}, accepted {other}"

    # __ior__() has different behavior and needs to be tested separately
    d = {1: 2}
    assert_raises(ValueError,
                  lambda: d.__ior__('FooBar'),
                  _msg='dictionary update sequence element #0 has length 1; 2 is required')
    assert_raises(TypeError,
                  lambda: d.__ior__(42),
                  _msg='\'int\' object is not iterable')
    assert_raises(TypeError,
                  lambda: d.__ior__([36]),
                  _msg='cannot convert dictionary update sequence element #0 to a sequence')
    assert_raises(TypeError,
                  lambda: d.__ior__(set([36])),
                  _msg='cannot convert dictionary update sequence element #0 to a sequence')
    res = d.__ior__(['aa'])
    assert res == {1: 2, 'a': 'a'}, f"unexpected result of dict union {res=}"
    assert_raises(TypeError,
                  lambda: d.__ior__(None),
                  _msg='TypeError: \'NoneType\' object is not iterable')


skip_if_unsupported(3, 9, test_dunion_ior0)
skip_if_unsupported(3, 9, test_dunion_or0)
skip_if_unsupported(3, 9, test_dunion_or1)
skip_if_unsupported(3, 9, test_dunion_ror0)
skip_if_unsupported(3, 9, test_dunion_other_types)
