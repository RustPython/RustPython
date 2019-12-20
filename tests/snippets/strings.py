from testutils import assert_raises, AssertRaises

assert "".__eq__(1) == NotImplemented
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

assert_raises(TypeError, lambda: str("abc", "utf-8"))
assert str(b"abc", "utf-8") == "abc"
assert str(b"abc", encoding="ascii") == "abc"

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

assert_raises(OverflowError, lambda: 'xy' * 234234234234234234234234234234)

a = 'Hallo'
assert a.lower() == 'hallo'
assert a.upper() == 'HALLO'
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

s = '1 2 3'
assert s.split(' ', 1) == ['1', '2 3']
assert s.rsplit(' ', 1) == ['1 2', '3']

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
assert "{} {}".format(1, 2) == "1 2"
assert "{0} {1}".format(2, 3) == "2 3"
assert "--{:s>4}--".format(1) == "--sss1--"
assert "{keyword} {0}".format(1, keyword=2) == "2 1"
assert "repr() shows quotes: {!r}; str() doesn't: {!s}".format(
    'test1', 'test2'
) == "repr() shows quotes: 'test1'; str() doesn't: test2", 'Output: {!r}, {!s}'.format('test1', 'test2')


class Foo:
    def __str__(self):
        return 'str(Foo)'

    def __repr__(self):
        return 'repr(Foo)'


f = Foo()
assert "{} {!s} {!r} {!a}".format(f, f, f, f) == 'str(Foo) str(Foo) repr(Foo) repr(Foo)'
assert "{foo} {foo!s} {foo!r} {foo!a}".format(foo=f) == 'str(Foo) str(Foo) repr(Foo) repr(Foo)'
# assert '{} {!r} {:10} {!r:10} {foo!r:10} {foo!r} {foo}'.format('txt1', 'txt2', 'txt3', 'txt4', 'txt5', foo='bar')


# Printf-style String formatting
assert "%d %d" % (1, 2) == "1 2"
assert "%*c  " % (3, '❤') == "  ❤  "
assert "%(first)s %(second)s" % {'second': 'World!', 'first': "Hello,"} == "Hello, World!"
assert "%(key())s" % {'key()': 'aaa'}
assert "%s %a %r" % (f, f, f) == "str(Foo) repr(Foo) repr(Foo)"
assert "repr() shows quotes: %r; str() doesn't: %s" % ("test1", "test2") == "repr() shows quotes: 'test1'; str() doesn't: test2"
assert "%f" % (1.2345) == "1.234500"
assert "%+f" % (1.2345) == "+1.234500"
assert "% f" % (1.2345) == " 1.234500"
assert "%f" % (-1.2345) == "-1.234500"
assert "%f" % (1.23456789012) == "1.234568"
assert "%f" % (123) == "123.000000"
assert "%f" % (-123) == "-123.000000"

assert_raises(TypeError, lambda: "My name is %s and I'm %(age)d years old" % ("Foo", 25), _msg='format requires a mapping')
assert_raises(TypeError, lambda: "My name is %(name)s" % "Foo", _msg='format requires a mapping')
assert_raises(ValueError, lambda: "This %(food}s is great!" % {"food": "cookie"}, _msg='incomplete format key')
assert_raises(ValueError, lambda: "My name is %" % "Foo", _msg='incomplete format')

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

ss = ['Hello', '안녕', '👋']
bs = [b'Hello', b'\xec\x95\x88\xeb\x85\x95', b'\xf0\x9f\x91\x8b']

for s, b in zip(ss, bs):
    assert s.encode() == b

for s, b, e in zip(ss, bs, ['u8', 'U8', 'utf-8', 'UTF-8', 'utf_8']):
    assert s.encode(e) == b
    # assert s.encode(encoding=e) == b

# str.isisprintable
assert "".isprintable()
assert " ".isprintable()
assert "abcdefg".isprintable()
assert not "abcdefg\n".isprintable()
assert "ʹ".isprintable()

# test unicode literals
assert "\xac" == "¬"
assert "\u0037" == "7"
assert "\u0040" == "@"
assert "\u0041" == "A"
assert "\u00BE" == "¾"
assert "\u9487" == "钇"
assert "\U0001F609" == "😉"

