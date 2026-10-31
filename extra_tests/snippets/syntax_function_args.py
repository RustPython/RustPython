from testutils import assert_raises


def error(call, expected):
    try:
        call()
    except TypeError as exc:
        assert str(exc) == expected, (str(exc), expected)
    else:
        raise AssertionError("expected TypeError")


error(lambda: id(obj=1), "id() takes no keyword arguments")
error(lambda: id(**{"obj": 1}), "id() takes no keyword arguments")
error(lambda: id(1, 2, obj=1), "id() takes no keyword arguments")
error(lambda: list.append(x=1), "unbound method list.append() needs an argument")
error(lambda: list.append(**{"x": 1}), "unbound method list.append() needs an argument")
error(
    lambda: list.append(1, x=1),
    "descriptor 'append' for 'list' objects doesn't apply to a 'int' object",
)
error(lambda: list.append([], x=1), "list.append() takes no keyword arguments")
error(lambda: list.append([], **{"x": 1}), "list.append() takes no keyword arguments")
error(lambda: [].append(x=1), "list.append() takes no keyword arguments")
error(lambda: [].clear(1, x=1), "list.clear() takes no keyword arguments")


class Index:
    def __index__(self):
        raise AssertionError("conversion ran before keyword check")


error(lambda: bin(Index(), x=1), "bin() takes no keyword arguments")
assert id(1, **{}) == id(1)
assert round(1.25, ndigits=1) == 1.2
assert pow(2, exp=3) == 8
assert sorted([2, 1], reverse=True) == [2, 1]
assert sum([1, 2], start=4) == 7
values = []
list.append(values, 1)
values.append(2)
assert values == [1, 2]
list.clear(values)
assert values == []

# Keyword metadata must forward raw arguments without changing body validation.
for choose in (min, max):
    assert choose([], default=42) == 42
    assert choose([1, 2], key=lambda value: -value) == (2 if choose is min else 1)
    try:
        choose(bogus=1)
    except TypeError as exc:
        assert str(exc) == f"{choose.__name__} expected at least 1 argument, got 0"
    else:
        raise AssertionError("missing positional argument was accepted")
    try:
        choose(1, 2, default=0)
    except TypeError as exc:
        assert str(exc) == (
            f"Cannot specify a default for {choose.__name__}() "
            "with multiple positional arguments"
        )
    else:
        raise AssertionError("default with multiple arguments was accepted")

# Named fields in argument structs must override positional-only arguments.
assert eval("value", globals={"value": 42}) == 42
namespace = {}
exec("value = 42", globals=namespace)
assert namespace["value"] == 42
assert compile(source="42", filename="<inference>", mode="eval")
assert __import__(name="sys").__name__ == "sys"

from io import StringIO

output = StringIO()
print(1, 2, sep=":", end="!", file=output)
assert output.getvalue() == "1:2!"

# Methods using a keyword-only argument struct must keep accepting keywords.
values = [1, 2]
values.sort(reverse=True)
assert values == [2, 1]

# Definitions built without the macros infer their calling convention too.
assert int.__new__(int, "11", base=2) == 3

def sum(x, y):
    return x+y

def total(a, b, c, d):
    return sum(sum(a,b), sum(c,d))


assert total(1,1,1,1) == 4
assert total(1,2,3,4) == 10

assert sum(1, 1) == 2
assert sum(1, 3) == 4


def sum2y(x, y):
    return x+y*2


assert sum2y(1, 1) == 3
assert sum2y(1, 3) == 7


def va(a, b=2, *c, d, **e):
    assert a == 1
    assert b == 22
    assert c == (3, 4)
    assert d == 1337
    assert e['f'] == 42


va(1, 22, 3, 4, d=1337, f=42)

assert va.__defaults__ == (2,)
assert va.__kwdefaults__ is None


def va2(*args, **kwargs):
    assert args == (5, 4)
    assert len(kwargs) == 0

va2(5, 4)
x = (5, 4)
va2(*x)

va2(5, *x[1:])


def va3(x, *, a, b=2, c=9):
    return x + b + c


assert va3(1, a=1, b=10) == 20

with assert_raises(TypeError):
    va3(1, 2, 3, a=1, b=10)

with assert_raises(TypeError):
    va3(1, b=10)


assert va3.__defaults__ is None
kw_defaults = va3.__kwdefaults__
# assert va3.__kwdefaults__ == {'b': 2, 'c': 9}
assert set(kw_defaults) == {'b', 'c'}
assert kw_defaults['b'] == 2
assert kw_defaults['c'] == 9

x = {'f': 42, 'e': 1337}
y = {'d': 1337}
va(1, 22, 3, 4, **x, **y)

# star arg after keyword args:
def fubar(x, y, obj=None):
    assert x == 4
    assert y == 5
    assert obj == 6

rest = [4, 5]
fubar(obj=6, *rest)


# https://www.python.org/dev/peps/pep-0468/
def func(**kwargs):
    return list(kwargs.items())

empty_kwargs = func()
assert empty_kwargs == []

kwargs = func(a=1, b=2)
assert kwargs == [('a', 1), ('b', 2)]

kwargs = func(a=1, b=2, c=3)
assert kwargs == [('a', 1), ('b', 2), ('c', 3)]


def inc(n):
    return n + 1

with assert_raises(SyntaxError):
    exec("inc(n=1, n=2)")

with assert_raises(SyntaxError):
    exec("def f(a=1, b): pass")


def f(a):
    pass

x = {'a': 1}
y = {'a': 2}
with assert_raises(TypeError):
    f(**x, **y)


def f(a, b, /, c, d, *, e, f):
    return a + b + c + d + e + f

assert f(1,2,3,4,e=5,f=6) == 21
assert f(1,2,3,d=4,e=5,f=6) == 21
assert f(1,2,c=3,d=4,e=5,f=6) == 21
with assert_raises(TypeError):
    f(1,b=2,c=3,d=4,e=5,f=6)
with assert_raises(TypeError):
    f(a=1,b=2,c=3,d=4,e=5,f=6)
with assert_raises(TypeError):
    f(1,2,3,4,5,f=6)
with assert_raises(TypeError):
    f(1,2,3,4,5,6)


def test_keyword_dict_stored_hashes():
    class Keyword(str):
        def __hash__(self):
            assert not getattr(self, "hashed", False), "keyword was rehashed"
            self.hashed = True
            return str.__hash__(self)

    def accept(*, value):
        return value

    source = {Keyword("value"): 42}
    assert accept(**source) == 42
    with assert_raises(TypeError) as raised:
        accept(value=1, **source)
    assert str(raised.exception).endswith(
        "got multiple values for keyword argument 'value'"
    )

    class NonString:
        def __hash__(self):
            assert not getattr(self, "hashed", False), "non-string key was rehashed"
            self.hashed = True
            return 0

    source = {NonString(): 42}
    with assert_raises(TypeError) as raised:
        accept(**source)
    assert str(raised.exception) == "keywords must be strings"

    failure = RuntimeError("keyword comparison failed")

    class BadEquality(str):
        __hash__ = str.__hash__

        def __eq__(self, other):
            raise failure

    left = {BadEquality("value"): 1}
    right = {BadEquality("value"): 2}
    with assert_raises(RuntimeError) as raised:
        accept(**left, **right)
    assert raised.exception is failure


test_keyword_dict_stored_hashes()
