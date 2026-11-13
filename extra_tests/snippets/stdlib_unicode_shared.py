# Exercises the Unicode semantics routed through the shared rustpython-unicode
# crate: str predicates, casefold, identifier rules, unicodedata queries,
# normalization, \N{} escapes, and re character classes.

import re
import unicodedata

# --- str classification predicates ---------------------------------------

# Numeric_Type chain: isdecimal ⊂ isdigit ⊂ isnumeric
assert "5".isdecimal() and "5".isdigit() and "5".isnumeric()
assert not "²".isdecimal()  # SUPERSCRIPT TWO: digit but not decimal
assert "²".isdigit() and "²".isnumeric()
assert not "⅓".isdigit()  # VULGAR FRACTION ONE THIRD: numeric only
assert "⅓".isnumeric()

assert "abc".isalpha()
assert "abc123".isalnum()
assert not "abc123".isalpha()
assert "あ".isalpha()  # HIRAGANA LETTER A

assert " \t\n".isspace()
assert "　".isspace()  # IDEOGRAPHIC SPACE
assert "hello world".isprintable()
assert not "\x00".isprintable()
assert " ".isprintable()  # ASCII space is printable

# identifier rules (XID_Start / XID_Continue, plus leading underscore)
assert "_var".isidentifier()
assert "유니코드".isidentifier()  # Hangul identifier
assert not "1abc".isidentifier()
assert not "a b".isidentifier()

# --- case mapping / casefold ---------------------------------------------

assert "ABC".lower() == "abc"
assert "abc".upper() == "ABC"
# casefold uses full mappings, unlike lower()
assert "ß".casefold() == "ss"  # LATIN SMALL LETTER SHARP S
assert "Σ".casefold() == "σ"  # GREEK CAPITAL SIGMA -> small sigma
assert "Straße".casefold() == "strasse"

# lone-surrogate safety: casefold must not panic on surrogates
surrogate = "\ud800"
assert surrogate.casefold() == surrogate

# --- unicodedata ----------------------------------------------------------

assert unicodedata.category("A") == "Lu"
assert unicodedata.category("1") == "Nd"
assert unicodedata.bidirectional("A") == "L"
assert unicodedata.decimal("٥") == 5  # ARABIC-INDIC DIGIT FIVE
assert unicodedata.digit("²") == 2
assert abs(unicodedata.numeric("⅓") - (1 / 3)) < 1e-6
assert unicodedata.name("☃") == "SNOWMAN"
assert unicodedata.lookup("SNOWMAN") == "☃"
assert unicodedata.lookup("LATIN CAPITAL LETTER GHA") == "\u01a2"
assert unicodedata.lookup("LATIN SMALL LETTER R WITH TILDE") == "r\u0303"
assert unicodedata.lookup("TANGUT IDEOGRAPH-17000") == "\U00017000"
assert unicodedata.ucd_3_2_0.lookup("SNOWMAN") == "☃"
try:
    unicodedata.ucd_3_2_0.lookup("LATIN CAPITAL LETTER GHA")
except KeyError:
    pass
else:
    raise AssertionError("3.2.0 has no name aliases")
assert unicodedata.combining("́") == 230  # COMBINING ACUTE ACCENT
assert unicodedata.mirrored("(") == 1
assert unicodedata.east_asian_width("あ") == "W"

# ucd_3_2_0 legacy view (used by stringprep)
assert unicodedata.ucd_3_2_0.unidata_version == "3.2.0"

# --- normalization --------------------------------------------------------

composed = "é"  # é
decomposed = "é"
assert unicodedata.normalize("NFC", decomposed) == composed
assert unicodedata.normalize("NFD", composed) == decomposed
assert unicodedata.is_normalized("NFC", composed)
assert not unicodedata.is_normalized("NFD", composed)

# --- \N{} escapes (compiler) ---------------------------------------------

assert "\N{SNOWMAN}" == "☃"
assert "\N{GREEK SMALL LETTER ALPHA}" == "α"

# --- re character classes -------------------------------------------------

