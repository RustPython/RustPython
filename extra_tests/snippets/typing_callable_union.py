from typing import Callable


# Ensure PEP 604 unions work with typing.Callable aliases.
TracebackFilter = bool | Callable[[int], int]
