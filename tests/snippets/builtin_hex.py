from testutils import assert_raises

assert hex(16) == '0x10'
assert hex(-16) == '-0x10'

assert_raises(TypeError, hex, {})  # ord() called with dict
