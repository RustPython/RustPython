x = 0
for i in [1, 2, 3, 4]:
    x += 1

assert x == 4

for i in [1, 2, 3]:
    x = i + 5
else:
    x = 3

assert x == 3

y = []
for x, in [(9,), [2]]:
    y.append(x)

assert y == [9, 2], str(y)

y = []
for x, *z in [(9,88,'b'), [2, 'bla'], [None]*4]:
    y.append(z)

assert y == [[88, 'b'], ['bla'], [None]*3], str(y)
