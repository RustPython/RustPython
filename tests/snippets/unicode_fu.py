
# Test the unicode support! 👋


ᚴ=2

assert ᚴ*8 == 16

ᚴ="👋"

c = ᚴ*3

assert c == '👋👋👋'

import unicodedata
assert unicodedata.category('a') == 'Ll'
assert unicodedata.category('A') == 'Lu'
assert unicodedata.name('a') == 'LATIN SMALL LETTER A'
assert unicodedata.lookup('LATIN SMALL LETTER A') == 'a'
assert unicodedata.bidirectional('a') == 'L'
assert unicodedata.normalize('NFC', 'bla') == 'bla'
