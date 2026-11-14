from xml.parsers import expat

# Without namespace expansion, element and attribute names retain prefixes.
for separator, expected_name, expected_attribute in (
    (None, "p:x", "p:a"),
    ("!", "urn:p!x", "urn:p!a"),
):
    for ordered in (False, True):
        parser = expat.ParserCreate(namespace_separator=separator)
        parser.ordered_attributes = ordered
        starts = []
        ends = []
        parser.StartElementHandler = lambda name, attrs: starts.append((name, attrs))
        parser.EndElementHandler = ends.append
        parser.Parse('<r xmlns:p="urn:p"><p:x p:a="v" a="u"></p:x></r>', True)
        assert [name for name, attrs in starts] == ["r", expected_name]
        assert ends == [expected_name, "r"]
        expected_attributes = (
            [expected_attribute, "v", "a", "u"]
            if ordered
            else {expected_attribute: "v", "a": "u"}
        )
        assert starts[1][1] == expected_attributes

# Parse accepts contiguous buffer objects, including views of mutable input.
for xml in (bytearray(b"<r>ok</r>"), memoryview(b"<r>ok</r>")):
    parser = expat.ParserCreate()
    text = []
    parser.CharacterDataHandler = text.append
    assert parser.Parse(xml, True) == 1
    assert "".join(text) == "ok"

try:
    expat.ParserCreate().Parse(memoryview(b"<r/>")[::2], True)
except BufferError:
    pass
else:
    raise AssertionError("Parse accepted a non-contiguous buffer")

# Keep the export alive through callbacks without retaining its read lock.
xml = bytearray(b"<r/>")
parser = expat.ParserCreate()
parser.StartElementHandler = lambda *_: xml.extend(b" ")
try:
    parser.Parse(xml, True)
except BufferError:
    pass
else:
    raise AssertionError("Parse released its input buffer before the callback")
xml.extend(b" ")


# Expat's Error* positions alias Current* even before a parse error.
def check_position_aliases(parser):
    for suffix in ("LineNumber", "ColumnNumber", "ByteIndex"):
        assert getattr(parser, "Error" + suffix) == getattr(parser, "Current" + suffix)
        try:
            setattr(parser, "Error" + suffix, 0)
        except AttributeError:
            pass
        else:
            raise AssertionError("error positions must be read-only")


parser = expat.ParserCreate()
check_position_aliases(parser)
parser.StartElementHandler = lambda *_: check_position_aliases(parser)
parser.Parse(b"<r>", False)
check_position_aliases(parser)
try:
    parser.Parse(b"", True)
except expat.ExpatError:
    check_position_aliases(parser)
else:
    raise AssertionError("incomplete XML accepted")

# Unicode input overrides encoding declarations only before parsing starts.
for declaration in ("iso-8859-1", "utf-16", "unknown-encoding"):
    parser = expat.ParserCreate()
    text = []
    parser.CharacterDataHandler = text.append
    parser.Parse(f'<?xml version="1.0" encoding="{declaration}"?><r>', False)
    parser.Parse("£€</r>", True)
    assert "".join(text) == "£€"

for first, tail, expected in (
    ("", '<?xml version="1.0" encoding="iso-8859-1"?><r>£</r>', "£"),
    (b"", '<?xml version="1.0" encoding="iso-8859-1"?><r>£</r>', "Â£"),
    ('<?xml version="1.0" encoding="iso-8859-1"?><r>', b"\xc2\xa3</r>", "£"),
    (b'<?xml version="1.0" encoding="iso-8859-1"?><r>', "£</r>", "Â£"),
    (b'<?xml version="1.0" encoding="iso-8859-1"?><r>', b"\xa3</r>", "£"),
):
    parser = expat.ParserCreate()
    text = []
    parser.CharacterDataHandler = text.append
    parser.Parse(first, False)
    parser.Parse(tail, True)
    assert "".join(text) == expected


# Buffer configuration uses integer objects, not the __index__ protocol.
class BufferIndex:
    def __index__(self):
        return 32


parser = expat.ParserCreate()
assert parser.buffer_size == 8192
assert parser.buffer_used == 0
parser.buffer_size = 8192
assert parser.buffer_text is False  # Identical size does not enable buffering.
for value in (BufferIndex(), "32"):
    try:
        parser.buffer_size = value
    except TypeError as exc:
        assert str(exc) == "buffer_size must be an integer"
    else:
        raise AssertionError("non-integer buffer size accepted")
