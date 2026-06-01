import math

from testutils import assert_raises

assert divmod(11, 3) == (3, 2)
assert divmod(8, 11) == (0, 8)
assert divmod(0.873, 0.252) == (3.0, 0.11699999999999999)
assert divmod(-86340, 86400) == (-1, 60)

assert_raises(ZeroDivisionError, divmod, 5, 0, _msg="divmod by zero")
assert_raises(ZeroDivisionError, divmod, 5.0, 0.0, _msg="divmod by zero")


def signbit(x):
    return math.copysign(1.0, x) < 0


# Zero remainder with opposite-sign divisor — quotient and remainder must
# both be zero with the divisor's sign, not propagate through the
# sign-correction branch.
q, r = divmod(0.0, -1.0)
assert q == 0.0 and signbit(q)
assert r == 0.0 and signbit(r)

q, r = divmod(6.0, -3.0)
assert q == -2.0
assert r == 0.0 and signbit(r)

q, r = divmod(-100.0, 10.0)
assert q == -10.0
assert r == 0.0 and not signbit(r)

# Zero quotient — sign matches the true quotient v1 / v2, not the sign that
# leaks from the (v1 - m) / v2 intermediate calculation.
q, r = divmod(-1.0, -2.0)
assert q == 0.0 and not signbit(q)
assert r == -1.0

q, r = divmod(-0.0, 1.0)
assert q == 0.0 and signbit(q)
assert r == 0.0 and not signbit(r)

# Spec invariant: divmod(a, b) == (a // b, a % b), including signed zero.
for a, b in [
    (0.0, -1.0),
    (6.0, -3.0),
    (-6.0, 3.0),
    (100.0, -10.0),
    (-100.0, 10.0),
    (-1.0, -2.0),
    (-0.0, 1.0),
    (-0.0, -1.0),
    (7.0, 3.0),
    (-7.0, 3.0),
    (7.0, -3.0),
    (-7.0, -3.0),
    (3.7, 1.5),
    (-3.7, 1.5),
]:
    dm = divmod(a, b)
    assert dm[0] == a // b
    assert dm[1] == a % b
    assert signbit(dm[0]) == signbit(a // b)
    assert signbit(dm[1]) == signbit(a % b)

# Spec invariants for float divmod:
#   q * b + r == a, r == 0 or sign(r) == sign(b), 0 <= abs(r) < abs(b).
for a, b in [
    (7.0, 3.0),
    (7.0, -3.0),
    (-7.0, 3.0),
    (-7.0, -3.0),
    (6.0, -3.0),
    (100.0, -10.0),
    (3.7, 1.5),
    (5.5, 2.0),
    (-5.5, 2.0),
]:
    q, r = divmod(a, b)
    assert q * b + r == a
    assert r == 0.0 or (r < 0.0) == (b < 0.0)
    assert abs(r) < abs(b)
