import sys

from testutils import AssertRaises, assert_raises, skip_if_unsupported

assert "".__eq__(1) == NotImplemented
assert "a" == "a"
assert """a""" == "a"
assert len(""" " "" " "" """) == 11
assert '"' == '"'
assert '"' == """\""""

assert (
    "\n"
    == """
"""
)

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
assert repr("\n\t") == "'\\n\\t'"

assert str(["a", "b", "can't"]) == "['a', 'b', \"can't\"]"

assert "xy" * 3 == "xyxyxy"
assert "x" * 0 == ""
assert "x" * -1 == ""

assert 3 * "xy" == "xyxyxy"
assert 0 * "x" == ""
assert -1 * "x" == ""

assert_raises(OverflowError, lambda: "xy" * 234234234234234234234234234234)

a = "Hallo"
assert a.lower() == "hallo"
assert a.upper() == "HALLO"
assert a.startswith("H")
assert a.startswith(("H", 1))
assert a.startswith(("A", "H"))
assert not a.startswith("f")
assert not a.startswith(("A", "f"))
assert a.endswith("llo")
assert a.endswith(("lo", 1))
assert a.endswith(("A", "lo"))
assert not a.endswith("on")
assert not a.endswith(("A", "ll"))
assert a.zfill(8) == "000Hallo"
assert "á".zfill(4) == "000á"
assert "🙂".zfill(5) == "0000🙂"
assert "+あ".zfill(5) == "+000あ"
assert a.isalnum()
assert not a.isdigit()
assert not a.isdecimal()
assert not a.isnumeric()
assert a.istitle()
assert "\u1c89".istitle()
assert "Ǳ".title() == "ǲ"
assert a.isalpha()
assert not "\u093f".isalpha()

# Combining characters differ slightly between Rust and Python
assert "\u006e".isalnum()
assert not "\u0303".isalnum()
assert not "\u006e\u0303".isalnum()
assert "\u00f1".isalnum()
assert not "\u0345".isalnum()
assert not "\u093f".isalnum()
for raw in range(0x0363, 0x036F):
    assert not chr(raw).isalnum()

# isdigit is true for exponents
assert "⁰".isdigit()
assert "⁰".isnumeric()
assert not "½".isdigit()
assert "½".isnumeric()
assert not "Ⅻ".isdigit()
assert "Ⅻ".isnumeric()

# isnumeric is broader than Rust's
assert "\u3405".isnumeric()

s = "1 2 3"
assert s.split(" ", 1) == ["1", "2 3"]
assert s.rsplit(" ", 1) == ["1 2", "3"]

b = "  hallo  "
assert b.strip() == "hallo"
assert b.lstrip() == "hallo  "
assert b.rstrip() == "  hallo"

s = "^*RustPython*^"
assert s.strip("^*") == "RustPython"
assert s.lstrip("^*") == "RustPython*^"
assert s.rstrip("^*") == "^*RustPython"
assert " abc ".strip(" é") == "abc"
assert "éabcé".strip("é\ud800") == "abc"


def test_strip_unchanged():
    class StrSubclass(str):
        pass

    for text in ("already clean", "déjà propre", "a\ud800b"):
        for method in (str.strip, str.lstrip, str.rstrip):
            for chars in (None, "#"):
                assert method(text, chars) is text
                result = method(StrSubclass(text), chars)
                assert result == text
                assert type(result) is str


test_strip_unchanged()

s = "RustPython"
assert s.ljust(8) == "RustPython"
assert s.rjust(8) == "RustPython"
assert s.ljust(12) == "RustPython  "
assert s.rjust(12) == "  RustPython"
assert s.ljust(12, "_") == "RustPython__"
assert s.rjust(12, "_") == "__RustPython"
# The fill character must be exactly one character long
assert_raises(TypeError, lambda: s.ljust(12, "__"))
assert_raises(TypeError, lambda: s.rjust(12, "__"))

c = "hallo"
assert c.capitalize() == "Hallo"
assert "ßello".capitalize() == "Ssello"
assert "İstanbul".capitalize() == "İstanbul"
assert "a\u0301bc".capitalize() == "Ábc"
assert "ΣΙΓΜΑ".capitalize() == "Σιγμα"
assert "😀hello".capitalize() == "😀hello"
assert "élan".capitalize() == "Élan"
assert c.center(11, "-") == "---hallo---"
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
assert c.index("a") == 1
assert c.rindex("l") == 3
assert c.find("h") == 0
assert c.rfind("x") == -1
assert c.islower()
assert c.title() == "Hallo"
assert c.count("l") == 2

assert "aaa".count("a") == 3
assert "aaa".count("a", 1) == 2
assert "aaa".count("a", 1, 2) == 1
assert "aaa".count("a", 2, 2) == 0
assert "aaa".count("a", 2, 1) == 0

# An empty needle is counted in characters, not in encoded positions.
assert "".count("") == 1
assert "abc".count("") == 4
assert "가나다".count("") == 4
assert "가나다".count("", 1) == 3
assert "가나다".count("", 1, 2) == 2
assert "가나다".count("", 4, 4) == 0
assert "a\U0001f600b".count("") == 4

assert "___a__".find("a") == 3
assert "___a__".find("a", -10) == 3
assert "___a__".find("a", -3) == 3
assert "___a__".find("a", -2) == -1
assert "___a__".find("a", -1) == -1
assert "___a__".find("a", 0) == 3
assert "___a__".find("a", 3) == 3
assert "___a__".find("a", 4) == -1
assert "___a__".find("a", 10) == -1
assert "___a__".rfind("a", 3) == 3
assert "___a__".index("a", 3) == 3

assert "___a__".find("a", 0, -10) == -1
assert "___a__".find("a", 0, -3) == -1
assert "___a__".find("a", 0, -2) == 3
assert "___a__".find("a", 0, -1) == 3
assert "___a__".find("a", 0, 0) == -1
assert "___a__".find("a", 0, 3) == -1
assert "___a__".find("a", 0, 4) == 3
assert "___a__".find("a", 0, 10) == 3

assert "___a__".find("a", 3, 3) == -1
assert "___a__".find("a", 3, 4) == 3
assert "___a__".find("a", 4, 3) == -1

assert "abcd".startswith("b", 1)
assert "abcd".startswith(("b", "z"), 1)
assert not "abcd".startswith("b", -4)
assert "abcd".startswith("b", -3)

assert not "abcd".startswith("b", 3, 3)
assert "abcd".startswith("", 3, 3)
assert not "abcd".startswith("", 4, 3)

assert "   ".isspace()
assert "hello\nhallo\nHallo".splitlines() == ["hello", "hallo", "Hallo"]
assert "hello\nhallo\nHallo\n".splitlines() == ["hello", "hallo", "Hallo"]
assert "hello\nhallo\nHallo".splitlines(keepends=True) == [
    "hello\n",
    "hallo\n",
    "Hallo",
]
assert "hello\nhallo\nHallo\n".splitlines(keepends=True) == [
    "hello\n",
    "hallo\n",
    "Hallo\n",
]
assert (
    "hello\vhallo\x0cHallo\x1cHELLO\x1dhoho\x1ehaha\x85another\u2028yetanother\u2029last\r\n.".splitlines()
    == [
        "hello",
        "hallo",
        "Hallo",
        "HELLO",
        "hoho",
        "haha",
        "another",
        "yetanother",
        "last",
        ".",
    ]
)
assert (
    "hello\vhallo\x0cHallo\x1cHELLO\x1dhoho\x1ehaha\x85another\u2028yetanother\u2029last\r\n.".splitlines(
        keepends=True
    )
    == [
        "hello\x0b",
        "hallo\x0c",
        "Hallo\x1c",
        "HELLO\x1d",
        "hoho\x1e",
        "haha\x85",
        "another\u2028",
        "yetanother\u2029",
        "last\r\n",
        ".",
    ]
)
assert "abc\t12345\txyz".expandtabs() == "abc     12345   xyz"
assert "-".join(["1", "2", "3"]) == "1-2-3"
assert "-".join(("1", "2")) == "1-2"
assert "-".join([]) == ""
assert "-".join(x for x in "ab") == "a-b"
with assert_raises(TypeError) as cm:
    "-".join(["a", 1])
