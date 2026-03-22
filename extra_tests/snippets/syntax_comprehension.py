
x = [1, 2, 3]

y = [a*a+1 for a in x]
assert y == [2, 5, 10]

z = [(b, c) for b in x for c in y]
# print(z)
assert z == [
    (1, 2), (1, 5), (1, 10),
    (2, 2), (2, 5), (2, 10),
    (3, 2), (3, 5), (3, 10)]

v = {b * 2 for b in x}
assert v == {2, 6, 4}

u = {str(b): b-2 for b in x}
assert u['3'] == 1
assert u['1'] == -1

y = [a+2 for a in x if a % 2]
print(y)
assert y == [3, 5]

z = [(9,), (10,)]
w = [x for x, in z]
assert w == [9, 10]

# generator targets shouldn't affect scopes out of comprehensions
[a for a in range(5)]
assert 'a' not in locals()
assert 'a' not in globals()

[b for a, b in [(1, 1), (2, 2)]]
assert 'b' not in locals()
assert 'b' not in globals()

{b: c for b, c in {1: 2}.items()}
assert 'b' not in locals()
assert 'c' not in locals()
assert 'b' not in globals()
assert 'c' not in globals()


def f():
    # Test no panic occurred.
    [[x := 1 for j in range(5)] for i in range(5)]


# Nested inlined comprehensions with lambda in the first iterator expression.
# The lambda's sub_table must be consumed before the inner comprehension's
# sub_table is spliced in, otherwise scope ordering is wrong.
def test_nested_comp_with_lambda():
    import itertools
    offsets = {0: [0], 1: [1], 3: [2]}
    grouped = [
        [x for _, x in group]
        for _, group in itertools.groupby(
            enumerate(sorted(offsets.keys())), lambda x: x[1] - x[0]
        )
    ]
    assert grouped == [[0, 1], [3]], f"got {grouped}"

test_nested_comp_with_lambda()


# Nested inlined comprehensions with throwaway `_` in both levels.
def test_nested_comp_underscore():
    data = [(1, "a", "x"), (2, "b", "y")]
    result = [[v for _, v in zip(range(2), row)] for _, *row in data]
    assert result == [["a", "x"], ["b", "y"]], f"got {result}"

test_nested_comp_underscore()


# Simple nested inlined comprehensions.
def test_simple_nested_comp():
    result = [[j * i for j in range(3)] for i in range(3)]
    assert result == [[0, 0, 0], [0, 1, 2], [0, 2, 4]]

test_simple_nested_comp()
