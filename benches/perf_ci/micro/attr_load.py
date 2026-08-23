# perf_ci axis: instance attribute load/store (__dict__ and slots).
import os

N = int(os.environ.get("PERF_CI_N", "100000"))


class Plain:
    def __init__(self):
        self.a = 1
        self.b = 2
        self.c = 3


class Slotted:
    __slots__ = ("a", "b")

    def __init__(self):
        self.a = 1
        self.b = 2


p = Plain()
s = Slotted()
total = 0
for _ in range(N):
    total += p.a + p.b + p.c
    total += s.a + s.b
    p.a = total & 0xFF
    s.a = total & 0xF

print(total)