assert str(cm.exception) == "sequence item 1: expected str instance, int found"


class JoinStr(str):
    pass


assert type("-".join([JoinStr("a")])) is str
assert type("-".join((JoinStr("a"), "b"))) is str
single = "single"
assert "-".join([single]) is single


def join_broken_iterable():
    yield 42
    yield "a"
    raise RuntimeError("producer failed")


with assert_raises(RuntimeError):
    "-".join(join_broken_iterable())

assert "HALLO".isupper()
assert not "123".isupper()
assert not "123".islower()
assert not "\U0001f431".isupper()
assert not "\U0001f431".islower()
assert "\U0001f431 CAT".isupper()
assert "\U0001f431 cat".islower()
if sys.version_info >= (3, 15):
    assert not "\u0295".islower()
    assert not "\u0295".isupper()
    assert not "\u0295".istitle()
assert "\u1c89".isupper()
assert "hello, my name is".partition("my ") == ("hello, ", "my ", "name is")
assert "hello".partition("is") == ("hello", "", "")
assert "hello, my name is".rpartition("is") == ("hello, my name ", "is", "")
assert "hello".rpartition("is") == ("", "", "hello")
assert not "".isdecimal()
assert "123".isdecimal()
assert not "\u00b2".isdecimal()

assert not "".isidentifier()
assert "python".isidentifier()
assert "_".isidentifier()
assert "유니코드".isidentifier()
assert not "😂".isidentifier()
assert not "123".isidentifier()

assert "Σίσυφος".swapcase() == "σΊΣΥΦΟΣ"
assert "\u0295".swapcase() == "\u0295"
assert "\u1c89".swapcase() == "\u1c8a"

# String Formatting
assert "{} {}".format(1, 2) == "1 2"
assert "{0} {1}".format(2, 3) == "2 3"
assert "--{:s>4}--".format(1) == "--sss1--"
assert "{keyword} {0}".format(1, keyword=2) == "2 1"
assert (
    "repr() shows quotes: {!r}; str() doesn't: {!s}".format("test1", "test2")
    == "repr() shows quotes: 'test1'; str() doesn't: test2"
), "Output: {!r}, {!s}".format("test1", "test2")


class Foo:
    def __str__(self):
        return "str(Foo)"

    def __repr__(self):
        return "repr(Foo)"


f = Foo()
assert "{} {!s} {!r} {!a}".format(f, f, f, f) == "str(Foo) str(Foo) repr(Foo) repr(Foo)"
assert (
    "{foo} {foo!s} {foo!r} {foo!a}".format(foo=f)
    == "str(Foo) str(Foo) repr(Foo) repr(Foo)"
)
# assert '{} {!r} {:10} {!r:10} {foo!r:10} {foo!r} {foo}'.format('txt1', 'txt2', 'txt3', 'txt4', 'txt5', foo='bar')


# Printf-style String formatting
assert "%d %d" % (1, 2) == "1 2"
assert "%*c  " % (3, "❤") == "  ❤  "
for precision in (-1, -3, -(2**31)):
    assert "%.*s" % (precision, "🐍hello") == ""
assert "%*.*d" % (-6, -3, 12) == "12    "
assert "%.*f" % (-3, 1.25) == "1"
assert "%.*g" % (-3, 12.5) == "1e+01"
assert_raises(OverflowError, "%.*s".__mod__, (-(2**31) - 1, "abc"))
assert_raises(TypeError, "%.*s".__mod__, (1.0, "abc"))


class PercentIndex:
    def __index__(self):
        return 7


class PercentInt(PercentIndex):
    def __init__(self, value):
        self.value = value

    def __int__(self):
        if isinstance(self.value, Exception):
            raise self.value
        return self.value


class PercentFloat(float):
    __int__ = PercentInt.__int__
    __index__ = PercentIndex.__index__


class PercentFloatIndex(float):
    __index__ = PercentIndex.__index__


class PercentIntSubclass(int):
    def __int__(self):
        raise AssertionError("int subclasses must use their stored value")


for template in ("%d", "%i", "%u", b"%d", b"%i", b"%u"):
    assert template % PercentInt(3) == template % 3
    assert template % PercentIndex() == template % 7
    for value in (None, TypeError("conversion failed")):
        with assert_raises(TypeError) as cm:
            template % PercentInt(value)
        conversion = chr(template[-1]) if isinstance(template, bytes) else template[-1]
        assert str(cm.exception) == (
            f"format argument: %{conversion} requires a real number, not PercentInt"
        )
    assert_raises(
        RuntimeError, template.__mod__, PercentInt(RuntimeError("conversion failed"))
    )
    number = PercentFloat(1.25)
    number.value = 3
    assert template % number == template % 3
    number.value = None
    assert_raises(TypeError, template.__mod__, number)
    number.value = RuntimeError("conversion failed")
    assert_raises(RuntimeError, template.__mod__, number)
    assert template % PercentFloatIndex(1.25) == template % 1
    assert template % PercentIntSubclass(1) == template % 1
assert "%x" % PercentInt(3) == "7"
assert b"%o" % PercentInt(3) == b"7"


def test_percent_character_subclasses():
    for base, value, templates in (
        (str, "ab", ("%c",)),
        (bytes, b"ab", (b"%c", bytearray(b"%c"))),
        (bytearray, b"ab", (b"%c", bytearray(b"%c"))),
    ):

        class Indexable(base):
            def __index__(self):
                raise AssertionError("%c must check the character length first")

        for template in templates:
            assert template % Indexable(value[:1]) == template % value[:1]
            for text in (value[:0], value):
                with assert_raises(TypeError) as expected:
                    template % base(text)
                with assert_raises(TypeError) as actual:
                    template % Indexable(text)
                assert str(actual.exception) == str(expected.exception)
            assert template % PercentIndex() == template % 7


test_percent_character_subclasses()

