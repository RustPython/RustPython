from testutils import assert_raises

class A(dict):
    def a():
        pass

    def b():
        pass


assert A.__dict__['a'] == A.a
with assert_raises(KeyError) as cm:
    A.__dict__['not here']

assert cm.exception.args[0] == "not here"

assert 'b' in A.__dict__
assert 'c' not in A.__dict__

assert '__dict__' in A.__dict__

assert A.__dict__.get("not here", "default") == "default"
assert A.__dict__.get("a", "default") is A.a
assert A.__dict__.get("not here") is None
