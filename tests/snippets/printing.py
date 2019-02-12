print(2 + 3)

try:
    print('test', end=4)
except TypeError:
    pass
else:
    assert False, "Expected TypeError on wrong type passed to end"

try:
    print('test', sep=['a'])
except TypeError:
    pass
else:
    assert False, "Expected TypeError on wrong type passed to sep"
