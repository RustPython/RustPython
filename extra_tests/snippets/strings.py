from testutils import assert_raises, AssertRaises, skip_if_unsupported

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
assert 'hello\nhallo\nHallo\n'.splitlines() == ['hello', 'hallo', 'Hallo']
assert 'hello\nhallo\nHallo'.splitlines(keepends=True) == ['hello\n', 'hallo\n', 'Hallo']
assert 'hello\nhallo\nHallo\n'.splitlines(keepends=True) == ['hello\n', 'hallo\n', 'Hallo\n']
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
assert "%e" % 1 == '1.000000e+00'
assert "%e" % 0 == '0.000000e+00'
assert "%e" % 0.1 == '1.000000e-01'
assert "%e" % 10 == '1.000000e+01'
assert "%.10e" % 1.2345678901234567890 == '1.2345678901e+00'
assert '%e' % float('nan') == 'nan'
assert '%e' % float('-nan') == 'nan'
assert '%E' % float('nan') == 'NAN'
assert '%e' % float('inf') == 'inf'
assert '%e' % float('-inf') == '-inf'
assert '%E' % float('inf') == 'INF'
assert "%g" % 123456.78901234567890 == '123457'
assert "%.0g" % 123456.78901234567890 == '1e+05'
assert "%.1g" % 123456.78901234567890 == '1e+05'
assert "%.2g" % 123456.78901234567890 == '1.2e+05'
assert "%g" % 1234567.8901234567890 == '1.23457e+06'
assert "%.0g" % 1234567.8901234567890 == '1e+06'
assert "%.1g" % 1234567.8901234567890 == '1e+06'
assert "%.2g" % 1234567.8901234567890 == '1.2e+06'
assert "%.3g" % 1234567.8901234567890 == '1.23e+06'
assert "%.5g" % 1234567.8901234567890 == '1.2346e+06'
assert "%.6g" % 1234567.8901234567890 == '1.23457e+06'
assert "%.7g" % 1234567.8901234567890 == '1234568'
assert "%.8g" % 1234567.8901234567890 == '1234567.9'
assert "%G" % 123456.78901234567890 == '123457'
assert "%.0G" % 123456.78901234567890 == '1E+05'
assert "%.1G" % 123456.78901234567890 == '1E+05'
assert "%.2G" % 123456.78901234567890 == '1.2E+05'
assert "%G" % 1234567.8901234567890 == '1.23457E+06'
assert "%.0G" % 1234567.8901234567890 == '1E+06'
assert "%.1G" % 1234567.8901234567890 == '1E+06'
assert "%.2G" % 1234567.8901234567890 == '1.2E+06'
assert "%.3G" % 1234567.8901234567890 == '1.23E+06'
assert "%.5G" % 1234567.8901234567890 == '1.2346E+06'
assert "%.6G" % 1234567.8901234567890 == '1.23457E+06'
assert "%.7G" % 1234567.8901234567890 == '1234568'
assert "%.8G" % 1234567.8901234567890 == '1234567.9'
assert '%g' % 0.12345678901234567890 == '0.123457'
assert '%g' % 0.12345678901234567890e-1 == '0.0123457'
assert '%g' % 0.12345678901234567890e-2 == '0.00123457'
assert '%g' % 0.12345678901234567890e-3 == '0.000123457'
assert '%g' % 0.12345678901234567890e-4 == '1.23457e-05'
assert '%g' % 0.12345678901234567890e-5 == '1.23457e-06'
assert '%.6g' % 0.12345678901234567890e-5 == '1.23457e-06'
assert '%.10g' % 0.12345678901234567890e-5 == '1.23456789e-06'
assert '%.20g' % 0.12345678901234567890e-5 == '1.2345678901234567384e-06'
assert '%G' % 0.12345678901234567890 == '0.123457'
assert '%G' % 0.12345678901234567890E-1 == '0.0123457'
assert '%G' % 0.12345678901234567890E-2 == '0.00123457'
assert '%G' % 0.12345678901234567890E-3 == '0.000123457'
assert '%G' % 0.12345678901234567890E-4 == '1.23457E-05'
assert '%G' % 0.12345678901234567890E-5 == '1.23457E-06'
assert '%.6G' % 0.12345678901234567890E-5 == '1.23457E-06'
assert '%.10G' % 0.12345678901234567890E-5 == '1.23456789E-06'
assert '%.20G' % 0.12345678901234567890E-5 == '1.2345678901234567384E-06'
assert '%g' % float('nan') == 'nan'
assert '%g' % float('-nan') == 'nan'
assert '%G' % float('nan') == 'NAN'
assert '%g' % float('inf') == 'inf'
assert '%g' % float('-inf') == '-inf'
assert '%G' % float('inf') == 'INF'
assert "%.0g" % 1.020e-13 == '1e-13'
assert "%.0g" % 1.020e-13 == '1e-13'
assert "%.1g" % 1.020e-13 == '1e-13'
assert "%.2g" % 1.020e-13 == '1e-13'
assert "%.3g" % 1.020e-13 == '1.02e-13'
assert "%.4g" % 1.020e-13 == '1.02e-13'
assert "%.5g" % 1.020e-13 == '1.02e-13'
assert "%.6g" % 1.020e-13 == '1.02e-13'
assert "%.7g" % 1.020e-13 == '1.02e-13'
assert "%g" % 1.020e-13 == '1.02e-13'
assert "%g" % 1.020e-4 == '0.000102'

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
iterable_str = "12345678😉"
str_iter = iter(iterable_str)

