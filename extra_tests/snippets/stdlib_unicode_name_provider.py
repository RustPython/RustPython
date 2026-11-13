# Run as a separate snippet: the host harness creates a fresh interpreter for
# every file, including WASI guests that cannot create subprocesses themselves.
# Do not prime the Unicode-name cache before checking failed imports below.
import codecs
import sys

sys.modules["unicodedata"] = None
try:
    codecs.namereplace_errors(ValueError())
except TypeError:
    pass
else:
    raise AssertionError("exception type must be checked before import")
decoder = codecs.getincrementaldecoder("unicode_escape")("ignore")
try:
    decoder.decode(rb"\N", final=False)
except UnicodeError as exc:
    assert str(exc) == "\\N escapes not supported (can't load unicodedata module)"
else:
    raise AssertionError("incomplete named escape must acquire the provider")
for data in (rb"\N", rb"\N{INVALID}", rb"\N{LATIN CAPITAL LETTER A}"):
    try:
        data.decode("unicode_escape", "ignore")
    except UnicodeError as exc:
        assert (
            str(exc) == "\\N escapes not supported (can't load unicodedata module)"
        ), str(exc)
    else:
        raise AssertionError("the import failure must precede name validation")
try:
    codecs.namereplace_errors(UnicodeEncodeError("ascii", "", 0, 0, "reason"))
except ImportError as exc:
    assert str(exc) == 'PyCapsule_Import could not import module "unicodedata"'
else:
    raise AssertionError("even an empty namereplace range acquires the provider")
import types

sys.modules["unicodedata"] = types.SimpleNamespace(_ucnhash_CAPI=42)
try:
    codecs.namereplace_errors(UnicodeEncodeError("ascii", "", 0, 0, "reason"))
except AttributeError as exc:
    assert str(exc) == 'PyCapsule_Import "unicodedata._ucnhash_CAPI" is not valid'
else:
    raise AssertionError("an invalid provider must not be cached")
del sys.modules["unicodedata"]
assert rb"\N{LATIN CAPITAL LETTER A}".decode("unicode_escape") == "A"
assert "unicodedata" in sys.modules
# A cached provider is shared with namereplace and remains valid after unload.
del sys.modules["unicodedata"]
assert chr(0x2603).encode("ascii", "namereplace") == rb"\N{SNOWMAN}"
assert "unicodedata" not in sys.modules
