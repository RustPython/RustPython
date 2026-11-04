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

# Brackets in a quoted external identifier are not internal-subset delimiters.
parser = expat.ParserCreate()
parser.Parse('<!DOCTYPE r SYSTEM "a]b["><r/>', True)

# A handler does not make an undeclared entity valid when there is no external
# subset that could provide its declaration.
parser = expat.ParserCreate()
parser.SkippedEntityHandler = lambda *_: None
try:
    parser.Parse("<!DOCTYPE r []><r>&missing;</r>", True)
except expat.ExpatError as error:
    assert error.code == expat.errors.codes["undefined entity"]
else:
    raise AssertionError("accepted an undeclared entity")

# Subset delimiters inside comments are ignored.
parser = expat.ParserCreate()
element_declarations = []
parser.ElementDeclHandler = lambda name, model: element_declarations.append(
    (name, model)
)
parser.Parse("<!DOCTYPE r [<!-- ] --><!ELEMENT r EMPTY>]><r/>", True)
assert len(element_declarations) == 1

# SYSTEM in the root name is not an external identifier.
parser = expat.ParserCreate()
parser.SkippedEntityHandler = lambda *_: None
try:
    parser.Parse("<!DOCTYPE SYSTEM><SYSTEM>&missing;</SYSTEM>", True)
except expat.ExpatError as error:
    assert error.code == expat.errors.codes["undefined entity"]
else:
    raise AssertionError("treated the root name as an external subset")

# Undeclared references buffered after the DOCTYPE reach the skipped handler
# when an external subset may declare them.
parser = expat.ParserCreate()
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse('<!DOCTYPE r SYSTEM "external.dtd"><r>&later;</r>', True)
assert skipped == [("later", 0)]

# An undeclared reference supplied by an external subset is silently skipped
# when no SkippedEntityHandler is installed.
parser = expat.ParserCreate()
parser.Parse('<!DOCTYPE r SYSTEM "external.dtd"><r>&missing;</r>', True)

# XML names in skipped entity references may contain non-ASCII characters.
parser = expat.ParserCreate()
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse('<!DOCTYPE r SYSTEM "external.dtd"><r>&é;</r>', True)
assert skipped == [("é", 0)]

# Enumerated ATTLIST types are reported as one type value.
parser = expat.ParserCreate()
attributes = []
parser.AttlistDeclHandler = lambda *args: attributes.append(args)
parser.Parse('<!DOCTYPE r [<!ATTLIST r a (x | y) "x">]><r/>', True)
assert attributes == [("r", "a", "(x|y)", "x", 0)]

parser = expat.ParserCreate()
attributes = []
parser.AttlistDeclHandler = lambda *args: attributes.append(args)
parser.Parse("<!DOCTYPE r [<!ATTLIST r a CDATA #FIXED 'x'>]><r/>", True)
assert attributes == [("r", "a", "CDATA", "x", 1)]

# Element content models retain their nested sequence and quantifier data.
parser = expat.ParserCreate()
models = []
parser.ElementDeclHandler = lambda name, model: models.append((name, model))
parser.Parse(
    "<!DOCTYPE r [<!ELEMENT r (a,b?)><!ELEMENT a EMPTY><!ELEMENT b ANY>]><r/>", True
)
assert models == [
    ("r", (6, 0, None, ((4, 0, "a", ()), (4, 1, "b", ())))),
    ("a", (1, 0, None, ())),
    ("b", (2, 0, None, ())),
]

parser = expat.ParserCreate()
models = []
parser.ElementDeclHandler = lambda name, model: models.append(model)
parser.Parse("<!DOCTYPE r [<!ELEMENT r (#PCDATA|a)*>]><r/>", True)
assert models == [(3, 2, None, ((4, 0, "a", ()),))]

parser = expat.ParserCreate()
models = []
parser.ElementDeclHandler = lambda name, model: models.append(model)
parser.Parse("<!DOCTYPE r [<!ELEMENT r (#PCDATA)>]><r/>", True)
assert models == [(3, 0, None, ())]

# Quoted SYSTEM is an internal entity value, not an external identifier.
parser = expat.ParserCreate()
entities = []
parser.EntityDeclHandler = lambda *args: entities.append(args)
parser.Parse("<!DOCTYPE r [<!ENTITY word 'SYSTEM'>]><r>&word;</r>", True)
assert entities[0][2] == "SYSTEM"

# An external subset without parameter-entity references reports non-standalone once.
parser = expat.ParserCreate()
not_standalone = []
parser.NotStandaloneHandler = lambda: not_standalone.append(True) or 1
parser.Parse('<!DOCTYPE r SYSTEM "external.dtd"><r/>', True)
assert not_standalone == [True]

# ALWAYS preserves an undeclared parameter-entity reference in a standalone
# document so the parser can reject it.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
try:
    parser.Parse(
        '<?xml version="1.0" standalone="yes"?><!DOCTYPE r [%missing;]><r/>', True
    )
