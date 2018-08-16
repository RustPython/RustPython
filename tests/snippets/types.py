assert type(type) is type

class Foo():
    pass

assert type(Foo) is type
assert type(Foo()) is Foo
