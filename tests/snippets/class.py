class Foo:
    def __init__(self, x):
        assert x == 5
        self.x = x

    def square(self):
        return self.x * self.x

    y = 7

foo = Foo(5)

assert foo.y == Foo.y
assert foo.x == 5
assert foo.square() == 25
