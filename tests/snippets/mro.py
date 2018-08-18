assert object.__mro__ == (object,)
assert type.__mro__ == (object,)

class A:
    pass

assert A.__mro__ == (A, object)