assert (
    "%(first)s %(second)s" % {"second": "World!", "first": "Hello,"} == "Hello, World!"
)
assert "%(key())s" % {"key()": "aaa"}
assert "%s %a %r" % (f, f, f) == "str(Foo) repr(Foo) repr(Foo)"
assert (
    "repr() shows quotes: %r; str() doesn't: %s" % ("test1", "test2")
    == "repr() shows quotes: 'test1'; str() doesn't: test2"
)
assert "%f" % (1.2345) == "1.234500"
assert "%+f" % (1.2345) == "+1.234500"
assert "% f" % (1.2345) == " 1.234500"
assert "%f" % (-1.2345) == "-1.234500"
assert "%f" % (1.23456789012) == "1.234568"
assert "%f" % (123) == "123.000000"
assert "%f" % (-123) == "-123.000000"
assert "%e" % 1 == "1.000000e+00"
assert "%e" % 0 == "0.000000e+00"
assert "%e" % 0.1 == "1.000000e-01"
assert "%e" % 10 == "1.000000e+01"
assert "%.10e" % 1.2345678901234567890 == "1.2345678901e+00"
assert "%e" % float("nan") == "nan"
assert "%e" % float("-nan") == "nan"
assert "%E" % float("nan") == "NAN"
assert "%e" % float("inf") == "inf"
assert "%e" % float("-inf") == "-inf"
assert "%E" % float("inf") == "INF"
assert "%g" % 123456.78901234567890 == "123457"
assert "%.0g" % 123456.78901234567890 == "1e+05"
assert "%.1g" % 123456.78901234567890 == "1e+05"
assert "%.2g" % 123456.78901234567890 == "1.2e+05"
assert "%g" % 1234567.8901234567890 == "1.23457e+06"
assert "%.0g" % 1234567.8901234567890 == "1e+06"
assert "%.1g" % 1234567.8901234567890 == "1e+06"
assert "%.2g" % 1234567.8901234567890 == "1.2e+06"
assert "%.3g" % 1234567.8901234567890 == "1.23e+06"
assert "%.5g" % 1234567.8901234567890 == "1.2346e+06"
assert "%.6g" % 1234567.8901234567890 == "1.23457e+06"
assert "%.7g" % 1234567.8901234567890 == "1234568"
assert "%.8g" % 1234567.8901234567890 == "1234567.9"
assert "%G" % 123456.78901234567890 == "123457"
assert "%.0G" % 123456.78901234567890 == "1E+05"
assert "%.1G" % 123456.78901234567890 == "1E+05"
assert "%.2G" % 123456.78901234567890 == "1.2E+05"
assert "%G" % 1234567.8901234567890 == "1.23457E+06"
assert "%.0G" % 1234567.8901234567890 == "1E+06"
assert "%.1G" % 1234567.8901234567890 == "1E+06"
assert "%.2G" % 1234567.8901234567890 == "1.2E+06"
assert "%.3G" % 1234567.8901234567890 == "1.23E+06"
assert "%.5G" % 1234567.8901234567890 == "1.2346E+06"
assert "%.6G" % 1234567.8901234567890 == "1.23457E+06"
assert "%.7G" % 1234567.8901234567890 == "1234568"
assert "%.8G" % 1234567.8901234567890 == "1234567.9"
assert "%g" % 0.12345678901234567890 == "0.123457"
assert "%g" % 0.12345678901234567890e-1 == "0.0123457"
assert "%g" % 0.12345678901234567890e-2 == "0.00123457"
assert "%g" % 0.12345678901234567890e-3 == "0.000123457"
assert "%g" % 0.12345678901234567890e-4 == "1.23457e-05"
assert "%g" % 0.12345678901234567890e-5 == "1.23457e-06"
assert "%.6g" % 0.12345678901234567890e-5 == "1.23457e-06"
assert "%.10g" % 0.12345678901234567890e-5 == "1.23456789e-06"
assert "%.20g" % 0.12345678901234567890e-5 == "1.2345678901234567384e-06"
assert "%G" % 0.12345678901234567890 == "0.123457"
assert "%G" % 0.12345678901234567890e-1 == "0.0123457"
assert "%G" % 0.12345678901234567890e-2 == "0.00123457"
assert "%G" % 0.12345678901234567890e-3 == "0.000123457"
assert "%G" % 0.12345678901234567890e-4 == "1.23457E-05"
assert "%G" % 0.12345678901234567890e-5 == "1.23457E-06"
assert "%.6G" % 0.12345678901234567890e-5 == "1.23457E-06"
assert "%.10G" % 0.12345678901234567890e-5 == "1.23456789E-06"
assert "%.20G" % 0.12345678901234567890e-5 == "1.2345678901234567384E-06"
assert "%g" % float("nan") == "nan"
assert "%g" % float("-nan") == "nan"
assert "%G" % float("nan") == "NAN"
assert "%g" % float("inf") == "inf"
assert "%g" % float("-inf") == "-inf"
assert "%G" % float("inf") == "INF"
assert "%.0g" % 1.020e-13 == "1e-13"
assert "%.0g" % 1.020e-13 == "1e-13"
assert "%.1g" % 1.020e-13 == "1e-13"
assert "%.2g" % 1.020e-13 == "1e-13"
assert "%.3g" % 1.020e-13 == "1.02e-13"
assert "%.4g" % 1.020e-13 == "1.02e-13"
assert "%.5g" % 1.020e-13 == "1.02e-13"
assert "%.6g" % 1.020e-13 == "1.02e-13"
assert "%.7g" % 1.020e-13 == "1.02e-13"
assert "%g" % 1.020e-13 == "1.02e-13"
assert "%g" % 1.020e-4 == "0.000102"

assert_raises(
    TypeError,
    lambda: "My name is %s and I'm %(age)d years old" % ("Foo", 25),
    _msg="format requires a mapping",
)
assert_raises(
    TypeError, lambda: "My name is %(name)s" % "Foo", _msg="format requires a mapping"
)
assert_raises(
    ValueError,
    lambda: "This %(food}s is great!" % {"food": "cookie"},
    _msg="incomplete format key",
)
assert_raises(ValueError, lambda: "My name is %" % "Foo", _msg="incomplete format")

assert "a" < "b"
assert "a" <= "b"
assert "a" <= "a"
assert "z" > "b"
assert "z" >= "b"
assert "a" >= "a"

# str.translate
assert "abc".translate({97: "🎅", 98: None, 99: "xd"}) == "🎅xd"
assert "abc".translate({97: 100}) == "dbc"
# Any `LookupError` leaves the character unchanged, not only `KeyError`.
assert "\x00bc".translate(["x"]) == "xbc"
assert "\x00\x01\x05".translate(("z", None)) == "z\x05"


class TranslateMissing(dict):
    def __missing__(self, key):
        return "M"


assert "abc".translate(TranslateMissing({97: "A"})) == "AMM"
assert "".translate(5) == ""
with assert_raises(TypeError):
    "a".translate(5)
with assert_raises(ValueError):
    "a".translate({97: 0x110000})
with assert_raises(TypeError):
    "a".translate({97: 1.5})

# str.maketrans
assert str.maketrans({"a": "abc", "b": None, "c": 33}) == {97: "abc", 98: None, 99: 33}
assert str.maketrans("hello", "world", "rust") == {
    104: 119,
    101: 111,
    108: 108,
    111: 100,
    114: None,
    117: None,
    115: None,
    116: None,
}


def try_mutate_str():
    word = "word"
    word[0] = "x"


assert_raises(TypeError, try_mutate_str)

ss = ["Hello", "안녕", "👋"]
bs = [b"Hello", b"\xec\x95\x88\xeb\x85\x95", b"\xf0\x9f\x91\x8b"]

for s, b in zip(ss, bs):
    assert s.encode() == b

for s, b, e in zip(ss, bs, ["u8", "U8", "utf-8", "UTF-8", "utf_8"]):
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
assert "\u00be" == "¾"
assert "\u9487" == "钇"
assert "\U0001f609" == "😉"

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

assert str.__rmod__("%i", 30) == NotImplemented
assert_raises(TypeError, lambda: str.__rmod__(30, "%i"))

