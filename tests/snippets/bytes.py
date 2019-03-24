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

# contains
assert b"ab" in b"abcd"
assert b"cd" in b"abcd"
assert b"abcd" in b"abcd"
assert b"a" in b"abcd"
assert b"d" in b"abcd"
assert b"dc" not in b"abcd"
assert 97 in b"abcd"
assert 150 not in b"abcd"
