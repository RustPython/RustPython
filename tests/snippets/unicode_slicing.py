def test_slice_bounds(s):
    # End out of range
    assert s[0:100] == s
    assert s[0:-100] == '' 
    # Start out of range
    assert s[100:1] == ''
    # Out of range both sides
    # This is the behaviour in cpython
    # assert s[-100:100] == s

def expect_index_error(s, index):
    try:
        s[index]
    except IndexError:
        pass
    else:
        assert False

unicode_str = "∀∂"
assert unicode_str[0] == "∀"
assert unicode_str[1] == "∂"
assert unicode_str[-1] == "∂"

test_slice_bounds(unicode_str)
expect_index_error(unicode_str, 100)
expect_index_error(unicode_str, -100)

ascii_str = "hello world"
test_slice_bounds(ascii_str)
assert ascii_str[0] == "h"
assert ascii_str[1] == "e"
assert ascii_str[-1] == "d"

