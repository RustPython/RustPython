class X():
    pass

class Y():
    pass

class A(X, Y):
    pass

assert (A, X, Y, object) == A.__mro__

class B(X, Y):
    pass

assert (B, X, Y, object) == B.__mro__

class C(A, B):
    pass

assert (C, A, B, X, Y, object) == C.__mro__

assert type.__mro__ == (type, object)