assert next(str_iter) == "1"
assert next(str_iter) == "2"
assert next(str_iter) == "3"
assert next(str_iter) == "4"
assert next(str_iter) == "5"
assert next(str_iter) == "6"
assert next(str_iter) == "7"
assert next(str_iter) == "8"
assert next(str_iter) == "😉"
assert next(str_iter, None) == None
assert_raises(StopIteration, next, str_iter)

str_iter_reversed = reversed(iterable_str)

assert next(str_iter_reversed) == "😉"
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

# test formatting float values
assert f'{5:f}' == '5.000000'
assert f'{-5:f}' == '-5.000000'
assert f'{5.0:f}' == '5.000000'
assert f'{-5.0:f}' == '-5.000000'
assert f'{5:.2f}' == '5.00'
assert f'{5.0:.2f}' == '5.00'
assert f'{-5:.2f}' == '-5.00'
assert f'{-5.0:.2f}' == '-5.00'
assert f'{5.0:04f}' == '5.000000'
assert f'{5.1234:+f}' == '+5.123400'
assert f'{5.1234: f}' == ' 5.123400'
assert f'{5.1234:-f}' == '5.123400'
assert f'{-5.1234:-f}' == '-5.123400'
assert f'{1.0:+}' == '+1.0'
assert f'--{1.0:f>4}--' == '--f1.0--'
assert f'--{1.0:f<4}--' == '--1.0f--'
assert f'--{1.0:d^4}--' == '--1.0d--'
assert f'--{1.0:d^5}--' == '--d1.0d--'
assert f'--{1.1:f>6}--' == '--fff1.1--'
assert '{}'.format(float('nan')) == 'nan'
assert '{:f}'.format(float('nan')) == 'nan'
assert '{:f}'.format(float('-nan')) == 'nan'
assert '{:F}'.format(float('nan')) == 'NAN'
assert '{}'.format(float('inf')) == 'inf'
assert '{:f}'.format(float('inf')) == 'inf'
assert '{:f}'.format(float('-inf')) == '-inf'
assert '{:F}'.format(float('inf')) == 'INF'
assert f'{1234567890.1234:,.2f}' == '1,234,567,890.12'
assert f'{1234567890.1234:_.2f}' == '1_234_567_890.12'
with AssertRaises(ValueError, msg="Unknown format code 'd' for object of type 'float'"):
    f'{5.0:04d}'

# Test % formatting
assert f'{10:%}' == '1000.000000%'
assert f'{10.0:%}' == '1000.000000%'
assert f'{10.0:.2%}' == '1000.00%'
assert f'{10.0:.8%}' == '1000.00000000%'
assert f'{-10:%}' == '-1000.000000%'
assert f'{-10.0:%}' == '-1000.000000%'
assert f'{-10.0:.2%}' == '-1000.00%'
assert f'{-10.0:.8%}' == '-1000.00000000%'
assert '{:%}'.format(float('nan')) == 'nan%'
assert '{:.2%}'.format(float('nan')) == 'nan%'
assert '{:%}'.format(float('inf')) == 'inf%'
assert '{:.2%}'.format(float('inf')) == 'inf%'
with AssertRaises(ValueError, msg='Invalid format specifier'):
    f'{10.0:%3}'

# Test e & E formatting
assert '{:e}'.format(10) == '1.000000e+01'
assert '{:.2e}'.format(11) == '1.10e+01'
assert '{:e}'.format(10.0) == '1.000000e+01'
assert '{:e}'.format(-10.0) == '-1.000000e+01'
assert '{:.2e}'.format(10.0) == '1.00e+01'
assert '{:.2e}'.format(-10.0) == '-1.00e+01'
assert '{:.2e}'.format(10.1) == '1.01e+01'
assert '{:.2e}'.format(-10.1) == '-1.01e+01'
assert '{:.2e}'.format(10.001) == '1.00e+01'
assert '{:.4e}'.format(100.234) == '1.0023e+02'
assert '{:.5e}'.format(100.234) == '1.00234e+02'
assert '{:.2E}'.format(10.0) == '1.00E+01'
assert '{:.2E}'.format(-10.0) == '-1.00E+01'
assert '{:e}'.format(float('nan')) == 'nan'
assert '{:e}'.format(float('-nan')) == 'nan'
assert '{:E}'.format(float('nan')) == 'NAN'
assert '{:e}'.format(float('inf')) == 'inf'
assert '{:e}'.format(float('-inf')) == '-inf'
assert '{:E}'.format(float('inf')) == 'INF'

