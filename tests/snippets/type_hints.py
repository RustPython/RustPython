
# See also: https://github.com/RustPython/RustPython/issues/587

def curry(foo: int, bla: int =2) -> float:
    return foo * 3.1415926 * bla

assert curry(2) > 10

print(curry.__annotations__)
assert curry.__annotations__['foo'] is int
assert curry.__annotations__['return'] is float
assert curry.__annotations__['bla'] is int
