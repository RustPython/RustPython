try:
    5 / 0
except ZeroDivisionError:
    pass
except:
    assert False, 'Expected ZeroDivisionError'

try:
    5 / -0.0
except ZeroDivisionError:
    pass
except:
    assert False, 'Expected ZeroDivisionError'

try:
    5 / (3-2)
except ZeroDivisionError:
    pass
except:
    assert False, 'Expected ZeroDivisionError'

try:
    5 % 0
except ZeroDivisionError:
    pass
except:
    assert False, 'Expected ZeroDivisionError'

try:
    raise ZeroDivisionError('Is an ArithmeticError subclass?')
except ArithmeticError:
    pass
except:
    assert False, 'Expected ZeroDivisionError to be a subclass of ArithmeticError'
