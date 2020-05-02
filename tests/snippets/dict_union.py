


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

test_dunion_ior0()
test_dunion_or0()
test_dunion_or1()
test_dunion_ror0()



