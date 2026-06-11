ls = [1, 2, 3]

i = iter(ls)
assert i.__next__() == 1
assert i.__next__() == 2
assert next(i) == 3

assert next(i, "w00t") == "w00t"

s = "你好"
i = iter(s)
i.__setstate__(1)
assert i.__next__() == "好"
assert i.__reduce__()[2] == 2


# Regression test for re-entrant callable iterator sentinel equality.
# Equality can execute Python code that calls back into the same iterator.
class ReenterEq:
    entered = False
    it = None

    def __eq__(self, other):
        if not self.entered:
            self.entered = True
            try:
                next(self.it)
            except StopIteration:
                pass
            else:
                raise AssertionError("inner next should stop at the sentinel")
        return True


value = ReenterEq()
sentinel = object()
callable_it = iter(lambda: value, sentinel)
value.it = callable_it
try:
    next(callable_it)
except StopIteration:
    pass
else:
    raise AssertionError("outer next should stop at the sentinel")
assert value.entered
