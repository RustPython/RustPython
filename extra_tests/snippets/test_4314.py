
class A(object):
    locals()[42] = 'abc'

assert A()

B = type("B", (), {1:1})
assert B()
assert repr(B())