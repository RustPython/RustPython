from testutils import assert_raises

assert "a" == chr(97)
assert "é" == chr(233)
assert "🤡" == chr(129313)

assert_raises(TypeError, lambda: chr(), "chr() takes exactly one argument (0 given)")
assert_raises(ValueError, lambda: chr(0x110005), "ValueError: chr() arg not in range(0x110000)")
