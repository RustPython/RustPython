# perf_ci axis: bound method call overhead (instance and builtin methods).
import os

N = int(os.environ.get("PERF_CI_N", "100000"))


class C:
    def get(self):
        return 1

    def add(self, x):
        return x + 1


obj = C()
lst = []
total = 0
for i in range(N):
    total += obj.get()
    total += obj.add(i)
    lst.append(i)
    lst.pop()

print(total, len(lst))
