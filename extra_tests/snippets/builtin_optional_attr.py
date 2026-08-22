def assert_raises(exc_type, func, *args):
    try:
        func(*args)
    except exc_type:
        return
    except BaseException as exc:
        raise AssertionError(
            f"expected {exc_type.__name__}, got {type(exc).__name__}"
        ) from exc
    raise AssertionError(f"expected {exc_type.__name__}")


class Plain:
    class_attr = "class"


plain = Plain()
plain.instance_attr = "instance"
sentinel = object()

assert hasattr(plain, "class_attr")
assert hasattr(plain, "instance_attr")
assert not hasattr(plain, "missing")
assert getattr(plain, "class_attr", sentinel) == "class"
assert getattr(plain, "instance_attr", sentinel) == "instance"
assert getattr(plain, "missing", sentinel) is sentinel
assert_raises(AttributeError, getattr, plain, "missing")
assert_raises(TypeError, hasattr, plain, 1)
assert_raises(TypeError, getattr, plain, 1, sentinel)


getattribute_calls = []


class CustomGetattribute:
    def __getattribute__(self, name):
        getattribute_calls.append(name)
        if name == "missing":
            raise AttributeError("custom miss")
        if name == "bad":
            raise ValueError("custom error")
        return f"value:{name}"


custom_getattribute = CustomGetattribute()
assert not hasattr(custom_getattribute, "missing")
assert getattr(custom_getattribute, "missing", sentinel) is sentinel
assert getattribute_calls == ["missing", "missing"]
assert getattr(custom_getattribute, "present", sentinel) == "value:present"
assert_raises(ValueError, hasattr, custom_getattribute, "bad")


getattr_calls = []


class CustomGetattr:
    def __getattr__(self, name):
        getattr_calls.append(name)
        if name == "present":
            return "fallback value"
        if name == "bad":
            raise SystemExit("custom error")
        raise AttributeError(name)


custom_getattr = CustomGetattr()
assert getattr(custom_getattr, "present", sentinel) == "fallback value"
assert not hasattr(custom_getattr, "missing")
assert getattr_calls == ["present", "missing"]
assert_raises(SystemExit, hasattr, custom_getattr, "bad")


class AttributeErrorSubclass(AttributeError):
    pass


class AttributeErrorDescriptor:
    def __get__(self, obj, owner):
        raise AttributeError("descriptor miss")


class AttributeErrorSubclassDescriptor:
    def __get__(self, obj, owner):
        raise AttributeErrorSubclass("descriptor miss")


class ValueErrorDescriptor:
    def __get__(self, obj, owner):
        raise ValueError("descriptor error")


class DescriptorContainer:
    attribute_error = AttributeErrorDescriptor()
    attribute_error_subclass = AttributeErrorSubclassDescriptor()
    value_error = ValueErrorDescriptor()


descriptor_container = DescriptorContainer()
assert not hasattr(descriptor_container, "attribute_error")
assert getattr(descriptor_container, "attribute_error", sentinel) is sentinel
assert not hasattr(descriptor_container, "attribute_error_subclass")
assert getattr(descriptor_container, "attribute_error_subclass", sentinel) is sentinel
assert_raises(ValueError, hasattr, descriptor_container, "value_error")


class Dynamic:
    pass


dynamic = Dynamic()
assert not hasattr(dynamic, "missing")
dynamic_calls = []


def dynamic_getattribute(self, name):
    dynamic_calls.append(name)
    if name == "missing":
        raise AttributeError(name)
    return object.__getattribute__(self, name)


Dynamic.__getattribute__ = dynamic_getattribute
assert not hasattr(dynamic, "missing")
assert dynamic_calls == ["missing"]
del Dynamic.__getattribute__
assert not hasattr(dynamic, "missing")
