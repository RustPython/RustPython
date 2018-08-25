

try:
    raise BaseException()
except BaseException as ex:
    print(ex)
    print(type(ex))
    # print(ex.__traceback__)
    # print(type(ex.__traceback__))


l = []
try:
    l.append(1)
    assert 0
    l.append(2)
except:
    l.append(3)
    print('boom')
finally:
    l.append(4)
    print('kablam')
assert l == [1, 3, 4]


l = []
try:
    l.append(1)
    assert 0
    l.append(2)
except AssertionError as ex:
    l.append(3)
    print('boom', type(ex))
finally:
    l.append(4)
    print('kablam')
assert l == [1, 3, 4]

l = []
try:
    l.append(1)
    assert 1
    l.append(2)
except AssertionError as ex:
    l.append(3)
    print('boom', type(ex))
finally:
    l.append(4)
    print('kablam')
assert l == [1, 2, 4]

l = []
try:
    try:
        l.append(1)
        assert 0
        l.append(2)
    finally:
        l.append(3)
        print('kablam')
except AssertionError as ex:
    l.append(4)
    print('boom', type(ex))
assert l == [1, 3, 4]

l = []
try:
    l.append(1)
    fubar
    l.append(2)
except NameError as ex:
    l.append(3)
    print('boom', type(ex))
assert l == [1, 3]


l = []
try:
    l.append(1)
    raise 1
except TypeError as ex:
    l.append(3)
    print('boom', type(ex))
assert l == [1, 3]
