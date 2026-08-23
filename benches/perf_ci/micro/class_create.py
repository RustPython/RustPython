# perf_ci axis: class object creation (type creation) and instantiation.
import os

N = int(os.environ.get("PERF_CI_N", "2000"))

total = 0
for i in range(N):
    class Dynamic:
        x = 1

        def method(self):
            return self.x

    obj = Dynamic()
    total += obj.method()


class Fixed:
    def __init__(self, a, b):
        self.a = a
        self.b = b


for i in range(N * 10):
    total += Fixed(i, i + 1).a

print(total)
