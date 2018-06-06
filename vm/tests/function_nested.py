# Blocked on LOAD_GLOBAL
def sum(x,y):
    return x+y

def total(a,b,c,d):
    return sum(sum(a,b),sum(c,d))

assert total(1,2,3,4) == 10
