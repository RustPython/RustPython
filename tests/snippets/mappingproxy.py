from testutils import assertRaises

class A(dict):
    def a():
        pass

    def b():
        pass


assert A.__dict__['a'] == A.a
with assertRaises(KeyError):
    A.__dict__['not_here']

assert 'b' in A.__dict__
assert 'c' not in A.__dict__

assert '__dict__' in A.__dict__
