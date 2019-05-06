from testutils import assertRaises


#__getitem__ not implemented yet
#a = bytearray(b'abc')
#assert a[0] == b'a'
#assert a[1] == b'b'

assert len(bytearray([1,2,3])) == 3

assert bytearray(b'1a23').isalnum()
assert not bytearray(b'1%a23').isalnum()

assert bytearray(b'abc').isalpha()
assert not bytearray(b'abc1').isalpha()

# travis doesn't like this
#assert bytearray(b'xyz').isascii()
#assert not bytearray([128, 157, 32]).isascii()

assert bytearray(b'1234567890').isdigit()
assert not bytearray(b'12ab').isdigit()

l = bytearray(b'lower')
assert l.islower()
assert not l.isupper()
assert l.upper().isupper()
assert not bytearray(b'Super Friends').islower()

assert bytearray(b' \n\t').isspace()
assert not bytearray(b'\td\n').isspace()

b = bytearray(b'UPPER')
assert b.isupper()
assert not b.islower()
assert b.lower().islower()
assert not bytearray(b'tuPpEr').isupper()

assert bytearray(b'Is Title Case').istitle()
assert not bytearray(b'is Not title casE').istitle()

a = bytearray(b'abcd')
a.clear()
assert len(a) == 0

with assertRaises(ValueError):
    bytearray([400])

b = bytearray(b'test')
assert len(b) == 4
b.pop()
assert len(b) == 3

c = bytearray([123, 255, 111])
assert len(c) == 3
c.pop()
assert len(c) == 2
c.pop()
c.pop()
with assertRaises(IndexError):
    c.pop()

a = bytearray(b'appen')
assert len(a) == 5
a.append(100)
assert len(a) == 6
assert a.pop() == 100

a = bytearray(b'hey there, what is up?')
assert(a.count(b'h', 3))
assert(a.count(bytearray(b'h'), 3))
assert(a.count(104, 3))
with assertRaises(TypeError):
    a.count('h')
with assertRaises(ValueError):
    a.count(53463)
