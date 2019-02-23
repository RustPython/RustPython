from testutils import assert_raises

assert ord("a") == 97
assert ord("é") == 233
assert ord("🤡") == 129313

assert_raises(TypeError, lambda: ord(), "ord() is called with no argument")
assert_raises(TypeError, lambda: ord(""), "ord() is called with an empty string")
assert_raises(TypeError, lambda: ord("ab"), "ord() is called with more than one character")
