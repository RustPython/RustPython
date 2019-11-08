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

# test unicode indexing, made more tricky by hebrew being a right-to-left language
hebrew_text = "בְּרֵאשִׁית, בָּרָא אֱלֹהִים, אֵת הַשָּׁמַיִם, וְאֵת הָאָרֶץ"
assert len(hebrew_text) == 60
assert len(hebrew_text[:]) == 60
assert hebrew_text[0] == 'ב'
assert hebrew_text[1] == 'ְ'
assert hebrew_text[2] == 'ּ'
assert hebrew_text[3] == 'ר'
assert hebrew_text[4] == 'ֵ'
assert hebrew_text[5] == 'א'
assert hebrew_text[6] == 'ש'
assert hebrew_text[5:10] == 'אשִׁי'
assert len(hebrew_text[5:10]) == 5
assert hebrew_text[-20:50] == 'מַיִם, וְא'
assert len(hebrew_text[-20:50]) == 10
assert hebrew_text[:-30:1] == 'בְּרֵאשִׁית, בָּרָא אֱלֹהִים, '
assert len(hebrew_text[:-30:1]) == 30
assert hebrew_text[10:-30] == 'ת, בָּרָא אֱלֹהִים, '
assert len(hebrew_text[10:-30]) == 20
assert hebrew_text[10:30:3] == 'תבר לִ,'
assert len(hebrew_text[10:30:3]) == 7
assert hebrew_text[10:30:-3] == ''
assert hebrew_text[30:10:-3] == 'אםהֱאּ '
assert len(hebrew_text[30:10:-3]) == 7
assert hebrew_text[30:10:-1] == 'א ,םיִהֹלֱא אָרָּב ,'
assert len(hebrew_text[30:10:-1]) == 20
