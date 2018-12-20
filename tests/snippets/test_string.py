import string


assert string.ascii_letters == 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'
assert string.ascii_lowercase == 'abcdefghijklmnopqrstuvwxyz'
assert string.ascii_uppercase == 'ABCDEFGHIJKLMNOPQRSTUVWXYZ'
assert string.digits == '0123456789'
assert string.hexdigits == '0123456789abcdefABCDEF'
assert string.octdigits == '01234567'
assert string.punctuation == '!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~'
# FIXME
#assert string.whitespace == ' \t\n\r\x0b\x0c', string.whitespace
#assert string.printable == '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~ \t\n\r\x0b\x0c'
