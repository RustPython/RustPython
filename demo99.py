

def foo(x):
    def bar(z):
        return z + x
    return bar

f = foo(9)
g = foo(10)

print(f(2))
print(g(2))

