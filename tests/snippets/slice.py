
a = slice(10)
assert a.start == None
assert a.stop == 10
assert a.step == None

a = slice(0, 10, 1)
assert a.start == 0
assert a.stop == 10
assert a.step == 1

assert slice(10).__repr__() == 'slice(None, 10, None)'
assert slice(None).__repr__() == 'slice(None, None, None)'
assert slice(0, 10, 13).__repr__() == 'slice(0, 10, 13)'
assert slice('0', 1.1, 2+3j).__repr__() == "slice('0', 1.1, (2+3j))"
