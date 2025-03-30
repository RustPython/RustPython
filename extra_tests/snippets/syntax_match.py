i = 0
z = 1
match i:
    case 0:
        z = 0
    case 1:
        z = 2
    case _:
        z = 3

assert z == 0
# Test enum
from enum import Enum

class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3

def test_color(color):
    z = -1
    match color:
        case Color.RED:
            z = 1
        case Color.GREEN:
            z = 2
        case Color.BLUE:
            z = 3
    assert z == color.value

for color in Color:
    test_color(color)

# test or
def test_or(i):
    z = -1
    match i:
        case 0 | 1:
            z = 0
        case 2 | 3:
            z = 1
        case _:
            z = 2
    return z

assert test_or(0) == 0
assert test_or(1) == 0
assert test_or(2) == 1
assert test_or(3) == 1
assert test_or(4) == 2
