import array
import xml.sax
from xml.etree.ElementTree import Element
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

events = []
parser = expat.ParserCreate()
parser.ProcessingInstructionHandler = lambda target, data: events.append(
    ("processing-instruction", target, data)
)
parser.CommentHandler = lambda data: events.append(("comment", data))
parser.StartCdataSectionHandler = lambda: events.append(("start-cdata",))
parser.CharacterDataHandler = lambda data: events.append(("characters", data))
parser.EndCdataSectionHandler = lambda: events.append(("end-cdata",))
parser.Parse(
    "<?target data?><?empty?><root><!--comment--><![CDATA[text]]></root>", True
)
assert events == [
    ("processing-instruction", "target", "data"),
    ("processing-instruction", "empty", ""),
    ("comment", "comment"),
    ("start-cdata",),
    ("characters", "text"),
    ("end-cdata",),
]

root = Element("root")
for value, type_name in (
    (None, "None"),
    (1, "int"),
    ("child", "str"),
    (array.array("b"), "array.array"),
    (type("A" * 60, (), {})(), "A" * 50),
    (type("€" * 20, (), {})(), "€" * 16),
):
    with assert_raises(TypeError) as caught:
        root.remove(value)
    assert str(caught.exception) == (
        "remove() argument must be xml.etree.ElementTree.Element, not " + type_name
    )


def test_element_frozen_attrib_identity_bounded():
    class Frozen(frozendict):
        def __getitem__(self, key):
            raise AssertionError("Element.get must use the stored entries")

        def keys(self):
            raise AssertionError("Element.keys must use the stored entries")

        def items(self):
            raise AssertionError("Element.items must use the stored entries")

    for mapping in (frozendict(), frozendict(a=1), Frozen(a=1)):
        element = Element("root")
        element.attrib = mapping
        assert element.attrib is mapping
        assert element.get("a", 7) == (1 if mapping else 7)
        assert element.get("missing", 7) == 7
        assert element.keys() == (["a"] if mapping else [])
        assert element.items() == ([("a", 1)] if mapping else [])
        with assert_raises(AttributeError):
            del element.attrib
        assert element.attrib is mapping
        element.clear()
        assert type(element.attrib) is dict and element.attrib == {}
        element.set("new", 2)
        assert element.attrib == {"new": 2}
        assert dict(mapping) == ({"a": 1} if mapping else {})


def test_element_frozen_attrib_mutation_bounded():
    class Key:
        def __hash__(self):
            raise AssertionError("frozen assignment must fail before hashing")

    element = Element("root")
    mapping = frozendict(a=1)
    element.attrib = mapping
    for key in ("a", "new", Key()):
        with assert_raises(TypeError) as caught:
            element.set(key, 2)
        assert str(caught.exception) == (
            "frozendict object does not support item assignment"
        )
        assert element.attrib is mapping and mapping == {"a": 1}


def test_element_frozen_attrib_copy_state_bounded():
    import copy

    for mapping in (frozendict(), frozendict(a=[1])):
        element = Element("root")
        element.attrib = mapping
        shallow = copy.copy(element)
        assert shallow is not element and shallow.attrib is mapping
        deep = copy.deepcopy(element)
        assert deep is not element
        assert type(deep.attrib) is frozendict and deep.attrib == mapping
        if mapping:
            assert deep.attrib["a"] is not mapping["a"]
            deep.attrib["a"].append(2)
            assert mapping["a"] == [1]
        state = element.__getstate__()
        assert state["attrib"] is mapping
        restored = Element("restored")
        restored.__setstate__(state)
        assert restored.attrib is mapping


test_element_frozen_attrib_identity_bounded()
test_element_frozen_attrib_mutation_bounded()
test_element_frozen_attrib_copy_state_bounded()
