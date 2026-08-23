# perf_ci axis: string concat, join, format, find, case conversion.
# PYTHONHASHSEED is pinned by the runner so str hashing is deterministic.
import os

N = int(os.environ.get("PERF_CI_N", "20000"))

parts = []
s = ""
for i in range(N):
    s += "x"
    parts.append("part%d" % (i % 100))

joined = ",".join(parts)
total = len(s) + len(joined)
for i in range(N):
    t = "prefix %d suffix" % i
    total += t.find("suffix")
    total += len(t.upper())
    total += len(f"{i}-{i:04d}")

print(total)
