def fib(n):
    # a, b = 1, 1
    a = 1
    b = 1
    for _ in range(n-1):
      temp = b
      b = a+b
      a = temp

      #a, b = b, a+b
    return b

print(fib(1))
print(fib(2))
print(fib(3))
print(fib(4))
print(fib(5))
