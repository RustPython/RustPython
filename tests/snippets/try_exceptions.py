

try:
    raise BaseException()
except BaseException as ex:
    print(ex)
    print(type(ex))
    # print(ex.__traceback__)
    # print(type(ex.__traceback__))


try:
    assert 0
except:
    print('boom')
finally:
    print('kablam')

try:
    assert 0
except AssertionError as ex:
    print('boom', type(ex))
finally:
    print('kablam')