assert re.fullmatch(r"\w+", "abc_123") is not None
assert re.fullmatch(r"\w+", "유니코드") is not None  # \w is Unicode-aware
assert re.fullmatch(r"\d+", "123") is not None
# \d matches Unicode decimal digits (category Nd), not just ASCII
assert re.fullmatch(r"\d", "٥") is not None  # ARABIC-INDIC DIGIT FIVE
assert re.fullmatch(r"\d", "५") is not None  # DEVANAGARI DIGIT FIVE
assert re.fullmatch(r"\d", "²") is None  # SUPERSCRIPT TWO (No), not decimal
assert re.fullmatch(r"\s+", " \t\n") is not None
# ASCII flag restricts \w to ASCII
assert re.fullmatch(r"\w+", "유", re.ASCII) is None
# case-insensitive matching routes through the shared case helpers
assert re.fullmatch(r"straße", "STRAßE", re.IGNORECASE) is not None

print("stdlib_unicode_shared: OK")

# Surrogates retain their UCD properties even though they are not Rust chars.
for database in (unicodedata, unicodedata.ucd_3_2_0):
    for codepoint in (0xD800, 0xDBFF, 0xDC00, 0xDFFF):
        assert database.bidirectional(chr(codepoint)) == "L"

# Existing characters must not acquire numeric values introduced after 3.2.
for character, value in (("\u09f8", 0.75), ("\u3405", 5.0), ("\u3483", 2.0)):
    assert unicodedata.numeric(character) == value
    assert unicodedata.ucd_3_2_0.numeric(character, None) is None
    try:
        unicodedata.ucd_3_2_0.numeric(character)
    except ValueError:
        pass
    else:
        raise AssertionError("modern numeric property leaked into Unicode 3.2")
assert unicodedata.ucd_3_2_0.numeric("\u4e00") == 1.0

# Native name consumers use the UCD file-order private alias/sequence records.
assert chr(0xF0000).encode("ascii", "namereplace") == rb"\N{NULL}"
assert chr(0xF0200).encode("ascii", "namereplace") == rb"\N{KEYCAP NUMBER SIGN}"
assert unicodedata.name(chr(0xF0000), None) is None
assert unicodedata.name(chr(0xF0200), None) is None

# A fresh process is required because a successful native lookup caches the
# provider for the interpreter, independently of subsequent module imports.
import subprocess
import sys

subprocess.run(
    [
        sys.executable,
        "-c",
        r"""
import codecs
import sys
sys.modules['unicodedata'] = None
try:
    codecs.namereplace_errors(ValueError())
except TypeError:
    pass
else:
    raise AssertionError('exception type must be checked before import')
decoder = codecs.getincrementaldecoder('unicode_escape')('ignore')
try:
    decoder.decode(br'\N', final=False)
except UnicodeError as exc:
    assert str(exc) == "\\N escapes not supported (can't load unicodedata module)"
else:
    raise AssertionError('incomplete named escape must acquire the provider')
for data in (br'\N', br'\N{INVALID}', br'\N{LATIN CAPITAL LETTER A}'):
    try:
        data.decode('unicode_escape', 'ignore')
    except UnicodeError as exc:
        assert str(exc) == "\\N escapes not supported (can't load unicodedata module)", str(exc)
    else:
        raise AssertionError('the import failure must precede name validation')
try:
    codecs.namereplace_errors(UnicodeEncodeError('ascii', '', 0, 0, 'reason'))
except ImportError as exc:
    assert str(exc) == 'PyCapsule_Import could not import module "unicodedata"'
else:
    raise AssertionError('even an empty namereplace range acquires the provider')
import types
sys.modules['unicodedata'] = types.SimpleNamespace(_ucnhash_CAPI=42)
try:
    codecs.namereplace_errors(UnicodeEncodeError('ascii', '', 0, 0, 'reason'))
except AttributeError as exc:
    assert str(exc) == 'PyCapsule_Import "unicodedata._ucnhash_CAPI" is not valid'
else:
    raise AssertionError('an invalid provider must not be cached')
del sys.modules['unicodedata']
assert br'\N{LATIN CAPITAL LETTER A}'.decode('unicode_escape') == 'A'
assert 'unicodedata' in sys.modules
# A cached provider is shared with namereplace and remains valid after unload.
del sys.modules['unicodedata']
assert chr(0x2603).encode('ascii', 'namereplace') == br'\N{SNOWMAN}'
assert 'unicodedata' not in sys.modules
""",
    ],
    check=True,
)
