# perf_ci axis: Python function call overhead (positional, default, kwargs).
import os

N = int(os.environ.get("PERF_CI_N", "100000"))


def f0():
    return 1


def f2(a, b):
    return a + b


def fd(a, b=2, c=3):
    return a + b + c


total = 0
for i in range(N):
    total += f0()
    total += f2(i, 1)
    total += fd(i)
    total += fd(i, c=5)

print(total)
