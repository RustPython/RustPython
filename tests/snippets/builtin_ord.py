from testutils import assert_raises

assert ord("a") == 97
assert ord("é") == 233
assert ord("🤡") == 129313
assert ord(b'a') == 97
assert ord(bytearray(b'a')) == 97

assert_raises(TypeError, lambda: ord(), "ord() is called with no argument")
assert_raises(TypeError, lambda: ord(""), "ord() is called with an empty string")
assert_raises(TypeError, lambda: ord("ab"), "ord() is called with more than one character")
assert_raises(TypeError, lambda: ord(b"ab"), "ord() expected a character, but string of length 2 found")
assert_raises(TypeError, lambda: ord(1), "ord() expected a string, bytes or bytearray, but found int")
