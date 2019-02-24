from testutils import assert_raises

assert format(5, "b") == "101"

assert_raises(TypeError, lambda: format(2, 3), 'format called with number')

assert format({}) == "{}"

assert_raises(TypeError, lambda: format({}, 'b'), 'format_spec not empty for dict')
