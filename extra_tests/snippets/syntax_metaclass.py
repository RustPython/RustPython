class MC(type):
    classes = []
    count = 0

    def __prepare__(name, bases):
        return {'prepared': True}

    def __new__(cls, name, bases, namespace):
        MC.classes.append(name)
        return type.__new__(cls, name, bases, namespace)

    def __call__(cls):
        MC.count += 1
        return type.__call__(cls, MC.count)

class C(object, metaclass=MC):
    def __new__(cls, count):
        self = object.__new__(cls)
        self.count = count
        return self

class D(metaclass=MC):
    pass

assert MC == type(C)
assert C == type(C())
assert MC.classes == ['C', 'D']
assert C().count == 2

assert C.prepared
assert D.prepared

class MC(type):
    pass

class MC2(MC):
    pass

class MC3(type):
    pass

class A():
    pass

assert type(A) == type

class B(metaclass=MC):
    pass

assert type(B) == MC

class C(B):
    pass

assert type(C) == MC

class D(metaclass=MC2):
    pass

assert type(D) == MC2

class E(C, D, metaclass=MC):
    pass

assert type(E) == MC2

class F(metaclass=MC3):
    pass

assert type(F) == MC3

try:
    class G(D, E, F):
        pass
    assert False
except TypeError:
    pass
