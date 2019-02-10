x = [1, 2, 3]
assert x[0] == 1
assert x[1] == 2
# assert x[7]

y = [2, *x]
assert y == [2, 1, 2, 3]

y.extend(x)
assert y == [2, 1, 2, 3, 1, 2, 3]

assert x * 0 == [], "list __mul__ by 0 failed"
assert x * -1 == [], "list __mul__ by -1 failed"
assert x * 2 == [1, 2, 3, 1, 2, 3], "list __mul__ by 2 failed"

assert ['a', 'b', 'c'].index('b') == 1
assert [5, 6, 7].index(7) == 2
try:
    ['a', 'b', 'c'].index('z')
except ValueError:
    pass
else:
    assert False, "ValueError was not raised"

x = [[1,0,-3], 'a', 1]
y = [[3,2,1], 'z', 2]
assert x < y, "list __lt__ failed"

x = [5, 13, 31]
y = [1, 10, 29]
assert x > y, "list __gt__ failed"


assert [1,2,'a'].pop() == 'a', "list pop failed"
try:
    [].pop()
except IndexError:
    pass
else:
    assert False, "IndexError was not raised"

recursive = []
recursive.append(recursive)
assert repr(recursive) == "[[...]]"
