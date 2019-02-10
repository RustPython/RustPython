# 10**308 cannot be represented exactly in f64, thus it is not equal to 1e308 float
assert not (10**308 == 1e308)
# but the 1e308 float can be converted to big int and then it still should be equal to itself
assert int(1e308) == 1e308

# and the equalities should be the same when operands switch sides
assert not (1e308 == 10**308)
assert 1e308 == int(1e308)

# floats that cannot be converted to big ints shouldn’t crash the vm
import math
assert not (10**500 == math.inf)
assert not (math.inf == 10**500)
assert not (10**500 == math.nan)
assert not (math.nan == 10**500)
