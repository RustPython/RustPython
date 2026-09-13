from testutils import assert_raises

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
        asdf  # noqa: F821

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

with assert_raises(TypeError):
    raise ZeroDivisionError from 5

try:
    raise ZeroDivisionError from NameError
except ZeroDivisionError as ex:
    assert type(ex.__cause__) == NameError

with assert_raises(NameError):
    try:
        raise NameError
    except:
        raise

with assert_raises(RuntimeError):
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

with assert_raises(ZeroDivisionError):
    try:
        1/0
    except:
        f()

with assert_raises(ZeroDivisionError):
    try:
        1/0
    except ZeroDivisionError:
        try:
            raise
        except NameError:
            pass
        raise

# try-return-finally behavior:
l = []
def foo():
    try:
        return 33
    finally:
        l.append(1337)

r = foo()
assert r == 33
assert l == [1337]


# Regression https://github.com/RustPython/RustPython/issues/867
for _ in [1, 2]:
    try:
        raise ArithmeticError()
    except ArithmeticError as e:
        continue


def g():
    try:
        1/0
    except ArithmeticError:
        return 5

try:
    g()
    raise NameError
except NameError as ex:
    assert ex.__context__ == None


def y():
    try:
        1/0
    except ArithmeticError:
        yield 5


try:
    y()
    raise NameError
except NameError as ex:
    assert ex.__context__ == None


try:
    {}[1]
except KeyError:
    try:
        raise RuntimeError()
    except RuntimeError:
        pass


try:
    try:
        raise ZeroDivisionError
    except ZeroDivisionError as ex:
        raise NameError from ex
except NameError as ex2:
    assert isinstance(ex2.__cause__, ZeroDivisionError)
else:
    assert False, "no raise"


try:
    try:
        try:
            raise ZeroDivisionError
        except ZeroDivisionError as ex:
            raise NameError from ex
    except NameError:
        raise
except NameError as ex2:
    assert isinstance(ex2.__cause__, ZeroDivisionError)
else:
    assert False, "no raise"


# the else clause requires at least one except clause:
with assert_raises(SyntaxError):
    exec("""
try:
    pass
else:
    pass
    """)


# Try requires at least except or finally (or both)
with assert_raises(SyntaxError):
    exec("""
try:
    pass
""")


# leaving the try block early emits an extra copy of the finally body, which
# must not consume the symbol tables of the nested scopes it contains
def return_from_try():
    log = []
    try:
        return "returned"
    finally:
        log.append((lambda x: x * 2)(3))
        log.append({t for t in [1, 2]})
        log.append([t for t in [3]])
        log.append({k: k for k in [4]})

        def nested():
            return 5

        class Nested:
            value = 6

        assert log == [6, {1, 2}, [3], {4: 4}], log
        assert nested() == 5
        assert Nested.value == 6


assert return_from_try() == "returned"


def break_and_continue_from_try():
    seen = []
    for i in range(4):
        try:
            if i == 1:
                continue
            if i == 3:
                break
            seen.append(i)
        finally:
            seen.append({t for t in [i]})
    return seen


assert break_and_continue_from_try() == [0, {0}, {1}, 2, {2}, {3}]


def return_from_try_runs_finally_once():
    log = []

    def inner():
        try:
            return "value"
        finally:
            log.append(sorted({t for t in "ab"}))

    assert inner() == "value"
    return log


assert return_from_try_runs_finally_once() == [["a", "b"]]


def generator_return_from_try():
    log = []

    def gen():
        try:
            return (yield "yielded")
        finally:
            log.append([t for t in "z"])

    g = gen()
    assert g.send(None) == "yielded"
    try:
        g.send("sent")
    except StopIteration as stop:
        assert stop.value == "sent", stop.value
    else:
        assert False, "generator did not stop"
    return log


assert generator_return_from_try() == [["z"]]


# the copy of the finally body is emitted where the try block is left, so its
# nested scopes must be looked up past the ones the rest of the try block opens
def scopes_after_the_early_exit():
    log = []

    def run(data, leave_early):
        try:
            if leave_early:
                return "early"
            return list(s * 2 for s in data)
        finally:
            log.append(sorted(k for k in data))

    assert run([1, 2], True) == "early"
    assert run([3, 1], False) == [6, 2]
    return log


assert scopes_after_the_early_exit() == [[1, 2], [1, 3]]


# a nested function in the try block is a scope too: taking its symbol table
# for the generator expression below built one without the `.0` argument
def named_scope_after_the_early_exit():
    log = []

    def run(data, leave_early):
        try:
            if leave_early:
                return "early"

            def inner():
                return [x + 1 for x in data]

            return inner()
        finally:
            log.append(sorted(k for k in data))

    assert run([2, 1], True) == "early"
    assert run([2, 1], False) == [3, 2]
    return log


assert named_scope_after_the_early_exit() == [[1, 2], [1, 2]]


# scopes that share a name resolve by position, so `inner` must not be found
# where the try block declares it
def same_name_scope_after_the_early_exit():
    log = []

    def run(value, leave_early):
        try:
            if leave_early:
                return "early"

            def inner():
                return value

            return inner()
        finally:

            def inner():
                return log

            assert inner() is log
            log.append((lambda x: x + value)(1))

    assert run(10, True) == "early"
    assert run(20, False) == 20
    return log


assert same_name_scope_after_the_early_exit() == [11, 21]


# breaking and continuing out of a loop copy the finally body the same way
def loop_exit_with_scopes_after_it():
    seen = []
    for i in range(4):
        try:
            if i == 1:
                continue
            if i == 3:
                break
            seen.append(sorted(t for t in [i]))
        finally:
            seen.append(sorted(k for k in [i, i + 1]))
    return seen


assert loop_exit_with_scopes_after_it() == [
    [0],
    [0, 1],
    [1, 2],
    [2],
    [2, 3],
    [3, 4],
], loop_exit_with_scopes_after_it()
