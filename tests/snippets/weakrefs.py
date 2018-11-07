from _weakref import ref


class X:
    pass


a = X()
b = ref(a)

assert b() is a

