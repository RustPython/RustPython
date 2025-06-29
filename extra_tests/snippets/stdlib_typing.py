from collections.abc import Awaitable, Callable
from typing import TypeVar

T = TypeVar("T")


def abort_signal_handler(
    fn: Callable[[], Awaitable[T]], on_abort: Callable[[], None] | None = None
) -> T:
    pass
