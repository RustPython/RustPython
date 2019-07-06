from testutils import assertRaises

class A(dict):
    def a():
        pass

    def b():
        pass


assert A.__dict__['a'] == A.a
with assertRaises(KeyError) as cm:
    A.__dict__['not here']

assert cm.exception.args[0] == "not here"

assert 'b' in A.__dict__
assert 'c' not in A.__dict__

assert '__dict__' in A.__dict__
