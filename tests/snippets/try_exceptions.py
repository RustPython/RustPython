from testutils import assertRaises

try:
    raise BaseException()
except BaseException as ex:
    print(ex)
    print(type(ex))
    # print(ex.__traceback__)
    # print(type(ex.__traceback__))

try:
    raise ZeroDivisionError
except ZeroDivisionError as ex:
    pass

class E(Exception):
    def __init__(self):
        asdf

try:
    raise E
except NameError as ex:
    pass

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

cause = None
try:
    try:
        raise ZeroDivisionError
    except ZeroDivisionError as ex:
        assert ex.__cause__ == None
        cause = ex
        raise NameError from ex
except NameError as ex2:
    assert ex2.__cause__ == cause
    assert ex2.__context__ == cause

try:
    raise ZeroDivisionError from None
except ZeroDivisionError as ex:
    assert ex.__cause__ == None

try:
    raise ZeroDivisionError
except ZeroDivisionError as ex:
    assert ex.__cause__ == None

with assertRaises(TypeError):
    raise ZeroDivisionError from 5

try:
    raise ZeroDivisionError from NameError
except ZeroDivisionError as ex:
    assert type(ex.__cause__) == NameError

with assertRaises(NameError):
    try:
        raise NameError
    except:
        raise

with assertRaises(RuntimeError):
    raise

context = None
try:
    try:
        raise ZeroDivisionError
    except ZeroDivisionError as ex:
        assert ex.__context__ == None
        context = ex
        raise NameError
except NameError as ex2:
    assert ex2.__context__ == context
    assert type(ex2.__context__) == ZeroDivisionError

try:
    raise ZeroDivisionError
except ZeroDivisionError as ex:
    assert ex.__context__ == None

try:
    raise ZeroDivisionError from NameError
except ZeroDivisionError as ex:
    assert type(ex.__cause__) == NameError
    assert ex.__context__ == None

try:
    try:
        raise ZeroDivisionError
    except ZeroDivisionError as ex:
        pass
    finally:
        raise NameError
except NameError as ex2:
    assert ex2.__context__ == None

def f():
    raise

with assertRaises(ZeroDivisionError):
    try:
        1/0
    except:
        f()

with assertRaises(ZeroDivisionError):
    try:
        1/0
    except ZeroDivisionError:
        try:
            raise
        except NameError:
            pass
        raise

# Regression https://github.com/RustPython/RustPython/issues/867
for _ in [1, 2]:
    try:
        raise ArithmeticError()
    except ArithmeticError as e:
        continue