except expat.ExpatError:
    pass
else:
    raise AssertionError(
        "accepted an undeclared parameter entity in a standalone document"
    )

# External parsed entities are not permitted in attribute values.
parser = expat.ParserCreate()
try:
    parser.Parse('<!DOCTYPE r [<!ENTITY e SYSTEM "x">]><r a="&e;"/>', True)
except expat.ExpatError as error:
    assert error.code == expat.errors.codes["reference to external entity in attribute"]
else:
    raise AssertionError("accepted an external entity in an attribute")

# Literal noncharacters used by the parser's internal marker format remain
# visible as character data when they occur in the XML document.
parser = expat.ParserCreate()
text = []
parser.CharacterDataHandler = text.append
parser.Parse("<r>a\ufdd0X\ufdd1b</r>", True)
assert "".join(text) == "a\ufdd0X\ufdd1b"

# XML declaration callbacks preserve which optional attributes were present.
parser = expat.ParserCreate()
declarations = []
parser.XmlDeclHandler = lambda *args: declarations.append(args)
parser.Parse('<?xml version="1.0"?><r/>', True)
assert declarations == [("1.0", None, -1)]

parser = expat.ParserCreate()
declarations = []
parser.XmlDeclHandler = lambda *args: declarations.append(args)
parser.Parse('<?xml version="1.0" encoding="UTF-16LE"?><r/>'.encode("utf-16le"), True)
assert declarations == [("1.0", "UTF-16LE", -1)]

# A long prolog comment exercises the incremental prefix scanner.
parser = expat.ParserCreate()
parser.Parse("<!--" + ("x" * (1024 * 1024)) + "--><r/>", True)

# Complete prolog comments are reported in the Parse call that provides them.
parser = expat.ParserCreate()
comments = []
parser.CommentHandler = comments.append
parser.Parse("<!--x-->", False)
assert comments == ["x"]
parser.Parse("<r/>", True)

# The UTF-16 declaration remains buffered until its terminator arrives.
parser = expat.ParserCreate()
declarations = []
parser.XmlDeclHandler = lambda *args: declarations.append(args)
utf16_document = '<?xml version="1.0" encoding="UTF-16LE"?><r/>'.encode("utf-16le")
parser.Parse(utf16_document[:4], False)
assert declarations == []
parser.Parse(utf16_document[4:], True)
assert declarations == [("1.0", "UTF-16LE", -1)]

# DTD comments and processing instructions are delivered in source order.
parser = expat.ParserCreate()
lexical = []
parser.CommentHandler = lambda text: lexical.append(("comment", text))
parser.ProcessingInstructionHandler = lambda *args: lexical.append(("pi", *args))
parser.Parse("<!DOCTYPE r [<!--inside--><?pi data?><!ELEMENT r EMPTY>]><r/>", True)
assert lexical == [("comment", "inside"), ("pi", "pi", "data")]

# Parameter entity replacement markup contributes DTD callbacks when enabled.
parser = expat.ParserCreate()
elements = []
parser.ElementDeclHandler = lambda name, model: elements.append(name)
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
parser.Parse('<!DOCTYPE r [<!ENTITY % decl "<!ELEMENT r EMPTY>">%decl;]><r/>', True)
assert elements == ["r"]

# NEVER mode leaves declarations inside parameter entities inactive.
parser = expat.ParserCreate()
attributes = []
parser.StartElementHandler = lambda name, attrs: attributes.append(attrs)
parser.Parse(
    "<!DOCTYPE r [<!ENTITY % attrs \"<!ATTLIST r a CDATA 'x'>\">%attrs;]><r/>",
    True,
)
assert attributes == [{}]

# Quoted ATTLIST keywords are literal defaults, not declaration markers.
parser = expat.ParserCreate()
attlists = []
parser.AttlistDeclHandler = lambda *args: attlists.append(args)
parser.Parse("<!DOCTYPE r [<!ATTLIST r a CDATA '#REQUIRED'>]><r/>", True)
assert attlists == [("r", "a", "CDATA", "#REQUIRED", 0)]

# Public identifier whitespace is normalized before declaration callbacks.
parser = expat.ParserCreate()
notations = []
parser.NotationDeclHandler = lambda *args: notations.append(args)
parser.Parse('<!DOCTYPE r [<!NOTATION n PUBLIC " a  b ">]><r/>', True)
assert notations[0][3] == "a b"

# The first declaration of an entity wins for callbacks and replacement.
parser = expat.ParserCreate()
entities = []
parser.EntityDeclHandler = lambda *args: entities.append(args)
parser.Parse("<!DOCTYPE r [<!ENTITY e 'x'><!ENTITY e 'y'>]><r>&e;</r>", True)
assert [entity[2] for entity in entities] == ["x"]

