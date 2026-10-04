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
