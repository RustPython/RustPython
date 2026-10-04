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
