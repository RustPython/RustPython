# perf_ci axis: dict store, load, delete, iteration with str and int keys.
# PYTHONHASHSEED is pinned by the runner so hashing is deterministic.
import os

N = int(os.environ.get("PERF_CI_N", "50000"))

d = {}
for i in range(N):
    d[i] = i
    d["k%d" % (i % 512)] = i

total = 0
for i in range(N):
    total += d[i]
    total += d["k%d" % (i % 512)]

for i in range(0, N, 2):
    del d[i]

for k in d:
    total += 1

print(total, len(d))
