from testutils import assert_raises

assert "a" == chr(97)
assert "é" == chr(233)
assert "🤡" == chr(129313)

assert_raises(TypeError, chr, _msg='chr() takes exactly one argument (0 given)')
assert_raises(ValueError, chr, 0x110005, _msg='ValueError: chr() arg not in range(0x110000)')
