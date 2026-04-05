import re
import unicodedata

assert "유니코드".isidentifier()
assert "५".isdecimal()
assert "\u3000".isspace()
assert " ".isprintable()
assert not "\n".isprintable()

assert unicodedata.category("\ud800") == "Cs"
assert unicodedata.lookup("SNOWMAN") == "☃"
assert unicodedata.name("☃") == "SNOWMAN"
assert unicodedata.normalize("NFC", "e\u0301") == "é"
assert unicodedata.digit("²") == 2
assert unicodedata.decimal("५") == 5
assert unicodedata.numeric("⅓") == 1 / 3

assert re.fullmatch(r"\w+", "가나다")
assert re.fullmatch(r"\d+", "५६७")
assert re.fullmatch(r"\s+", "\u3000")
