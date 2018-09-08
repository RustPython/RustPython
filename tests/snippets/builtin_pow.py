assert pow(3,2) == 9
assert pow(5,3, 100) == 25

#causes overflow
# assert pow(41, 7, 2) == 1
assert pow(7, 2, 49) == 0
