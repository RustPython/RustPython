from testutils import assert_raises

# 2.456984346552728
res = 10**500 / (4 * 10**499 + 7 * 10**497 + 3 * 10**494)
assert 2.456984 <= res <= 2.456985

# 95.23809523809524
res = 10**3000 / (10**2998 + 5 * 10**2996)
assert 95.238095 <= res <= 95.238096

assert 10**500 / (2*10**(500-308)) == 5e307
assert 10**500 / (10**(500-308)) == 1e308
assert_raises(OverflowError, lambda: 10**500 / (10**(500-309)), _msg='too big result')

# a bit more than f64::MAX = 1.7976931348623157e+308_f64
assert (2 * 10**308) / 2 == 1e308

# when dividing too big int by a float, the operation should fail
assert_raises(OverflowError, lambda: (2 * 10**308) / 2.0, _msg='division of big int by float')
