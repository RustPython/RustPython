# perf_ci axis: list append, index, slice, sort, comprehension.
import os

N = int(os.environ.get("PERF_CI_N", "50000"))

lst = []
for i in range(N):
    lst.append((i * 7919) % N)

total = 0
for i in range(N):
    total += lst[i]

sub = lst[: N // 2]
squares = [x * x for x in sub]
lst.sort()
total += lst[0] + lst[-1] + len(squares)

print(total)
