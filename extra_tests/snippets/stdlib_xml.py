import xml.sax
from xml.parsers import expat

from testutils import assert_raises

assert expat.XML_PARAM_ENTITY_PARSING_NEVER == 0
assert expat.XML_PARAM_ENTITY_PARSING_UNLESS_STANDALONE == 1
assert expat.XML_PARAM_ENTITY_PARSING_ALWAYS == 2

parser = expat.ParserCreate()
for value in (0, 1, 2, 3, -1, True):
    assert parser.SetParamEntityParsing(value) == 1

for value in ("x", None):
    with assert_raises(TypeError):
        parser.SetParamEntityParsing(value)

with assert_raises(OverflowError):
    parser.SetParamEntityParsing(2**100)

assert parser.GetBase() is None
assert parser.SetBase("example.xml") is None
assert parser.GetBase() == "example.xml"
for value in (b"example.xml", None, 123):
    with assert_raises(TypeError):
        parser.SetBase(value)


class Handler(xml.sax.handler.ContentHandler):
    def __init__(self):
        self.events = []

    def startElement(self, name, attrs):
        self.events.append(("start", name))

    def endElement(self, name):
        self.events.append(("end", name))


handler = Handler()
xml.sax.parseString("<main><child /></main>", handler)
assert handler.events == [
    ("start", "main"),
    ("start", "child"),
    ("end", "child"),
    ("end", "main"),
]
