from dataclasses import dataclass
from typing import Any

__all__ = ["context"]


@dataclass
class Context:
    name: str
    something: Any


_context = Context(
    name="test name",
    something=None,
)


def context() -> Context:
    return _context


if __name__ == "__main__":
    print(context().name)
