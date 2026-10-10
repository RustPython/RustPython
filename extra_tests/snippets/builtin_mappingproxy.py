from testutils import assert_raises


class A(dict):
    def a():
        pass

    def b():
        pass


assert A.__dict__["a"] == A.a
with assert_raises(KeyError) as cm:
    A.__dict__["not here"]

assert cm.exception.args[0] == "not here"

assert "b" in A.__dict__
assert "c" not in A.__dict__

assert "__dict__" in A.__dict__

assert A.__dict__.get("not here", "default") == "default"
assert A.__dict__.get("a", "default") is A.a
assert A.__dict__.get("not here") is None


# A mappingproxy preserves arbitrary equality-comparison results unchanged.
def check_mappingproxy_comparison_results():
    import operator
    from types import MappingProxyType

    from testutils import assert_raises

    class Result:
        def __bool__(self):
            raise AssertionError("comparison result must not be coerced to bool")

    result = Result()
    calls = []

    def compare(name):
        def method(self, other):
            calls.append(name)
            return result

        return method

    class Compared:
        __eq__ = compare("eq")
        __ne__ = compare("ne")
        __lt__ = compare("lt")
        __le__ = compare("le")
        __gt__ = compare("gt")
        __ge__ = compare("ge")

    class CustomMapping(Compared, dict):
        pass

    for operation, reflected in (
        (operator.eq, "eq"),
        (operator.ne, "ne"),
    ):
        calls.clear()
        assert operation(MappingProxyType(CustomMapping()), {}) is result
        assert calls == [operation.__name__], calls
        calls.clear()
        assert operation(MappingProxyType({}), Compared()) is result
        assert calls == [reflected], calls

    for operation, reflected in (
        (operator.lt, "gt"),
        (operator.le, "ge"),
        (operator.gt, "lt"),
        (operator.ge, "le"),
    ):
        calls.clear()
        with assert_raises(TypeError):
            operation(MappingProxyType(CustomMapping()), {})
        assert calls == [], calls
        calls.clear()
        assert operation(MappingProxyType({}), Compared()) is result
        assert calls == [reflected], calls


check_mappingproxy_comparison_results()
