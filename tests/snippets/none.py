assert None is None

y = None
x = None
assert x is y

def none():
    pass

def none2():
    return None

assert none() is none()
assert none() is x

assert none() is none2()

assert str(None) == 'None'
assert repr(None) == 'None'
assert type(None)() is None
