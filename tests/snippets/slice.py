
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

assert slice(10) == slice(10)
assert slice(-1) != slice(1)
assert slice(0, 10, 3) != slice(0, 11, 3)
assert slice(0, None, 3) != slice(0, 'a', 3)
assert slice(0, 'a', 3) == slice(0, 'a', 3)

assert slice(0, 0, 0).__eq__(slice(0, 0, 0))
assert not slice(0, 0, 1).__eq__(slice(0, 0, 0))
assert not slice(0, 1, 0).__eq__(slice(0, 0, 0))
assert not slice(1, 0, 0).__eq__(slice(0, 0, 0))
assert slice(1, 0, 0).__ne__(slice(0, 0, 0))
assert slice(0, 1, 0).__ne__(slice(0, 0, 0))
assert slice(0, 0, 1).__ne__(slice(0, 0, 0))

assert slice(0).__eq__(0) == NotImplemented
assert slice(0).__ne__(0) == NotImplemented
assert slice(None).__ne__(slice(0))