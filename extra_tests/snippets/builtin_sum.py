import math
from fractions import Fraction

# Float and complex accumulators keep a compensation term, as in CPython 3.14
assert sum([0.1, 0.2, 0.3]) == 0.6
assert sum(b"/%\x97!\x17", 8.962552828027274) == 299.9625528280273
assert sum([0.1, 0.2, 0.3], 0j) == 0.6 + 0j
assert sum([1j, 0.1, 0.2, 0.3]) == 0.6 + 1j
assert sum([0.1j, 0.2j, 0.3j]) == 0.6j

# The first item of another type falls back to plain addition
assert sum([0.5, Fraction(1, 4)]) == 0.75
assert sum([0.5j, Fraction(1, 4)]) == 0.25 + 0.5j

# The compensation never turns an infinite sum into nan or drops a zero's sign
assert sum([math.inf, 1.0]) == math.inf
assert repr(sum([-0.0j], -0.0j)) == "(-0-0j)"