# test str index
index_str = "Rust Python"

assert index_str[0] == "R"
assert index_str[-1] == "n"

assert_raises(TypeError, lambda: index_str["a"])

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
assert not "ジョジョ".isupper()
assert not "ジョジョ".islower()

# test str.format_map()
#
# The following tests were performed in Python 3.7.5:
# Python 3.7.5 (default, Dec 19 2019, 17:11:32)
# [GCC 5.4.0 20160609] on linux

# >>> '{x} {y}'.format_map({'x': 1, 'y': 2})
# '1 2'
assert "{x} {y}".format_map({"x": 1, "y": 2}) == "1 2"

# >>> '{x:04d}'.format_map({'x': 1})
# '0001'
assert "{x:04d}".format_map({"x": 1}) == "0001"

# >>> '{x} {y}'.format_map('foo')
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: string indices must be integers
with AssertRaises(TypeError, None):
    "{x} {y}".format_map("foo")

# >>> '{x} {y}'.format_map(['foo'])
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: list indices must be integers or slices, not str
with AssertRaises(TypeError, None):
    "{x} {y}".format_map(["foo"])

# >>> '{x} {y}'.format_map()
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: format_map() takes exactly one argument (0 given)
with AssertRaises(
    TypeError, msg="TypeError: format_map() takes exactly one argument (0 given)"
):
    ("{x} {y}".format_map(),)

# >>> '{x} {y}'.format_map('foo', 'bar')
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# TypeError: format_map() takes exactly one argument (2 given)
with AssertRaises(
    TypeError, msg="TypeError: format_map() takes exactly one argument (2 given)"
):
    "{x} {y}".format_map("foo", "bar")

# >>> '{x} {y}'.format_map({'x': 1})
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# KeyError: 'y'
with AssertRaises(KeyError, msg="KeyError: 'y'"):
    "{x} {y}".format_map({"x": 1})

# >>> '{x} {y}'.format_map({'x': 1, 'z': 2})
# Traceback (most recent call last):
#   File "<stdin>", line 1, in <module>
# KeyError: 'y'
with AssertRaises(KeyError, msg="KeyError: 'y'"):
    "{x} {y}".format_map({"x": 1, "z": 2})

# >>> '{{literal}}'.format_map('foo')
# '{literal}'
assert "{{literal}}".format_map("foo") == "{literal}"

# test formatting float values
assert f"{5:f}" == "5.000000"
assert f"{-5:f}" == "-5.000000"
assert f"{5.0:f}" == "5.000000"
assert f"{-5.0:f}" == "-5.000000"
assert f"{5:.2f}" == "5.00"
assert f"{5.0:.2f}" == "5.00"
assert f"{-5:.2f}" == "-5.00"
assert f"{-5.0:.2f}" == "-5.00"
assert f"{5.0:04f}" == "5.000000"
assert f"{5.1234:+f}" == "+5.123400"
assert f"{5.1234: f}" == " 5.123400"
assert f"{5.1234:-f}" == "5.123400"
assert f"{-5.1234:-f}" == "-5.123400"
assert f"{1.0:+}" == "+1.0"
assert f"--{1.0:f>4}--" == "--f1.0--"
assert f"--{1.0:f<4}--" == "--1.0f--"
assert f"--{1.0:d^4}--" == "--1.0d--"
assert f"--{1.0:d^5}--" == "--d1.0d--"
assert f"--{1.1:f>6}--" == "--fff1.1--"
assert "{}".format(float("nan")) == "nan"
assert "{:f}".format(float("nan")) == "nan"
assert "{:f}".format(float("-nan")) == "nan"
assert "{:F}".format(float("nan")) == "NAN"
assert "{}".format(float("inf")) == "inf"
assert "{:f}".format(float("inf")) == "inf"
assert "{:f}".format(float("-inf")) == "-inf"
assert "{:F}".format(float("inf")) == "INF"
assert f"{1234567890.1234:,.2f}" == "1,234,567,890.12"
assert f"{1234567890.1234:_.2f}" == "1_234_567_890.12"
with AssertRaises(ValueError, msg="Unknown format code 'd' for object of type 'float'"):
    f"{5.0:04d}"

# Test % formatting
assert f"{10:%}" == "1000.000000%"
assert f"{10.0:%}" == "1000.000000%"
assert f"{10.0:.2%}" == "1000.00%"
assert f"{10.0:.8%}" == "1000.00000000%"
assert f"{-10:%}" == "-1000.000000%"
assert f"{-10.0:%}" == "-1000.000000%"
assert f"{-10.0:.2%}" == "-1000.00%"
assert f"{-10.0:.8%}" == "-1000.00000000%"
assert "{:%}".format(float("nan")) == "nan%"
assert "{:.2%}".format(float("nan")) == "nan%"
assert "{:%}".format(float("inf")) == "inf%"
assert "{:.2%}".format(float("inf")) == "inf%"
with AssertRaises(ValueError, msg="Invalid format specifier"):
    f"{10.0:%3}"

# Test e & E formatting
assert "{:e}".format(10) == "1.000000e+01"
assert "{:.2e}".format(11) == "1.10e+01"
assert "{:e}".format(10.0) == "1.000000e+01"
assert "{:e}".format(-10.0) == "-1.000000e+01"
assert "{:.2e}".format(10.0) == "1.00e+01"
assert "{:.2e}".format(-10.0) == "-1.00e+01"
assert "{:.2e}".format(10.1) == "1.01e+01"
assert "{:.2e}".format(-10.1) == "-1.01e+01"
assert "{:.2e}".format(10.001) == "1.00e+01"
assert "{:.4e}".format(100.234) == "1.0023e+02"
assert "{:.5e}".format(100.234) == "1.00234e+02"
assert "{:.2E}".format(10.0) == "1.00E+01"
assert "{:.2E}".format(-10.0) == "-1.00E+01"
assert "{:e}".format(float("nan")) == "nan"
assert "{:e}".format(float("-nan")) == "nan"
assert "{:E}".format(float("nan")) == "NAN"
assert "{:e}".format(float("inf")) == "inf"
assert "{:e}".format(float("-inf")) == "-inf"
assert "{:E}".format(float("inf")) == "INF"

