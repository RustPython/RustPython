# comp
a = b"abcd"
b = b"ab"
c = b"abcd"

assert a > b
assert a >= b
assert b < a
assert b <= a
assert a == c

# hash not implemented for iterator
# assert hash(iter(a)) == hash(iter(b"abcd"))

assert repr(a) == "b'abcd'"
assert len(a) == 4

assert a + b == b"abcdab"
try:
    b"ab" + "ab"
    assert false
except TypeError:
    assert True
