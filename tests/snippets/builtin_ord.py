from testutils import assert_raises

assert ord("a") == 97
assert ord("é") == 233
assert ord("🤡") == 129313
assert ord(b'a') == 97
assert ord(bytearray(b'a')) == 97

assert_raises(TypeError, ord, _msg='ord() is called with no argument')
assert_raises(TypeError, ord, "", _msg='ord() is called with an empty string')
assert_raises(TypeError, ord, "ab", _msg='ord() is called with more than one character')
assert_raises(TypeError, ord, b"ab", _msg='ord() expected a character, but string of length 2 found')
assert_raises(TypeError, ord, 1, _msg='ord() expected a string, bytes or bytearray, but found int')