# Test g & G formatting
assert "{:g}".format(10.0) == "10"
assert "{:g}".format(100000.0) == "100000"
assert "{:g}".format(123456.78901234567890) == "123457"
assert "{:.0g}".format(123456.78901234567890) == "1e+05"
assert "{:.1g}".format(123456.78901234567890) == "1e+05"
assert "{:.2g}".format(123456.78901234567890) == "1.2e+05"
assert "{:g}".format(1234567.8901234567890) == "1.23457e+06"
assert "{:.0g}".format(1234567.8901234567890) == "1e+06"
assert "{:.1g}".format(1234567.8901234567890) == "1e+06"
assert "{:.2g}".format(1234567.8901234567890) == "1.2e+06"
assert "{:.3g}".format(1234567.8901234567890) == "1.23e+06"
assert "{:.5g}".format(1234567.8901234567890) == "1.2346e+06"
assert "{:.6g}".format(1234567.8901234567890) == "1.23457e+06"
assert "{:.7g}".format(1234567.8901234567890) == "1234568"
assert "{:.8g}".format(1234567.8901234567890) == "1234567.9"
assert "{:G}".format(123456.78901234567890) == "123457"
assert "{:.0G}".format(123456.78901234567890) == "1E+05"
assert "{:.1G}".format(123456.78901234567890) == "1E+05"
assert "{:.2G}".format(123456.78901234567890) == "1.2E+05"
assert "{:G}".format(1234567.8901234567890) == "1.23457E+06"
assert "{:.0G}".format(1234567.8901234567890) == "1E+06"
assert "{:.1G}".format(1234567.8901234567890) == "1E+06"
assert "{:.2G}".format(1234567.8901234567890) == "1.2E+06"
assert "{:.3G}".format(1234567.8901234567890) == "1.23E+06"
assert "{:.5G}".format(1234567.8901234567890) == "1.2346E+06"
assert "{:.6G}".format(1234567.8901234567890) == "1.23457E+06"
assert "{:.7G}".format(1234567.8901234567890) == "1234568"
assert "{:.8G}".format(1234567.8901234567890) == "1234567.9"
assert "{:g}".format(0.12345678901234567890) == "0.123457"
assert "{:g}".format(0.12345678901234567890e-1) == "0.0123457"
assert "{:g}".format(0.12345678901234567890e-2) == "0.00123457"
assert "{:g}".format(0.12345678901234567890e-3) == "0.000123457"
assert "{:g}".format(0.12345678901234567890e-4) == "1.23457e-05"
assert "{:g}".format(0.12345678901234567890e-5) == "1.23457e-06"
assert "{:.6g}".format(0.12345678901234567890e-5) == "1.23457e-06"
assert "{:.10g}".format(0.12345678901234567890e-5) == "1.23456789e-06"
assert "{:.20g}".format(0.12345678901234567890e-5) == "1.2345678901234567384e-06"
assert "{:G}".format(0.12345678901234567890) == "0.123457"
assert "{:G}".format(0.12345678901234567890e-1) == "0.0123457"
assert "{:G}".format(0.12345678901234567890e-2) == "0.00123457"
assert "{:G}".format(0.12345678901234567890e-3) == "0.000123457"
assert "{:G}".format(0.12345678901234567890e-4) == "1.23457E-05"
assert "{:G}".format(0.12345678901234567890e-5) == "1.23457E-06"
assert "{:.6G}".format(0.12345678901234567890e-5) == "1.23457E-06"
assert "{:.10G}".format(0.12345678901234567890e-5) == "1.23456789E-06"
assert "{:.20G}".format(0.12345678901234567890e-5) == "1.2345678901234567384E-06"
assert "{:g}".format(float("nan")) == "nan"
assert "{:g}".format(float("-nan")) == "nan"
assert "{:G}".format(float("nan")) == "NAN"
assert "{:g}".format(float("inf")) == "inf"
assert "{:g}".format(float("-inf")) == "-inf"
assert "{:G}".format(float("inf")) == "INF"
assert "{:.0g}".format(1.020e-13) == "1e-13"
assert "{:.0g}".format(1.020e-13) == "1e-13"
assert "{:.1g}".format(1.020e-13) == "1e-13"
assert "{:.2g}".format(1.020e-13) == "1e-13"
assert "{:.3g}".format(1.020e-13) == "1.02e-13"
assert "{:.4g}".format(1.020e-13) == "1.02e-13"
assert "{:.5g}".format(1.020e-13) == "1.02e-13"
assert "{:.6g}".format(1.020e-13) == "1.02e-13"
assert "{:.7g}".format(1.020e-13) == "1.02e-13"
assert "{:g}".format(1.020e-13) == "1.02e-13"
assert "{:g}".format(1.020e-4) == "0.000102"

# Test n & N formatting
assert "{:n}".format(999999.1234) == "999999"
assert "{:n}".format(9999.1234) == "9999.12"
assert "{:n}".format(-1000000.1234) == "-1e+06"
assert "{:n}".format(1000000.1234) == "1e+06"
assert "{:.1n}".format(1000000.1234) == "1e+06"
assert "{:.2n}".format(1000000.1234) == "1e+06"
assert "{:.3n}".format(1000000.1234) == "1e+06"
assert "{:.4n}".format(1000000.1234) == "1e+06"
assert "{:.5n}".format(1000000.1234) == "1e+06"
assert "{:.6n}".format(1000000.1234) == "1e+06"
assert "{:.7n}".format(1000000.1234) == "1000000"
assert "{:.8n}".format(1000000.1234) == "1000000.1"
assert "{:.10n}".format(1000000.1234) == "1000000.123"
assert "{:.11n}".format(1000000.1234) == "1000000.1234"
assert "{:.11n}".format(-1000000.1234) == "-1000000.1234"
assert "{:0n}".format(-1000000.1234) == "-1e+06"
assert "{:n}".format(-1000000.1234) == "-1e+06"
assert "{:-1n}".format(-1000000.1234) == "-1e+06"

with AssertRaises(ValueError, msg="Unknown format code 'N' for object of type 'float'"):
    "{:N}".format(999999.1234)
with AssertRaises(ValueError, msg="Unknown format code 'N' for object of type 'float'"):
    "{:.1N}".format(1000000.1234)
with AssertRaises(ValueError, msg="Unknown format code 'N' for object of type 'float'"):
    "{:0N}".format(-1000000.1234)
with AssertRaises(ValueError, msg="Unknown format code 'N' for object of type 'float'"):
    "{:-1N}".format(-1000000.1234)


# remove*fix test
def test_removeprefix():
    s = "foobarfoo"
    s_ref = "foobarfoo"
    assert s.removeprefix("f") == s_ref[1:]
    assert s.removeprefix("fo") == s_ref[2:]
    assert s.removeprefix("foo") == s_ref[3:]

    assert s.removeprefix("") == s_ref
    assert s.removeprefix("bar") == s_ref
    assert s.removeprefix("lol") == s_ref
    assert s.removeprefix("_foo") == s_ref
    assert s.removeprefix("-foo") == s_ref
    assert s.removeprefix("afoo") == s_ref
    assert s.removeprefix("*foo") == s_ref

    assert s == s_ref, "undefined test fail"

    s_uc = "😱foobarfoo🖖"
    s_ref_uc = "😱foobarfoo🖖"
    assert s_uc.removeprefix("😱") == s_ref_uc[1:]
    assert s_uc.removeprefix("😱fo") == s_ref_uc[3:]
    assert s_uc.removeprefix("😱foo") == s_ref_uc[4:]

    assert s_uc.removeprefix("🖖") == s_ref_uc
    assert s_uc.removeprefix("foo") == s_ref_uc
    assert s_uc.removeprefix(" ") == s_ref_uc
    assert s_uc.removeprefix("_😱") == s_ref_uc
    assert s_uc.removeprefix(" 😱") == s_ref_uc
    assert s_uc.removeprefix("-😱") == s_ref_uc
    assert s_uc.removeprefix("#😱") == s_ref_uc


def test_removeprefix_types():
    s = "0123456"
    s_ref = "0123456"
    others = [0, ["012"]]
    found = False
    for o in others:
        try:
            s.removeprefix(o)
        except:
            found = True

        assert found, f"Removeprefix accepts other type: {type(o)}: {o=}"


