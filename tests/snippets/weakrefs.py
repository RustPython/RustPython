from _weakref import ref


class X:
    pass


a = X()
b = ref(a)

assert callable(b)
assert b() is a

