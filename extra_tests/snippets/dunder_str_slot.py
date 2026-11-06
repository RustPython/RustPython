from types import MethodWrapperType, WrapperDescriptorType

from testutils import assert_raises

# These native __str__ implementations use #[pymethod]. They must install
# tp_str and expose its slot wrapper, not an ordinary method descriptor.
for cls, expected in [(bytes, "b'abc'"), (bytearray, "bytearray(b'abc')")]:
    descriptor = cls.__dict__["__str__"]
    assert type(descriptor) is WrapperDescriptorType
    assert descriptor.__objclass__ is cls
    assert descriptor.__name__ == "__str__"
    value = cls(b"abc")
    assert type(value.__str__) is MethodWrapperType
    assert str(value) == descriptor(value) == value.__str__() == expected
    with assert_raises(TypeError):
        descriptor(object())

    class Subclass(cls):
        def __repr__(self):
            return "overridden repr"

    value = Subclass(b"abc")
    expected = "b'abc'" if cls is bytes else "Subclass(b'abc')"
    # Inherit the native str slot, rather than object.__str__ calling repr.
    assert str(value) == expected
    assert descriptor(value) == expected
    # Implicit str dispatch must use the type's slot, ignoring an instance
    # attribute that is visible to an ordinary explicit method call.
    value.__str__ = lambda: "instance shadow"
    assert value.__str__() == "instance shadow"
    assert str(value) == expected

    # Changing the Python class method must update its str slot. Removing it
    # must restore the inherited native slot, not the instance's attribute.
    Subclass.__str__ = lambda self: "class override"
    assert str(value) == "class override"
    assert descriptor(value) == expected
    del Subclass.__str__
    assert str(value) == expected