def test_removesuffix():
    s = "foobarfoo"
    s_ref = "foobarfoo"
    assert s.removesuffix("o") == s_ref[:-1]
    assert s.removesuffix("oo") == s_ref[:-2]
    assert s.removesuffix("foo") == s_ref[:-3]

    assert s.removesuffix("") == s_ref
    assert s.removesuffix("bar") == s_ref
    assert s.removesuffix("lol") == s_ref
    assert s.removesuffix("foo_") == s_ref
    assert s.removesuffix("foo-") == s_ref
    assert s.removesuffix("foo*") == s_ref
    assert s.removesuffix("fooa") == s_ref

    assert s == s_ref, "undefined test fail"

    s_uc = "😱foobarfoo🖖"
    s_ref_uc = "😱foobarfoo🖖"
    assert s_uc.removesuffix("🖖") == s_ref_uc[:-1]
    assert s_uc.removesuffix("oo🖖") == s_ref_uc[:-3]
    assert s_uc.removesuffix("foo🖖") == s_ref_uc[:-4]

    assert s_uc.removesuffix("😱") == s_ref_uc
    assert s_uc.removesuffix("foo") == s_ref_uc
    assert s_uc.removesuffix(" ") == s_ref_uc
    assert s_uc.removesuffix("🖖_") == s_ref_uc
    assert s_uc.removesuffix("🖖 ") == s_ref_uc
    assert s_uc.removesuffix("🖖-") == s_ref_uc
    assert s_uc.removesuffix("🖖#") == s_ref_uc


def test_removesuffix_types():
    s = "0123456"
    s_ref = "0123456"
    others = [0, 6, ["6"]]
    found = False
    for o in others:
        try:
            s.removesuffix(o)
        except:
            found = True

        assert found, f"Removesuffix accepts other type: {type(o)}: {o=}"


skip_if_unsupported(3, 9, test_removeprefix)
skip_if_unsupported(3, 9, test_removeprefix_types)
skip_if_unsupported(3, 9, test_removesuffix)
skip_if_unsupported(3, 9, test_removesuffix_types)


# Regression to
# https://github.com/RustPython/RustPython/issues/2840

a = "abc123()"

assert id(a) == id(a)
assert id(a) != id(a * -1)
assert id(a) != id(a * 0)
assert id(a) == id(a * 1)  # only cases
assert id(a) == id(1 * a)  # when `id` stays the same
assert id(a) != id(a * 2)


class MyString(str):
    pass


b = MyString("0123abc*&")
assert id(b) == id(b)
assert id(b) != id(b * -1)
assert id(b) != id(b * 0)
assert id(b) != id(b * 1)
assert id(b) != id(1 * b)
assert id(b) != id(b * 2)


def test_huge_width():
    # A width that cannot be allocated is a MemoryError, not an aborted
    # process, and a tabsize wider than a C int does not fit at all.
    for meth in ("center", "ljust", "rjust", "zfill"):
        assert_raises(MemoryError, lambda meth=meth: getattr("a", meth)(1 << 62))
    assert_raises(OverflowError, lambda: "\ta".expandtabs(1 << 62))
    assert_raises(OverflowError, lambda: "\ta".expandtabs(2**31))
    # The widest tabsize that still fits is accepted. With no tab to expand
    # there is nothing to lay out, so the width is never allocated.
    assert "a".expandtabs(2**31 - 1) == "a"


test_huge_width()


def test_replace_empty_pattern():
    # An empty pattern matches at every code point boundary, and only there.
    # Matching it byte by byte instead put the replacement inside a multi-byte
    # character, so what came back was no longer the text that went in.
    assert "abc".replace("", "-") == "-a-b-c-"
    assert "ábç".replace("", "#") == "#á#b#ç#"
    assert "😀".replace("", "-") == "-😀-"
    assert "".replace("", "-") == "-"
    assert "abc".replace("", "") == "abc"

    # The count is a number of insertions, and the one after the last
    # character only happens if the count reaches that far.
    assert "abc".replace("", "-", 0) == "abc"
    assert "abc".replace("", "-", 1) == "-abc"
    assert "abc".replace("", "-", 3) == "-a-b-c"
    assert "abc".replace("", "-", 4) == "-a-b-c-"
    assert "abc".replace("", "-", 99) == "-a-b-c-"
    assert "ábç".replace("", "#", 2) == "#á#bç"

    # The result has to stay readable as text afterwards.
    spread = "á".replace("", "-")
    assert len(spread) == 3
    assert list(spread) == ["-", "á", "-"]
    assert spread[1] == "á"
    assert spread.upper() == "-Á-"
    assert spread.encode("utf-8") == b"-\xc3\xa1-"

    # A pattern that is not empty was already fine and stays that way.
    assert "ábç".replace("b", "#") == "á#ç"
    assert "ábç".replace("á", "#") == "#bç"
    assert "aaa".replace("a", "b", 2) == "bba"


test_replace_empty_pattern()


def test_replace_unchanged():
    for text in ("already clean", "déjà propre", "a\ud800b"):
        for old, new, count in (
            (text[:1], "#", 0),
            (text[:1], text[:1], -1),
            (text + " more", "replacement", -1),
        ):
            assert text.replace(old, new, count) is text
            result = MyString(text).replace(old, new, count)
            assert result == text
            assert type(result) is str

    # Returning the original string must not bypass argument conversion.
    assert_raises(TypeError, lambda: "abc".replace(1, "x", 0))
    assert_raises(TypeError, lambda: "abc".replace("a", 1, 0))
    assert_raises(TypeError, lambda: "abc".replace("a", "a", None))


test_replace_unchanged()


def test_replace_missing_pattern():
    for text in ("ordinary text", "\u00e1\u03b2\U0001f600", "a\ud800b\udfff"):
        for count in (-1, 1, 10):
            assert text.replace("\r", "replacement", count) is text
            result = MyString(text).replace("\r", "replacement", count)
            assert type(result) is str
            assert result == text

    # Continue the same search after the first match, including lone surrogates.
    assert "ababa".replace("aba", "x") == "xba"
    assert "\ud800x\ud800".replace("\ud800", "\udfff", 1) == "\udfffx\ud800"
    assert "\ud800x\ud800".replace("\ud800", "\udfff") == "\udfffx\udfff"


test_replace_missing_pattern()


def test_expandtabs_zero_tabsize():
    # With no width to advance to, the tabs come out and nothing else moves.
    # A tab that followed a character used to ask for a run of usize::MAX
    # spaces and take the interpreter down with it.
    for tabsize in (0, -1, -8):
        assert "a\tb".expandtabs(tabsize) == "ab"
        assert "ab\tcd\tef".expandtabs(tabsize) == "abcdef"
        assert "a\nb\tc".expandtabs(tabsize) == "a\nbc"
        assert "a\r\nb\tc".expandtabs(tabsize) == "a\r\nbc"
        assert "á\tb".expandtabs(tabsize) == "áb"
        assert "😀\tb".expandtabs(tabsize) == "😀b"
        assert "\ta".expandtabs(tabsize) == "a"
        assert "\t".expandtabs(tabsize) == ""
        assert "".expandtabs(tabsize) == ""
        assert "no tabs".expandtabs(tabsize) == "no tabs"
        assert b"a\tb".expandtabs(tabsize) == b"ab"
        assert bytearray(b"a\tb").expandtabs(tabsize) == bytearray(b"ab")

    # A tab size that is actually there keeps working.
    assert "a\tb".expandtabs(8) == "a       b"
    assert "a\tb".expandtabs(1) == "a b"
    assert "abcd\te".expandtabs(4) == "abcd    e"
    assert "a\nb\tc".expandtabs(4) == "a\nb   c"