# Test g & G formatting
assert '{:g}'.format(10.0) == '10'
assert '{:g}'.format(100000.0) == '100000'
assert '{:g}'.format(123456.78901234567890) == '123457'
assert '{:.0g}'.format(123456.78901234567890) == '1e+05'
assert '{:.1g}'.format(123456.78901234567890) == '1e+05'
assert '{:.2g}'.format(123456.78901234567890) == '1.2e+05'
assert '{:g}'.format(1234567.8901234567890) == '1.23457e+06'
assert '{:.0g}'.format(1234567.8901234567890) == '1e+06'
assert '{:.1g}'.format(1234567.8901234567890) == '1e+06'
assert '{:.2g}'.format(1234567.8901234567890) == '1.2e+06'
assert '{:.3g}'.format(1234567.8901234567890) == '1.23e+06'
assert '{:.5g}'.format(1234567.8901234567890) == '1.2346e+06'
assert '{:.6g}'.format(1234567.8901234567890) == '1.23457e+06'
assert '{:.7g}'.format(1234567.8901234567890) == '1234568'
assert '{:.8g}'.format(1234567.8901234567890) == '1234567.9'
assert '{:G}'.format(123456.78901234567890) == '123457'
assert '{:.0G}'.format(123456.78901234567890) == '1E+05'
assert '{:.1G}'.format(123456.78901234567890) == '1E+05'
assert '{:.2G}'.format(123456.78901234567890) == '1.2E+05'
assert '{:G}'.format(1234567.8901234567890) == '1.23457E+06'
assert '{:.0G}'.format(1234567.8901234567890) == '1E+06'
assert '{:.1G}'.format(1234567.8901234567890) == '1E+06'
assert '{:.2G}'.format(1234567.8901234567890) == '1.2E+06'
assert '{:.3G}'.format(1234567.8901234567890) == '1.23E+06'
assert '{:.5G}'.format(1234567.8901234567890) == '1.2346E+06'
assert '{:.6G}'.format(1234567.8901234567890) == '1.23457E+06'
assert '{:.7G}'.format(1234567.8901234567890) == '1234568'
assert '{:.8G}'.format(1234567.8901234567890) == '1234567.9'
assert '{:g}'.format(0.12345678901234567890) == '0.123457'
assert '{:g}'.format(0.12345678901234567890e-1) == '0.0123457'
assert '{:g}'.format(0.12345678901234567890e-2) == '0.00123457'
assert '{:g}'.format(0.12345678901234567890e-3) == '0.000123457'
assert '{:g}'.format(0.12345678901234567890e-4) == '1.23457e-05'
assert '{:g}'.format(0.12345678901234567890e-5) == '1.23457e-06'
assert '{:.6g}'.format(0.12345678901234567890e-5) == '1.23457e-06'
assert '{:.10g}'.format(0.12345678901234567890e-5) == '1.23456789e-06'
assert '{:.20g}'.format(0.12345678901234567890e-5) == '1.2345678901234567384e-06'
assert '{:G}'.format(0.12345678901234567890) == '0.123457'
assert '{:G}'.format(0.12345678901234567890E-1) == '0.0123457'
assert '{:G}'.format(0.12345678901234567890E-2) == '0.00123457'
assert '{:G}'.format(0.12345678901234567890E-3) == '0.000123457'
assert '{:G}'.format(0.12345678901234567890E-4) == '1.23457E-05'
assert '{:G}'.format(0.12345678901234567890E-5) == '1.23457E-06'
assert '{:.6G}'.format(0.12345678901234567890E-5) == '1.23457E-06'
assert '{:.10G}'.format(0.12345678901234567890E-5) == '1.23456789E-06'
assert '{:.20G}'.format(0.12345678901234567890E-5) == '1.2345678901234567384E-06'
assert '{:g}'.format(float('nan')) == 'nan'
assert '{:g}'.format(float('-nan')) == 'nan'
assert '{:G}'.format(float('nan')) == 'NAN'
assert '{:g}'.format(float('inf')) == 'inf'
assert '{:g}'.format(float('-inf')) == '-inf'
assert '{:G}'.format(float('inf')) == 'INF'
assert '{:.0g}'.format(1.020e-13) == '1e-13'
assert '{:.0g}'.format(1.020e-13) == '1e-13'
assert '{:.1g}'.format(1.020e-13) == '1e-13'
assert '{:.2g}'.format(1.020e-13) == '1e-13'
assert '{:.3g}'.format(1.020e-13) == '1.02e-13'
assert '{:.4g}'.format(1.020e-13) == '1.02e-13'
assert '{:.5g}'.format(1.020e-13) == '1.02e-13'
assert '{:.6g}'.format(1.020e-13) == '1.02e-13'
assert '{:.7g}'.format(1.020e-13) == '1.02e-13'
assert '{:g}'.format(1.020e-13) == '1.02e-13'
assert "{:g}".format(1.020e-4) == '0.000102'

