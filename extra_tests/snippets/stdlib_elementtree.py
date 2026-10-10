import array
from xml.etree.ElementTree import Element

from testutils import assert_raises

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
