
def logged(f):
    def wrapper(a, b):
        print('Calling function', f)
        return f(a, b + 1)
    return wrapper


@logged
def add(a, b):
    return a + b

c = add(10, 3)

assert c == 14

