assert bin(0) == '0b0'
assert bin(1) == '0b1'
assert bin(-1) == '-0b1'
assert bin(2**24) == '0b1' + '0' * 24
assert bin(2**24-1) == '0b' + '1' * 24
assert bin(-(2**24)) == '-0b1' + '0' * 24
assert bin(-(2**24-1)) == '-0b' + '1' * 24

# TODO: change to 2**65 when we have support for pow in bigint
a = 2**20 * 2**20 * 2**25
assert bin(a) == '0b1' + '0' * 65
assert bin(a-1) == '0b' + '1' * 65
assert bin(-(a)) == '-0b1' + '0' * 65
assert bin(-(a-1)) == '-0b' + '1' * 65
