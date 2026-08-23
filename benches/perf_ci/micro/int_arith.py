# perf_ci axis: integer arithmetic in a hot loop.
# Deterministic; sized for callgrind instruction counting (small native runtime).
import os

N = int(os.environ.get("PERF_CI_N", "200000"))

total = 0
i = 0
while i < N:
    total += i * 3 + (i >> 2) - (i % 7)
    i += 1

acc = 0
for j in range(N):
    acc = acc * 2 % 1000003 + j

print(total, acc)
