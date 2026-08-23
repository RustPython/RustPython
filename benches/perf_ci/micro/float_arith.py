# perf_ci axis: float arithmetic in a hot loop.
import os

N = int(os.environ.get("PERF_CI_N", "200000"))

x = 0.0
y = 1.0
for i in range(N):
    x += y * 1.000001
    y = y * 0.999999 + x * 1e-9

print(x, y)
