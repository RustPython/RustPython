from testutils import assert_raises

assert "a" == 'a'
assert """a""" == "a"
assert len(""" " "" " "" """) == 11
assert "\"" == '"'
assert "\"" == """\""""

assert "\n" == """
"""

assert len(""" " \" """) == 5
assert len("é") == 1
assert len("é") == 2
assert len("あ") == 1

assert type("") is str
assert type(b"") is bytes

assert str(1) == "1"
assert str(2.1) == "2.1"
assert str() == ""
assert str("abc") == "abc"

assert repr("a") == "'a'"
assert repr("can't") == '"can\'t"'
assert repr('"won\'t"') == "'\"won\\'t\"'"
assert repr('\n\t') == "'\\n\\t'"

assert str(["a", "b", "can't"]) == "['a', 'b', \"can't\"]"

assert "xy" * 3 == "xyxyxy"
assert "x" * 0 == ""
assert "x" * -1 == ""

assert 3 * "xy" == "xyxyxy"
assert 0 * "x" == ""
assert -1 * "x" == ""

a = 'Hallo'
assert a.lower() == 'hallo'
assert a.upper() == 'HALLO'
assert a.split('al') == ['H', 'lo']
assert a.startswith('H')
assert a.startswith(('H', 1))
assert a.startswith(('A', 'H'))
assert not a.startswith('f')
assert not a.startswith(('A', 'f'))
assert a.endswith('llo')
assert a.endswith(('lo', 1))
assert a.endswith(('A', 'lo'))
assert not a.endswith('on')
assert not a.endswith(('A', 'll'))
assert a.zfill(8) == '000Hallo'
assert a.isalnum()
assert not a.isdigit()
assert not a.isdecimal()
assert not a.isnumeric()
assert a.istitle()
assert a.isalpha()



b = '  hallo  '
assert b.strip() == 'hallo'
assert b.lstrip() == 'hallo  '
assert b.rstrip() == '  hallo'

s = '^*RustPython*^'
assert s.strip('^*') == 'RustPython'
assert s.lstrip('^*') == 'RustPython*^'
assert s.rstrip('^*') == '^*RustPython'

s = 'RustPython'
assert s.ljust(8) == 'RustPython'
assert s.rjust(8) == 'RustPython'
assert s.ljust(12) == 'RustPython  '
assert s.rjust(12) == '  RustPython'
assert s.ljust(12, '_') == 'RustPython__'
assert s.rjust(12, '_') == '__RustPython'
# The fill character must be exactly one character long
assert_raises(TypeError, lambda: s.ljust(12, '__'))
assert_raises(TypeError, lambda: s.rjust(12, '__'))

c = 'hallo'
assert c.capitalize() == 'Hallo'
assert c.center(11, '-') == '---hallo---'
assert ["koki".center(i, "|") for i in range(3, 10)] == [
    "koki",
    "koki",
    "|koki",
    "|koki|",
    "||koki|",
    "||koki||",
    "|||koki||",
]


assert ["kok".center(i, "|") for i in range(2, 10)] == [
    "kok",
    "kok",
    "kok|",
    "|kok|",
    "|kok||",
    "||kok||",
    "||kok|||",
    "|||kok|||",
]


# requires CPython 3.7, and the CI currently runs with 3.6
# assert c.isascii()
assert c.index('a') == 1
assert c.rindex('l') == 3
assert c.find('h') == 0
assert c.rfind('x') == -1
assert c.islower()
assert c.title() == 'Hallo'
assert c.count('l') == 2

assert 'aaa'.count('a') == 3
assert 'aaa'.count('a', 1) == 2
assert 'aaa'.count('a', 1, 2) == 1
assert 'aaa'.count('a', 2, 2) == 0
assert 'aaa'.count('a', 2, 1) == 0

assert '___a__'.find('a') == 3
assert '___a__'.find('a', -10) == 3
assert '___a__'.find('a', -3) == 3
assert '___a__'.find('a', -2) == -1
assert '___a__'.find('a', -1) == -1
assert '___a__'.find('a', 0) == 3
assert '___a__'.find('a', 3) == 3
assert '___a__'.find('a', 4) == -1
assert '___a__'.find('a', 10) == -1
assert '___a__'.rfind('a', 3) == 3
assert '___a__'.index('a', 3) == 3

assert '___a__'.find('a', 0, -10) == -1
assert '___a__'.find('a', 0, -3) == -1
assert '___a__'.find('a', 0, -2) == 3
assert '___a__'.find('a', 0, -1) == 3
assert '___a__'.find('a', 0, 0) == -1
assert '___a__'.find('a', 0, 3) == -1
assert '___a__'.find('a', 0, 4) == 3
assert '___a__'.find('a', 0, 10) == 3

assert '___a__'.find('a', 3, 3) == -1
assert '___a__'.find('a', 3, 4) == 3
assert '___a__'.find('a', 4, 3) == -1

assert 'abcd'.startswith('b', 1)
assert 'abcd'.startswith(('b', 'z'), 1)
assert not 'abcd'.startswith('b', -4)
assert 'abcd'.startswith('b', -3)

assert not 'abcd'.startswith('b', 3, 3)
assert 'abcd'.startswith('', 3, 3)
assert not 'abcd'.startswith('', 4, 3)

assert '   '.isspace()
assert 'hello\nhallo\nHallo'.splitlines() == ['hello', 'hallo', 'Hallo']
assert 'abc\t12345\txyz'.expandtabs() == 'abc     12345   xyz'
assert '-'.join(['1', '2', '3']) == '1-2-3'
assert 'HALLO'.isupper()
assert "hello, my name is".partition("my ") == ('hello, ', 'my ', 'name is')
assert "hello".partition("is") == ('hello', '', '')
assert "hello, my name is".rpartition("is") == ('hello, my name ', 'is', '')
assert "hello".rpartition("is") == ('', '', 'hello')
assert not ''.isdecimal()
assert '123'.isdecimal()
assert not '\u00B2'.isdecimal()

assert not ''.isidentifier()
assert 'python'.isidentifier()
assert '_'.isidentifier()
assert '유니코드'.isidentifier()
assert not '😂'.isidentifier()
assert not '123'.isidentifier()

# String Formatting
assert "{} {}".format(1,2) == "1 2"
assert "{0} {1}".format(2,3) == "2 3"
assert "--{:s>4}--".format(1) == "--sss1--"
assert "{keyword} {0}".format(1, keyword=2) == "2 1"

assert 'a' < 'b'
assert 'a' <= 'b'
assert 'a' <= 'a'
assert 'z' > 'b'
assert 'z' >= 'b'
assert 'a' >= 'a'

# str.translate
assert "abc".translate({97: '🎅', 98: None, 99: "xd"}) == "🎅xd"

# str.maketrans
assert str.maketrans({"a": "abc", "b": None, "c": 33}) == {97: "abc", 98: None, 99: 33}
assert str.maketrans("hello", "world", "rust") == {104: 119, 101: 111, 108: 108, 111: 100, 114: None, 117: None, 115: None, 116: None}

def try_mutate_str():
   word = "word"
   word[0] = 'x'

assert_raises(TypeError, try_mutate_str)
