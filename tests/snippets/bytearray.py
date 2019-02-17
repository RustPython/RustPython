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

assert bytearray(b'lower').islower()
assert not bytearray(b'Super Friends').islower()

assert bytearray(b' \n\t').isspace()
assert not bytearray(b'\td\n').isspace()

assert bytearray(b'UPPER').isupper()
assert not bytearray(b'tuPpEr').isupper()

assert bytearray(b'Is Title Case').istitle()
assert not bytearray(b'is Not title casE').istitle()
