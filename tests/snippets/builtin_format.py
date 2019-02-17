assert format(5, "b") == "101"

try:
    format(2, 3)
except TypeError:
    pass
else:
    assert False, "TypeError not raised when format is called with a number"

assert format({}) == "{}"

try:
    format({}, 'b')
except TypeError:
    pass
else:
    assert False, "TypeError not raised when format_spec not empty for dict"
