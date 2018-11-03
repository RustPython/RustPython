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
