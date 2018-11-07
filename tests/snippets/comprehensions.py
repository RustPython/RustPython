
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
# TODO: how to check set equality?
# assert v == {2, 6, 4}

u = {str(b): b-2 for b in x}
assert u['3'] == 1
assert u['1'] == -1

y = [a+2 for a in x if a % 2]
print(y)
assert y == [3, 5]
