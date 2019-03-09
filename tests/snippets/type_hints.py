
# See also: https://github.com/RustPython/RustPython/issues/587

def curry(foo: int, bla=2) -> float:
    return foo * 3.1415926 * bla

assert curry(2) > 10
