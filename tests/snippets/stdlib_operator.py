import _operator

assert _operator._compare_digest("abcdef", "abcdef")
assert not _operator._compare_digest("abcdef", "abc")
assert not _operator._compare_digest("abc", "abcdef")