# test str iter
iterable_str = "123456789"
str_iter = iter(iterable_str)

assert next(str_iter) == "1"
assert next(str_iter) == "2"
assert next(str_iter) == "3"
assert next(str_iter) == "4"
assert next(str_iter) == "5"
assert next(str_iter) == "6"
assert next(str_iter) == "7"
assert next(str_iter) == "8"
assert next(str_iter) == "9"
assert next(str_iter, None) == None
assert_raises(StopIteration, next, str_iter)

str_iter_reversed = reversed(iterable_str)

assert next(str_iter_reversed) == "9"
assert next(str_iter_reversed) == "8"
assert next(str_iter_reversed) == "7"
assert next(str_iter_reversed) == "6"
assert next(str_iter_reversed) == "5"
assert next(str_iter_reversed) == "4"
assert next(str_iter_reversed) == "3"
assert next(str_iter_reversed) == "2"
assert next(str_iter_reversed) == "1"
assert next(str_iter_reversed, None) == None
assert_raises(StopIteration, next, str_iter_reversed)

assert str.__rmod__('%i', 30) == NotImplemented
assert_raises(TypeError, lambda: str.__rmod__(30, '%i'))

# test str index
index_str = 'Rust Python'

assert index_str[0] == 'R'
assert index_str[-1] == 'n'

assert_raises(TypeError, lambda: index_str['a'])

assert chr(9).__repr__() == "'\\t'"
assert chr(99).__repr__() == "'c'"
assert chr(999).__repr__() == "'ϧ'"
assert chr(9999).__repr__() == "'✏'"
assert chr(99999).__repr__() == "'𘚟'"
assert chr(999999).__repr__() == "'\\U000f423f'"

assert "a".__ne__("b")
assert not "a".__ne__("a")
assert not "".__ne__("")
assert "".__ne__(1) == NotImplemented

# check non-cased characters
assert "A_B".isupper()
assert "a_b".islower()
assert "A1".isupper()
assert "1A".isupper()
assert "a1".islower()
assert "1a".islower()
assert "가나다a".islower()
assert "가나다A".isupper()

# test str.format_map()
#
# The following tests were performed in Python 3.7.5:
# Python 3.7.5 (default, Dec 19 2019, 17:11:32)
# [GCC 5.4.0 20160609] on linux

# >>> '{x} {y}'.format_map({'x': 1, 'y': 2})
# '1 2'
assert '{x} {y}'.format_map({'x': 1, 'y': 2}) == '1 2'

# >>> '{x:04d}'.format_map({'x': 1})
# '0001'
assert '{x:04d}'.format_map({'x': 1}) == '0001'

# >>> '{x} {y}'.format_map('foo')
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: string indices must be integers
with AssertRaises(TypeError, None):
    '{x} {y}'.format_map('foo')

# >>> '{x} {y}'.format_map(['foo'])
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: list indices must be integers or slices, not str
with AssertRaises(TypeError, None):
    '{x} {y}'.format_map(['foo'])

# >>> '{x} {y}'.format_map()
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: format_map() takes exactly one argument (0 given)
with AssertRaises(TypeError, msg='TypeError: format_map() takes exactly one argument (0 given)'):
    '{x} {y}'.format_map(),

# >>> '{x} {y}'.format_map('foo', 'bar')
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: format_map() takes exactly one argument (2 given)
with AssertRaises(TypeError, msg='TypeError: format_map() takes exactly one argument (2 given)'):
    '{x} {y}'.format_map('foo', 'bar')

# >>> '{x} {y}'.format_map({'x': 1})
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# KeyError: 'y'
with AssertRaises(KeyError, msg="KeyError: 'y'"):
    '{x} {y}'.format_map({'x': 1})

# >>> '{x} {y}'.format_map({'x': 1, 'z': 2})
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# KeyError: 'y'
with AssertRaises(KeyError, msg="KeyError: 'y'"):
    '{x} {y}'.format_map({'x': 1, 'z': 2})

# >>> '{{literal}}'.format_map('foo')
# '{literal}'
assert '{{literal}}'.format_map('foo') == '{literal}'
