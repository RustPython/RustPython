try:
    5 / 0
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    5 / -0.0
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    5 / (2-2)
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    5 % 0
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    5 // 0
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    5.3 // (-0.0)
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    divmod(5, 0)
except ZeroDivisionError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'

try:
    raise ZeroDivisionError('Is an ArithmeticError subclass?')
except ArithmeticError:
    pass
else:
    assert False, 'Expected ZeroDivisionError'
