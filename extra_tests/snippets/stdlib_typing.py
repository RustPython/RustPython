from collections.abc import Awaitable, Callable
from typing import TypeVar

import _typing
from testutils import assert_raises

T = TypeVar("T")


def abort_signal_handler(
    fn: Callable[[], Awaitable[T]], on_abort: Callable[[], None] | None = None
) -> T:
    pass


# Ensure PEP 604 unions work with typing.Callable aliases.
TracebackFilter = bool | Callable[[int], int]


# Test that Union/Optional in function parameter annotations work correctly.
# This tests that annotation scopes can access global implicit symbols (like Union)
# that are imported at module level but not explicitly bound in the function scope.
# Regression test for: rich
from typing import Optional, Union


def function_with_union_param(x: Optional[Union[int, str]] = None) -> None:
    pass


class ClassWithUnionParams:
    def __init__(
        self,
        color: Optional[Union[str, int]] = None,
        bold: Optional[bool] = None,
    ) -> None:
        pass

    def method(self, value: Union[int, float]) -> Union[str, bytes]:
        return str(value)


# _idfunc takes exactly one argument, checked before the argument is read.

assert _typing._idfunc(1) == 1
with assert_raises(TypeError):
    _typing._idfunc()


# ParamSpecArgs shows a non-ParamSpec origin by its repr, which is where the
# recursion guard lives; nesting them deeply must not walk the native stack.

from typing import ParamSpec, ParamSpecArgs

spec = ParamSpec("spec")
assert repr(spec.args) == "spec.args"
assert repr(spec.kwargs) == "spec.kwargs"

nested = object()
for _ in range(2000):
    nested = ParamSpecArgs(nested)
try:
    repr(nested)
except RecursionError:
    pass
