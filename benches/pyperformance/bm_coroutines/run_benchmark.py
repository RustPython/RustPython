"""
Benchmark for recursive coroutines.

Author: Kumar Aditya
"""

import pyperf


async def fibonacci(n: int) -> int:
    if n <= 1:
        return n
    return await fibonacci(n - 1) + await fibonacci(n - 2)


def bench_coroutines(loops: int) -> float:
    range_it = range(loops)
    t0 = pyperf.perf_counter()
    for _ in range_it:
        coro = fibonacci(25)
        try:
            while True:
                coro.send(None)
        except StopIteration:
            pass
    return pyperf.perf_counter() - t0


if __name__ == "__main__":
    runner = pyperf.Runner()
    runner.metadata['description'] = "Benchmark coroutines"
    runner.bench_time_func('coroutines', bench_coroutines)
