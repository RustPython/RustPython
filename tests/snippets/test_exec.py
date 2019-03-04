exec("def square(x):\n return x * x\n")
assert 16 == square(4)

d = {}
exec("def square(x):\n return x * x\n", {}, d)
assert 16 == d['square'](4)

exec("assert 2 == x", {}, {'x': 2})
exec("assert 2 == x", {'x': 2}, {})
exec("assert 4 == x", {'x': 2}, {'x': 4})

exec("assert max(1, 2) == 2", {}, {})

exec("assert max(1, 5, square(5)) == 25", None)

#
# These doesn't work yet:
#
# Local environment shouldn't replace global environment:
#
# exec("assert max(1, 5, square(5)) == 25", None, {})
#
# Closures aren't available if local scope is replaced:
#
# def g():
#     seven = "seven"
#     def f():
#         try:
#             exec("seven", None, {})
#         except NameError:
#             pass
#         else:
#             raise NameError("seven shouldn't be in scope")
#     f()
# g()

try:
    exec("", 1)
except TypeError:
    pass
else:
    raise TypeError("exec should fail unless globals is a dict or None")
