assert hex(16) == '0x10'
assert hex(-16) == '-0x10'

try:
    hex({})
except TypeError:
    pass
else:
    assert False
