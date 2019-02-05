assert range(2**63+1)[2**63] == 9223372036854775808

assert len(range(10, 5)) == 0, 'Range with no elements should have length = 0'
assert len(range(10, 5, -2)) == 3, 'Expected length 3, for elements: 10, 8, 6'
assert len(range(5, 10, 2)) == 3, 'Expected length 3, for elements: 5, 7, 9'