# remove*fix test
def test_removeprefix():
    s = 'foobarfoo'
    s_ref='foobarfoo'
    assert s.removeprefix('f') == s_ref[1:]
    assert s.removeprefix('fo') == s_ref[2:]
    assert s.removeprefix('foo') == s_ref[3:]

    assert s.removeprefix('') == s_ref
    assert s.removeprefix('bar') == s_ref
    assert s.removeprefix('lol') == s_ref
    assert s.removeprefix('_foo') == s_ref
    assert s.removeprefix('-foo') == s_ref
    assert s.removeprefix('afoo') == s_ref
    assert s.removeprefix('*foo') == s_ref

    assert s==s_ref, 'undefined test fail'

    s_uc = '😱foobarfoo🖖'
    s_ref_uc = '😱foobarfoo🖖'
    assert s_uc.removeprefix('😱') == s_ref_uc[1:]
    assert s_uc.removeprefix('😱fo') == s_ref_uc[3:]
    assert s_uc.removeprefix('😱foo') == s_ref_uc[4:]
    
    assert s_uc.removeprefix('🖖') == s_ref_uc
    assert s_uc.removeprefix('foo') == s_ref_uc
    assert s_uc.removeprefix(' ') == s_ref_uc
    assert s_uc.removeprefix('_😱') == s_ref_uc
    assert s_uc.removeprefix(' 😱') == s_ref_uc
    assert s_uc.removeprefix('-😱') == s_ref_uc
    assert s_uc.removeprefix('#😱') == s_ref_uc

def test_removeprefix_types():
    s='0123456'
    s_ref='0123456'
    others=[0,['012']]
    found=False
    for o in others:
        try:
            s.removeprefix(o)
        except:
            found=True

        assert found, f'Removeprefix accepts other type: {type(o)}: {o=}'

def test_removesuffix():
    s='foobarfoo'
    s_ref='foobarfoo'
    assert s.removesuffix('o') == s_ref[:-1]
    assert s.removesuffix('oo') == s_ref[:-2]
    assert s.removesuffix('foo') == s_ref[:-3]

    assert s.removesuffix('') == s_ref
    assert s.removesuffix('bar') == s_ref
    assert s.removesuffix('lol') == s_ref
    assert s.removesuffix('foo_') == s_ref
    assert s.removesuffix('foo-') == s_ref
    assert s.removesuffix('foo*') == s_ref
    assert s.removesuffix('fooa') == s_ref

    assert s==s_ref, 'undefined test fail'

    s_uc = '😱foobarfoo🖖'
    s_ref_uc = '😱foobarfoo🖖'
    assert s_uc.removesuffix('🖖') == s_ref_uc[:-1]
    assert s_uc.removesuffix('oo🖖') == s_ref_uc[:-3]
    assert s_uc.removesuffix('foo🖖') == s_ref_uc[:-4]
    
    assert s_uc.removesuffix('😱') == s_ref_uc
    assert s_uc.removesuffix('foo') == s_ref_uc
    assert s_uc.removesuffix(' ') == s_ref_uc
    assert s_uc.removesuffix('🖖_') == s_ref_uc
    assert s_uc.removesuffix('🖖 ') == s_ref_uc
    assert s_uc.removesuffix('🖖-') == s_ref_uc
    assert s_uc.removesuffix('🖖#') == s_ref_uc

def test_removesuffix_types():
    s='0123456'
    s_ref='0123456'
    others=[0,6,['6']]
    found=False
    for o in others:
        try:
            s.removesuffix(o)
        except:
            found=True

        assert found, f'Removesuffix accepts other type: {type(o)}: {o=}'

skip_if_unsupported(3,9,test_removeprefix)
skip_if_unsupported(3,9,test_removeprefix_types)
skip_if_unsupported(3,9,test_removesuffix)
skip_if_unsupported(3,9,test_removesuffix_types)
