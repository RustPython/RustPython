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
