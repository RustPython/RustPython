from testutils import assert_raises


class Foo(object):
    pass


Foo.__repr__ = Foo.__str__

foo = Foo()
# Since the default __str__ implementation calls __repr__ and __repr__ is
# actually __str__, str(foo) should raise a RecursionError.
assert_raises(RecursionError, str, foo)


# A __call__ that is the object being called dispatches through the call slot
# again, and none of that pushes a Python frame.


class Caller:
    pass


caller = Caller()
Caller.__call__ = caller
assert_raises(RecursionError, caller)


# The same shape through the descriptor protocol: resolving the attribute
# fetches __get__, which is the descriptor itself.


class Descr:
    pass


descr = Descr()
Descr.__get__ = descr
Descr.x = descr
try:
    descr.x
except (RecursionError, TypeError):
    # RecursionError here, TypeError from the call of a non-callable elsewhere
    pass
else:
    raise AssertionError("descr.x should not resolve")
