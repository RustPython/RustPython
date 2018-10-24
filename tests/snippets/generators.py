
r = []

def make_numbers():
    yield 1
    yield 2
    r.append(42)
    yield 3

for a in make_numbers():
    r.append(a)

assert r == [1, 2, 42, 3]

r = list(x for x in [1, 2, 3])
assert r == [1, 2, 3]
