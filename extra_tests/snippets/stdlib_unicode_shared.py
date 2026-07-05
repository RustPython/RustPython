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
