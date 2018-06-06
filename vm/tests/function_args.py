def sum(x, y):
    return x+y

# def total(a, b, c, d):
#     return sum(sum(a,b), sum(c,d))
#
# assert total(1,1,1,1) == 4
# assert total(1,2,3,4) == 10

assert sum(1,1) == 2
assert sum(1,3) == 4

def sum2y(x, y):
    return x+y*2

assert sum2y(1,1) == 3
assert sum2y(1,3) == 7
