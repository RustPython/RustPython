# perf_ci axis: exception raise/catch, including try bodies that do not raise.
import os

N = int(os.environ.get("PERF_CI_N", "50000"))


class AppError(Exception):
    pass


total = 0
for i in range(N):
    try:
        if i % 2:
            raise AppError(i)
        total += 1
    except AppError:
        total += 2

    try:
        total += 1
    except ValueError:
        pass

    try:
        try:
            raise ValueError(i)
        except KeyError:
            pass
    except ValueError:
        total += 1

print(total)
