# break from a nested for loop

def foo():
    sum = 0
    for i in range(10):
        sum += i
        for j in range(10):
            sum += j
            break
    return sum

assert foo() == 45


# continue statement

def primes(limit):
    """Finds all the primes from 2 up to a given number using the Sieve of Eratosthenes."""
    sieve = [False] * (limit + 1)
    for i in range(2, limit + 1):
        if sieve[i]:
            continue
        yield i

        for j in range(2 * i, limit + 1, i):
            sieve[j] = True


assert list(primes(1)) == []
assert list(primes(2)) == [2]
assert list(primes(10)) == [2, 3, 5, 7]
assert list(primes(13)) == [2, 3, 5, 7, 11, 13]

