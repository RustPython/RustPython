class X():
    pass

class Y():
    pass

class A(X, Y):
    pass

print(A.__mro__)

class B(X, Y):
    pass

print(B.__mro__)

class C(A, B):
    pass

print(C.__mro__)