parser.buffer_size = True
assert parser.buffer_size == 1 and parser.buffer_text is True
parser.buffer_text = False
assert parser.buffer_size == 1 and parser.buffer_used == 0
for name in ("buffer_size", "buffer_text", "CharacterDataHandler"):
    try:
        delattr(parser, name)
    except RuntimeError as exc:
        assert str(exc) == "Cannot delete attribute"
    else:
        raise AssertionError("parser attribute deleted")

# Flushed data is detached before invoking callbacks.
# A callback may change the buffer without leaving a Rust lock held.
parser = expat.ParserCreate()
parser.buffer_text = True
observed = []
resized = False


def resize_during_flush(text):
    global resized
    observed.append((text, parser.buffer_used))
    if not resized:
        resized = True
        parser.buffer_size = 64


parser.CharacterDataHandler = resize_during_flush
parser.Parse("<r>é<x/>z</r>", True)
assert observed == [("éz", 0)]
assert parser.buffer_size == 64 and parser.buffer_used == 0

# CharacterDataHandler replacement flushes existing data to the old handler.
parser = expat.ParserCreate()
parser.buffer_text = True
old_text, new_text = [], []
parser.CharacterDataHandler = old_text.append


def replace_from_character_callback(text):
    old_text.append(text)
    if len(old_text) == 1:
        parser.CharacterDataHandler = new_text.append


parser.CharacterDataHandler = replace_from_character_callback
parser.Parse(b"<r>a<x/>b</r>", True)
assert old_text == ["ab"] and new_text == []
assert parser.CharacterDataHandler is not replace_from_character_callback
assert parser.buffer_used == 0

# Flush exceptions propagate; the used count is reset even on failure.
parser = expat.ParserCreate()
parser.buffer_text = True


def fail_buffered_text(text):
    assert parser.buffer_used == 0
    raise ValueError("buffer callback failure")


parser.CharacterDataHandler = fail_buffered_text
try:
    parser.Parse("<r>é</r>", True)
except ValueError as exc:
    assert str(exc) == "buffer callback failure"
else:
    raise AssertionError("buffer callback exception swallowed")
assert parser.buffer_used == 0

# An external parser inherits configuration but not pending character data.
parser = expat.ParserCreate()
parser.buffer_size = 64
parser.CharacterDataHandler = lambda text: None
child = parser.ExternalEntityParserCreate("context")
assert child.buffer_text is True and child.buffer_size == 64
assert child.buffer_used == 0
assert child.CharacterDataHandler is parser.CharacterDataHandler
child.buffer_text = False
assert parser.buffer_text is True

# An overflow-triggered flush rechecks callback changes before delivering the
# new backend event. Disabling buffering must not discard that new text;
# removing the character handler must discard it.
for remove_handler in (False, True):
    parser = expat.ParserCreate()
    parser.buffer_size = 2
    delivered = []

    def change_during_overflow(text):
        delivered.append(text)
        if len(delivered) == 1:
            if remove_handler:
                parser.CharacterDataHandler = None
            else:
                parser.buffer_text = False

    parser.CharacterDataHandler = change_during_overflow
    parser.Parse(b"<r>a<x/>bc</r>", True)
    assert delivered == (["a"] if remove_handler else ["a", "bc"])
    assert parser.buffer_used == 0

# Callback failure clears all handlers and cannot resume queued events on a
# subsequent Parse call, regardless of character buffering.
for buffered in (False, True):
    parser = expat.ParserCreate()
    parser.buffer_text = buffered
    parser.StartElementHandler = lambda *args: None
    parser.EndElementHandler = lambda *args: None

    def fail_character_callback(text):
        raise ValueError("stop parser")

    parser.CharacterDataHandler = fail_character_callback
    try:
        parser.Parse(b"<r>x<a/>y", False)
    except ValueError as exc:
        assert str(exc) == "stop parser"
    else:
        raise AssertionError("callback exception swallowed")
    assert parser.CharacterDataHandler is None
    assert parser.StartElementHandler is None
    assert parser.EndElementHandler is None
    assert parser.buffer_used == 0
    try:
        parser.Parse(b"</r>", True)
    except expat.ExpatError as exc:
        assert exc.code == expat.errors.codes[expat.errors.XML_ERROR_FINISHED]
    else:
        raise AssertionError("aborted parser resumed")

# A callback return conversion error is not a callback exception: Expat keeps
# the handlers and permits a subsequent Parse call to finish the document.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
invalid_result = lambda *args: object()
parser.ExternalEntityRefHandler = invalid_result
try:
    parser.Parse(b'<!DOCTYPE r SYSTEM "a"><r/>', False)
except TypeError:
    pass
else:
    raise AssertionError("non-integer external handler result accepted")
assert parser.ExternalEntityRefHandler is invalid_result
assert parser.Parse(b"", True) == 1
