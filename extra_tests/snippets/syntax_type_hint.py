
# See also: https://github.com/RustPython/RustPython/issues/587

def curry(foo: int, bla: int =2) -> float:
    return foo * 3.1415926 * bla

assert curry(2) > 10

print(curry.__annotations__)
assert curry.__annotations__['foo'] is int
assert curry.__annotations__['return'] is float
assert curry.__annotations__['bla'] is int

import collections.abc
import typing

P = typing.ParamSpec('P')
assert P.args.__hash__ is None
assert P.kwargs.__hash__ is None
assert not isinstance(P.args, collections.abc.Hashable)
assert not isinstance(P.kwargs, collections.abc.Hashable)
try:
    hash(P.args)
    assert False, "expected TypeError"
except TypeError:
    pass
try:
    hash(P.kwargs)
    assert False, "expected TypeError"
except TypeError:
    pass
