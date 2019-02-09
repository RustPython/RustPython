assert divmod(11, 3) == (3, 2)
assert divmod(8,11) == (0, 8)
assert divmod(0.873, 0.252) == (3.0, 0.11699999999999999)

try:
    divmod(5, 0)
except ZeroDivisionError:
    pass
else:
    assert False, "Expected divmod by zero to throw ZeroDivisionError"

try:
    divmod(5.0, 0.0)
except ZeroDivisionError:
    pass
else:
    assert False, "Expected divmod by zero to throw ZeroDivisionError"
