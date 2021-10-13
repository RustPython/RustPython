from testutils import assert_raises

class Foo(object):
    pass

Foo.__repr__ = Foo.__str__

foo = Foo()
# Since the default __str__ implementation calls __repr__ and __repr__ is
# actually __str__, str(foo) should raise a RecursionError.
assert_raises(RecursionError, str, foo)
