class MyObject:
    pass


assert not MyObject() == MyObject()
assert MyObject() != MyObject()
myobj = MyObject()
assert myobj == myobj
assert not myobj != myobj

object.__subclasshook__(1) == NotImplemented

assert MyObject().__eq__(MyObject()) == NotImplemented
assert MyObject().__ne__(MyObject()) == NotImplemented
assert MyObject().__lt__(MyObject()) == NotImplemented
assert MyObject().__le__(MyObject()) == NotImplemented
assert MyObject().__gt__(MyObject()) == NotImplemented
assert MyObject().__ge__(MyObject()) == NotImplemented

obj = MyObject()

assert obj.__eq__(obj) is True
assert obj.__ne__(obj) is False

assert not hasattr(obj, "a")
obj.__dict__ = {"a": 1}
assert obj.a == 1

del obj.__dict__
d = obj.__dict__
assert isinstance(d, dict)
assert len(d) == 0

try:
    obj.a
    assert False, "AttributeError expected"
except AttributeError:
    pass

# Value inside the formatter goes through a different path of resolution.
# Check that it still works all the same
d = {
    0: "ab",
}
assert "ab ab" == "{k[0]} {vv}".format(k=d, vv=d[0])


from testutils import assert_raises


class AttributeHook:
    existing = "class"

    @property
    def absent(self):
        raise AttributeError("descriptor miss")

    @property
    def invalid(self):
        raise ValueError("descriptor error")

    def __getattr__(self, name):
        if name == "refused":
            raise AttributeError("hook refusal")
        return "hook:" + name


class InheritedAttributeHook(AttributeHook):
    pass


hooked = InheritedAttributeHook()
hooked.instance = "instance"
assert hooked.instance == "instance"
assert hooked.existing == "class"
assert hooked.missing == "hook:missing"
assert hooked.absent == "hook:absent"
with assert_raises(ValueError):
    hooked.invalid
with assert_raises(AttributeError) as caught:
    hooked.refused
assert caught.exception.__context__ is None
assert not hasattr(hooked, "refused")
assert getattr(hooked, "refused", "default") == "default"

# Changing __getattribute__ must invalidate the generic lookup path.
InheritedAttributeHook.__getattribute__ = lambda self, name: "custom:" + name
assert hooked.existing == "custom:existing"
InheritedAttributeHook.__getattribute__ = object.__getattribute__
assert hooked.existing == "class"
assert hooked.missing == "hook:missing"
del InheritedAttributeHook.__getattribute__
assert hooked.existing == "class"


class SlotAttributeHook:
    __slots__ = ("value",)

    def __getattr__(self, name):
        return "unset"


slotted_hook = SlotAttributeHook()
assert slotted_hook.value == "unset"
slotted_hook.value = "set"
assert slotted_hook.value == "set"

# Retain the raw hook across callbacks, but delay its descriptor binding.
for lookup_kind in ("property", "getattribute"):
    for change in ("replace", "delete"):
        hook_events = []

        class ChangingHookDescriptor:
            def __get__(self, obj, owner):
                hook_events.append("bind")
                return lambda name: "original:" + name

        class ChangingAttributeHook:
            __getattr__ = ChangingHookDescriptor()

        def changing_lookup(self, name):
            hook_events.append("lookup")
            if change == "replace":
                ChangingAttributeHook.__getattr__ = lambda self, name: "replacement"
            else:
                del ChangingAttributeHook.__getattr__
            raise AttributeError("lookup miss")

        if lookup_kind == "property":
            ChangingAttributeHook.value = property(
                lambda self: changing_lookup(self, "value")
            )
        else:
            ChangingAttributeHook.__getattribute__ = changing_lookup
        assert ChangingAttributeHook().value == "original:value"
        assert hook_events == ["lookup", "bind"]


class AddedAttributeHook:
    def __getattribute__(self, name):
        AddedAttributeHook.__getattr__ = lambda self, name: "added"
        raise AttributeError("initial miss")


with assert_raises(AttributeError):
    AddedAttributeHook().value
assert AddedAttributeHook().value == "added"


class RaisingAttributeHook(AttributeHook):
    def __getattr__(self, name):
        raise ValueError("hook error")


with assert_raises(ValueError) as caught:
    RaisingAttributeHook().absent
assert caught.exception.__context__ is None
try:
    raise RuntimeError("outer error")
except RuntimeError as outer:
    with assert_raises(ValueError) as caught:
        RaisingAttributeHook().absent
    assert caught.exception.__context__ is outer


# object.__getstate__ reads names from a cached __slotnames__ list
class NonStrSlotName:
    __slotnames__ = [1]


with assert_raises(TypeError) as caught:
    NonStrSlotName().__getstate__()
assert str(caught.exception) == "attribute name must be string, not 'int'"


class GrowingSlotNames:
    __slotnames__ = ["a", "b"]

    def __getattr__(self, name):
        if name == "a":
            GrowingSlotNames.__slotnames__.append("c")
            return 1
        raise AttributeError(name)


with assert_raises(RuntimeError):
    GrowingSlotNames().__getstate__()


class RaisingSlot:
    __slotnames__ = ["x"]

    def __getattr__(self, name):
        raise ValueError("boom")


with assert_raises(ValueError):
    RaisingSlot().__getstate__()


class MissingSlot:
    __slotnames__ = ["x", "y"]
    y = 2


assert MissingSlot().__getstate__() == (None, {"y": 2})