# An unresolved external-subset entity in an attribute is skipped.
parser = expat.ParserCreate()
attributes = []
parser.StartElementHandler = lambda name, attrs: attributes.append(attrs)
parser.Parse('<!DOCTYPE r SYSTEM "external.dtd"><r a="x&missing;y"/>', True)
assert attributes == [{"a": "xy"}]

# A Unicode parameter-entity name remains declared when its reference is expanded.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
entities = []
parser.EntityDeclHandler = lambda *args: entities.append(args)
parser.Parse("<!DOCTYPE r [<!ENTITY % é \"<!ENTITY e 'ok'>\">%é;]><r>&e;</r>", True)
assert any(entity[0] == "e" and entity[2] == "ok" for entity in entities)

# Declaration-looking PI data is not replayed as a DTD declaration.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
entities = []
parser.EntityDeclHandler = lambda *args: entities.append(args)
try:
    parser.Parse("<!DOCTYPE r [<?meta <!ENTITY e SYSTEM 'x'?>]><r>&e;</r>", True)
except expat.ExpatError as error:
    assert error.code == expat.errors.codes["undefined entity"]
else:
    raise AssertionError("parsed declaration-looking PI data as an entity")
assert entities == []

# PUBLIC entity system identifiers retain their literal whitespace.
parser = expat.ParserCreate()
entities = []
parser.EntityDeclHandler = lambda *args: entities.append(args)
parser.Parse("<!DOCTYPE r [<!ENTITY e PUBLIC ' a  b ' '  system  id  '>]><r/>", True)
assert entities[0][4:6] == ("  system  id  ", "a b")

# Parameter entity references in DTD processing instructions are not scanned
# or modified as declarations/references.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
pis = []
parser.ProcessingInstructionHandler = lambda *args: pis.append(args)
parser.Parse("<!DOCTYPE r SYSTEM 'x' [<?pi %missing;?>]><r/>", True)
assert pis == [("pi", "%missing;")]

# Undefined parameter entities in enabled parsing mode reach the skipped
# entity handler with parameter-entity flag 1.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse("<!DOCTYPE r [%missing;]><r/>", True)
assert skipped == [("missing", 1)]

# A parameter-entity reference is skipped if its declaration appears later.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse("<!DOCTYPE r [%x;<!ENTITY % x ''>]><r/>", True)
assert skipped == [("x", 1)]

# The DOCTYPE start callback precedes skipped parameter-entity callbacks.
parser = expat.ParserCreate()
parser.SetParamEntityParsing(expat.XML_PARAM_ENTITY_PARSING_ALWAYS)
doctype_events = []
parser.StartDoctypeDeclHandler = lambda *args: doctype_events.append("start")
parser.SkippedEntityHandler = lambda *args: doctype_events.append("skipped")
parser.EndDoctypeDeclHandler = lambda: doctype_events.append("end")
parser.Parse("<!DOCTYPE r [%missing;]><r/>", True)
assert doctype_events == ["start", "skipped", "end"]

# BOM-prefixed UTF-16 remains valid when the BOM arrives in its own Parse call.
parser = expat.ParserCreate()
declarations = []
parser.XmlDeclHandler = lambda *args: declarations.append(args)
utf16_document = '<?xml version="1.0" encoding="UTF-16LE"?><r/>'.encode("utf-16le")
parser.Parse(b"\xff\xfe", False)
parser.Parse(utf16_document, True)
assert declarations == [("1.0", "UTF-16LE", -1)]

# A UTF-8 BOM split across Parse calls still allows XML declaration dispatch.
parser = expat.ParserCreate()
declarations = []
parser.XmlDeclHandler = lambda *args: declarations.append(args)
parser.Parse(b"\xef", False)
parser.Parse(b"\xbb", False)
parser.Parse(b'\xbf<?xml version="1.0"?><r/>', True)
assert declarations == [("1.0", None, -1)]

# Parameter and general entities use separate declaration namespaces.
parser = expat.ParserCreate()
external = []
parser.ExternalEntityRefHandler = lambda *args: external.append(args) or 1
parser.Parse(
    '<!DOCTYPE r [<!ENTITY % e SYSTEM "parameter.dtd">'
    '<!ENTITY e SYSTEM "general.dtd">]><r>&e;</r>',
    True,
)
assert external[0][2] == "general.dtd"

# Skipped names in ISO-8859-1 input are decoded using the document encoding.
parser = expat.ParserCreate()
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse(
    b'<?xml version="1.0" encoding="ISO-8859-1"?>'
    b'<!DOCTYPE r SYSTEM "external.dtd"><r>&\xe9;</r>',
    True,
)
assert skipped == [("é", 0)]

# References in internal entity values can be supplied by an external subset.
parser = expat.ParserCreate()
skipped = []
parser.SkippedEntityHandler = lambda *args: skipped.append(args)
parser.Parse(
    '<!DOCTYPE r SYSTEM "external.dtd" [<!ENTITY e "&missing;">]><r>&e;</r>',
    True,
)
assert skipped == [("missing", 0)]
