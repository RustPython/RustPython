# Test the unicode support! 👋


ᚴ = 2

assert ᚴ * 8 == 16

ᚴ = "👋"

c = ᚴ * 3

assert c == "👋👋👋"

import unicodedata

assert unicodedata.category("a") == "Ll"
assert unicodedata.category("A") == "Lu"
assert unicodedata.name("a") == "LATIN SMALL LETTER A"
assert unicodedata.lookup("LATIN SMALL LETTER A") == "a"
assert unicodedata.bidirectional("a") == "L"
assert unicodedata.east_asian_width("\u231a") == "W"
assert unicodedata.normalize("NFC", "bla") == "bla"
square_aruhua = "<square> 30A2 30EB 30D5 30A1"
assert unicodedata.decomposition("\u3301") == square_aruhua
assert unicodedata.ucd_3_2_0.decomposition("\u3301") == square_aruhua

# These are from NormalizationCorrections.txt
assert unicodedata.decomposition("\uf951") == "964B"
assert unicodedata.ucd_3_2_0.decomposition("\uf951") == "964B"

# testing unicodedata.ucd_3_2_0 for idna
assert "abcСĤ".encode("idna") == b"xn--abc-7sa390b"
assert "abc䄣Ĳ".encode("idna") == b"xn--abcij-zb5f"

# from CPython tests
assert "python.org".encode("idna") == b"python.org"
assert "python.org.".encode("idna") == b"python.org."
assert "pyth\xf6n.org".encode("idna") == b"xn--pythn-mua.org"
assert "pyth\xf6n.org.".encode("idna") == b"xn--pythn-mua.org."
assert b"python.org".decode("idna") == "python.org"
assert b"python.org.".decode("idna") == "python.org."
assert b"xn--pythn-mua.org".decode("idna") == "pyth\xf6n.org"
assert b"xn--pythn-mua.org.".decode("idna") == "pyth\xf6n.org."

assert unicodedata.ucd_3_2_0.east_asian_width("\u231a") == "N"
assert not unicodedata.ucd_3_2_0.mirrored("\u0f3a")
