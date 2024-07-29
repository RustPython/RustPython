from testutils import assert_raises


class Thing:
    def __init__(self):
        self.x = 1


t = Thing()
assert t.x == 1

del t.__dict__

with assert_raises(AttributeError):
    _ = t.x

assert t.__dict__ == {}
