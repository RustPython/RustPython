
# Test the unicode support! 👋


ᚴ=2

assert ᚴ*8 == 16

ᚴ="👋"

c = ᚴ*3

assert c == '👋👋👋'
