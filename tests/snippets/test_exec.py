exec("def square(x):\n return x * x\n")
assert 16 == square(4)

d = {}
exec("def square(x):\n return x * x\n", {}, d)
assert 16 == d['square'](4)

exec("assert 2 == x", {}, {'x': 2})
exec("assert 2 == x", {'x': 2}, {})
exec("assert 4 == x", {'x': 2}, {'x': 4})

exec("assert max(1, 2) == 2", {}, {})

exec("max(1, 5, square(5)) == 25", None)

try:
    exec("", 1)
except TypeError:
    pass
else:
    raise TypeError("exec should fail unless globals is a dict or None")
