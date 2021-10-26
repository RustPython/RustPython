from typing import TypeVar

Y = TypeVar('Y')
assert dict[str,Y][int] == dict[str, int]