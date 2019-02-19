def foo():
    sum = 0
    for i in range(10):
        sum += i
        for j in range(10):
            sum += j
            break
    return sum

assert foo() == 45