test_expandtabs_zero_tabsize()


def test_concat_error_message():
    # CPython falls back to str's sequence concatenation once a reflected
    # __radd__ declines, so the message names the concatenation, not the
    # generic binary operation. Types that define __radd__ (every number)
    # used to take the generic path here.
    def message(other):
        try:
            "a" + other
        except TypeError as err:
            return str(err)
        raise AssertionError("TypeError was not raised")

    assert message(1) == 'can only concatenate str (not "int") to str'
    assert message(1.5) == 'can only concatenate str (not "float") to str'
    assert message(True) == 'can only concatenate str (not "bool") to str'
    assert message([]) == 'can only concatenate str (not "list") to str'
    assert message(None) == 'can only concatenate str (not "NoneType") to str'
    assert message(b"b") == 'can only concatenate str (not "bytes") to str'

    class Declines:
        def __radd__(self, other):
            return NotImplemented

    assert message(Declines()) == 'can only concatenate str (not "Declines") to str'

    # A __radd__ that returns a value still wins.
    class Accepts:
        def __radd__(self, other):
            return "from-radd"

    assert "a" + Accepts() == "from-radd"


test_concat_error_message()


def test_subscript_typeerror_message():
    def message(index):
        try:
            "abc"[index]
        except TypeError as err:
            return str(err)
        raise AssertionError("TypeError was not raised")

    assert message("def") == "string indices must be integers, not 'str'"
    assert message(object()) == "string indices must be integers, not 'object'"
    assert message(1.5) == "string indices must be integers, not 'float'"
    assert message(None) == "string indices must be integers, not 'NoneType'"

    Long = type("A" * 250, (), {})
    assert message(Long()) == "string indices must be integers, not '{}'".format(
        "A" * 200
    )


test_subscript_typeerror_message()


# Decimal conversion errors identify the consumed argument, including star arguments.
for is_bytes in (False, True):
    for conversion in "diu":
        for template, values, context in (
            ("%" + conversion, "bad", ""),
            ("%" + conversion, ("bad",), " 1"),
            ("%s %" + conversion, ("ok", "bad"), " 2"),
            ("%*.*" + conversion, (5, 2, "bad"), " 3"),
            ("%(value)" + conversion, {"value": "bad"}, " 'value'"),
        ):
            if is_bytes:
                template = template.encode()
                if isinstance(values, dict):
                    values = {b"value": "bad"}
                    context = " b'value'"
                elif isinstance(values, tuple) and len(values) == 2:
                    values = (b"ok", "bad")
            with assert_raises(TypeError) as caught:
                template % values
            assert str(caught.exception) == (
                f"format argument{context}: %{conversion} requires a real number, not str"
            )


def test_percent_callback_order_bounded():
    for template in ("%r %", b"%r %", bytearray(b"%r %")):
        calls = []

        class Value:
            def __repr__(self):
                calls.append("repr")
                return "value"

        with assert_raises(ValueError):
            template % (Value(),)
        assert calls == ["repr"]

    for template, key in (
        ("%(x)*r", "x"),
        (b"%(x)*r", b"x"),
        (bytearray(b"%(x)*r"), b"x"),
    ):
        calls = []

        class Mapping:
            def __getitem__(self, name):
                calls.append(name)
                return 5

        with assert_raises(ValueError):
            template % Mapping()
        assert calls == [key]


test_percent_callback_order_bounded()


class PartitionString(str):
    pass


partition_string = PartitionString("abc")
assert partition_string.partition(":")[0] is partition_string
assert partition_string.rpartition(":")[2] is partition_string


# Codec searches run while nonempty input is exported, but after an empty
# export is released. They must see the original storage, not a prior copy.
def test_decode_codec_search_buffer_lifetime():
    import codecs

    for kind in (bytearray, memoryview):
        for initial in (b"x", b""):
            data = bytearray(initial)
            source = data if kind is bytearray else memoryview(data)
            events = []
            name = "snippet_buffer_%s_%s" % (kind.__name__, len(initial))

            def search(encoding):
                if encoding != name:
                    return None
                try:
                    data.extend(b"y")
                    events.append("resized")
                except BufferError:
                    events.append("exported")
                if initial:
                    data[0] = ord("z")
                return codecs.lookup("utf-8")

            codecs.register(search)
            try:
                assert str(source, name) == ("z" if initial else "")
                # Normal mode never looks up the encoding for empty input.
                if initial or sys.flags.dev_mode:
                    expected = (
                        "exported" if initial or kind is memoryview else "resized"
                    )
                    assert events == [expected], events
                else:
                    assert events == []
            finally:
                codecs.unregister(search)
                if kind is memoryview:
                    source.release()
            data.extend(b"released")

    class Exporter:
        def __init__(self, data):
            self.data = data
            self.events = []

        def __buffer__(self, flags):
            self.events.append("acquire")
            return memoryview(self.data)

        def __release_buffer__(self, view):
            self.events.append("release")

    for initial in (b"x", b""):
        exporter = Exporter(bytearray(initial))
        name = "snippet_exporter_%s" % len(initial)

        def search(encoding):
            if encoding != name:
                return None
            exporter.events.append("lookup")
            if initial:
                exporter.data[0] = ord("z")
            return codecs.lookup("utf-8")

        codecs.register(search)
        try:
            assert str(exporter, name) == ("z" if initial else "")
            expected = (
                ["acquire", "lookup", "release"] if initial else ["acquire", "release"]
            )
            if not initial and sys.flags.dev_mode:
                expected.append("lookup")
            assert exporter.events == expected, exporter.events
        finally:
            codecs.unregister(search)


test_decode_codec_search_buffer_lifetime()


class CodecBufferBytearray(bytearray):
    exports = 0

    def __buffer__(self, flags):
        self.exports += 1
        return memoryview(b"first" if self.exports == 1 else b"second")


codec_buffer = CodecBufferBytearray(b"unused")
assert str(codec_buffer, "ascii") == "first"
assert codec_buffer.exports == 1


if sys.flags.dev_mode:
    import codecs
    import encodings

    utf8_codec = codecs.lookup("utf8")
    codecs.unregister(encodings.search_function)
    codec_input = bytearray(b"x")
    codec_events = []

    def search_utf8_alias(name):
        if name != "utf8":
            return None
        with assert_raises(BufferError):
            codec_input.extend(b"y")
        codec_input[0] = ord("z")
        codec_events.append(name)
        return utf8_codec

    codecs.register(search_utf8_alias)
    try:
        assert str(codec_input, "UTF8") == "z"
        assert codec_events == ["utf8"]
    finally:
        codecs.unregister(search_utf8_alias)
        codecs.register(encodings.search_function)
    codec_input.extend(b"released")


