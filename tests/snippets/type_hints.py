
# See also: https://github.com/RustPython/RustPython/issues/587

def curry(foo: int):  # TODO: -> float:
    return foo * 3.1415926 * 2

assert curry(2) > 10
