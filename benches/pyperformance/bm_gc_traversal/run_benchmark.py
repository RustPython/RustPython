import pyperf
import gc

N_LEVELS = 1000


def create_recursive_containers(n_levels):

    current_list = []
    for n in range(n_levels):
        new_list = [None] * n
        for index in range(n):
            new_list[index] = current_list
        current_list = new_list

    return current_list


def benchamark_collection(loops, n_levels):
    total_time = 0
    all_cycles = create_recursive_containers(n_levels)
    for _ in range(loops):
        gc.collect()
        # Main loop to measure
        t0 = pyperf.perf_counter()
        collected = gc.collect()
        total_time += pyperf.perf_counter() - t0

        assert collected is None or collected == 0

    return total_time


if __name__ == "__main__":
    runner = pyperf.Runner()
    runner.metadata["description"] = "GC traversal benchmark"
    runner.bench_time_func("gc_traversal", benchamark_collection, N_LEVELS)
