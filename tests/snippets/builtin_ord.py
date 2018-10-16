assert ord("a") == 97
assert ord("é") == 233
assert ord("🤡") == 129313
try:
    ord()
except TypeError:
    pass
else:
    assert False, "TypeError not raised when ord() is called with no argument"

try:
    ord("")
except TypeError:
    pass
else:
    assert False, "TypeError not raised when ord() is called with an empty string"

try:
    ord("ab")
except TypeError:
    pass
else:
    assert False, "TypeError not raised when ord() is called with more than one character"