def test_decode_callback_input():
    import array
    import codecs

    class BytesWithBuffer(bytes):
        def __buffer__(self, flags):
            raise AssertionError("bytes decoding must use the native payload")

    assert str(BytesWithBuffer(b"native"), "ascii") == "native"
    bad_bytes = BytesWithBuffer(b"\xffA")
    with assert_raises(UnicodeDecodeError) as caught:
        bad_bytes.decode("ascii")
    assert type(caught.exception.object) is bytes
    assert (
        caught.exception.object == bad_bytes
        and caught.exception.object is not bad_bytes
    )
    for wrap in (bytes, BytesWithBuffer, bytearray, memoryview):
        data = bytearray(b"x")
        source = wrap(data)
        name = "snippet_decoder_input_%s" % wrap.__name__.lower()

        def decode(view, errors="strict"):
            assert type(view) is memoryview
            assert view.readonly and view.obj is None
            with assert_raises(TypeError):
                view[0] = ord("z")
            if wrap is bytearray:
                source[0] = ord("z")
            elif wrap is memoryview:
                data[0] = ord("z")
            return (chr(view[0]), len(view))

        def search(encoding):
            if encoding == name:
                return codecs.CodecInfo(name=name, encode=None, decode=decode)

        codecs.register(search)
        try:
            assert str(source, name) == ("x" if issubclass(wrap, bytes) else "z")
        finally:
            codecs.unregister(search)
            if wrap is memoryview:
                source.release()

    for source in (array.array("I", [0x61]), memoryview(b"_abc")[1:]):
        expected_bytes = bytes(source)

        def decode_bytes(view, errors="strict"):
            assert view.itemsize == 1 and view.format == "B"
            assert view.shape == (len(expected_bytes),) and view.strides == (1,)
            assert view.readonly and view.obj is None
            assert bytes(view) == expected_bytes
            return ("ok", len(view))

        def search_bytes(encoding):
            if encoding == "snippet_decoder_layout":
                return codecs.CodecInfo(name=encoding, encode=None, decode=decode_bytes)

        codecs.register(search_bytes)
        try:
            assert str(source, "snippet_decoder_layout") == "ok"
        finally:
            codecs.unregister(search_bytes)

    data = bytearray(b"\xffA")

    def mutate_original(exc):
        # The input remains exported, but its read lock must not surround Python.
        with assert_raises(BufferError):
            data.extend(b"y")
        data[1] = ord("B")
        return ("X", exc.end)

    codecs.register_error("snippet_mutate_original", mutate_original)
    assert data.decode("ascii", "snippet_mutate_original") == "XA"
    assert data == b"\xffB"
    data.extend(b"released")

    def change_object_from_index(exc):
        class Restart:
            def __index__(self):
                exc.object = b"\xffZ"
                return 1

        return ("X", Restart())

    codecs.register_error("snippet_restart_index", change_object_from_index)
    assert b"\xffA".decode("ascii", "snippet_restart_index") == "XZ"

    for position, error, message in (
        (10**100, OverflowError, "Python int too large to convert to C ssize_t"),
        ("x", TypeError, "'str' object cannot be interpreted as an integer"),
        (-100, IndexError, "position -99 from error handler out of bounds"),
    ):

        def invalid_position(exc):
            return ("X", position)

        codecs.register_error("snippet_invalid_position", invalid_position)
        with assert_raises(error) as caught:
            b"\xff".decode("ascii", "snippet_invalid_position")
        assert str(caught.exception) == message


test_decode_callback_input()


def test_text_codec_aliases_use_registry():
    import codecs
    import encodings

    aliases = ("u8", "utf", "latin", "646")

    def search(encoding):
        if encoding in aliases + ("utf16", "utf32"):
            return codecs.CodecInfo(
                name=encoding,
                encode=lambda obj, errors="strict": (b"custom", len(obj)),
                decode=lambda obj, errors="strict": ("custom", len(obj)),
            )

    codecs.unregister(encodings.search_function)
    codecs.register(search)
    try:
        for alias in aliases:
            assert "x".encode(alias) == b"custom"
            assert b"x".decode(alias) == "custom"
        # CPython's actual fast-path spellings still bypass custom codecs.
        assert "x".encode("utf8") == b"x"
        assert b"x".decode("ascii") == "x"
        for name, native_encode in (
            ("utf16", codecs.utf_16_encode),
            ("utf32", codecs.utf_32_encode),
        ):
            encoded = native_encode("x")[0]
            assert "x".encode(name) == encoded
            assert encoded.decode(name) == "x"
    finally:
        codecs.unregister(search)
        codecs.register(encodings.search_function)


test_text_codec_aliases_use_registry()


def test_text_codec_error_boundaries():
    import codecs
    import warnings

    for encoding in ("ascii", "utf8", "utf16", "utf32"):
        with assert_raises(UnicodeEncodeError) as caught:
            "\ud800".encode(encoding)
        assert not hasattr(caught.exception, "__notes__")
    with assert_raises(UnicodeDecodeError) as caught:
        b"\xff".decode("ascii")
    assert not hasattr(caught.exception, "__notes__")
    with assert_raises(UnicodeDecodeError) as caught:
        b"\xff".decode("u8")
    assert caught.exception.__notes__ == ["decoding with 'u8' codec failed"]

    result = bytearray(b"x")

    def search(encoding):
        if encoding == "snippet_bytearray_encoder":
            return codecs.CodecInfo(
                name=encoding,
                encode=lambda obj, errors="strict": (result, len(obj)),
                decode=None,
            )

    codecs.register(search)
    try:
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            assert "x".encode("snippet_bytearray_encoder") == b"x"
        assert len(caught) == 1 and caught[0].category is RuntimeWarning
        assert (
            str(caught[0].message)
            == "encoder snippet_bytearray_encoder returned bytearray instead of bytes; use codecs.encode() to encode to arbitrary types"
        )
        with warnings.catch_warnings():
            warnings.simplefilter("error")
            with assert_raises(RuntimeWarning) as caught:
                "x".encode("snippet_bytearray_encoder")
            assert not hasattr(caught.exception, "__notes__")
    finally:
        codecs.unregister(search)


test_text_codec_error_boundaries()


for wide_encoding, invalid, error_encoding in (
    ("utf-16", b"\xff\xfe\x00\xdc", "utf-16-le"),
    ("utf-16", b"\xfe\xff\xdc\x00", "utf-16-be"),
    ("utf-32", b"\xff\xfe\x00\x00\x00\x00\x11\x00", "utf-32-le"),
    ("utf-32", b"\x00\x00\xfe\xff\x00\x11\x00\x00", "utf-32-be"),
):
    with assert_raises(UnicodeDecodeError) as caught:
        invalid.decode(wide_encoding)
    assert caught.exception.encoding == error_encoding
    assert not hasattr(caught.exception, "__notes__")


for wide_encoding, surrogate in (
    ("utf-16", b"\xff\xfe\x00\xd8"),
    ("utf-16", b"\xfe\xff\xd8\x00"),
    ("utf-32", b"\xff\xfe\x00\x00\x00\xd8\x00\x00"),
    ("utf-32", b"\x00\x00\xfe\xff\x00\x00\xd8\x00"),
):
    assert surrogate.decode(wide_encoding, "surrogatepass") == "\ud800"


def test_text_decoder_str_subclass():
    import codecs

    class Text(str):
        def __str__(self):
            raise AssertionError("decoder result normalization must use its payload")

    def search(name):
        if name == "snippet_str_subclass":
            return codecs.CodecInfo(
                name=name,
                encode=None,
                decode=lambda obj, errors="strict": (text, len(obj)),
            )

    codecs.register(search)
    try:
        for value in ("", "a", "é", "Ā", "decoded"):
            text = Text(value)
            result = b"x".decode("snippet_str_subclass")
            assert result == value
            if not value or len(value) == 1 and ord(value) <= 255:
                assert type(result) is str and result is value
            else:
                assert result is text
    finally:
        codecs.unregister(search)


test_text_decoder_str_subclass()
