"""Repeatable baseline benchmark for the Python-to-Python call trampoline.

Run this with a release RustPython binary, not CPython:

    ./target/release/rustpython benches/tailcall_baseline.py

Each timed loop runs inside one RustPython process. Cases are rotated between
rounds so that every case is sampled at different points in the run.
"""

import dis
import sys
import time


DEFAULT_SAMPLES = 14
DEFAULT_ITERATIONS = 1_000_000
DEFAULT_DEEP_ITERATIONS = 10_000
DEFAULT_DEEP_DEPTH = 100


def add_one(value):
    return value + 1


class Adder:
    def add_one(self, value):
        return value + 1


bound_add_one = Adder().add_one


def shallow_inner(value):
    return value + 1


def shallow_outer(value):
    return shallow_inner(value)


def recursive_add(depth, value):
    if depth:
        return recursive_add(depth - 1, value)
    return value + 1


def bench_inline(iterations, _depth):
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = value + 1
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    return elapsed


def bench_exact_function(iterations, _depth):
    function = add_one
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = function(value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    return elapsed


def bench_exact_bound_method(iterations, _depth):
    method = bound_add_one
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = method(value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    return elapsed


def bench_shallow_nested(iterations, _depth):
    function = shallow_outer
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = function(value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    return elapsed


def bench_deep_recursive(iterations, depth):
    function = recursive_add
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = function(depth, value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    return elapsed


def shallow_activation_generator(iterations):
    """Time calls which each activate a fresh trampoline in their callee."""
    function = shallow_outer
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = function(value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    yield elapsed


def bench_shallow_activations(iterations, _depth):
    # Generator frames do not issue TailCall themselves. Each shallow_outer()
    # invocation therefore starts and finishes a new trampoline when it calls
    # shallow_inner(), including a fresh frame_stack allocation.
    return next(shallow_activation_generator(iterations))


def deep_activation_generator(iterations, depth):
    """Time deep calls which each allocate and spill a fresh frame stack."""
    function = recursive_add
    value = 0
    start = time.perf_counter_ns()
    for _index in range(iterations):
        value = function(depth, value)
    elapsed = time.perf_counter_ns() - start
    assert value == iterations
    yield elapsed


def bench_deep_activations(iterations, depth):
    return next(deep_activation_generator(iterations, depth))


def parse_positive_int(name, default):
    prefix = "--" + name + "="
    for argument in sys.argv[1:]:
        if argument.startswith(prefix):
            value = int(argument[len(prefix) :])
            if value <= 0:
                raise ValueError(prefix + " must be positive")
            return value
    return default


def median(values):
    ordered = sorted(values)
    midpoint = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[midpoint]
    return (ordered[midpoint - 1] + ordered[midpoint]) / 2


def require_instruction(function, opname):
    instructions = dis.get_instructions(function, adaptive=True)
    if not any(instruction.opname == opname for instruction in instructions):
        raise RuntimeError(function.__name__ + " did not specialize to " + opname)


def verify_specializations():
    expected = [
        (bench_exact_function, "CALL_PY_EXACT_ARGS"),
        (bench_exact_bound_method, "CALL_BOUND_METHOD_EXACT_ARGS"),
        (bench_shallow_nested, "CALL_PY_EXACT_ARGS"),
        (shallow_outer, "CALL_PY_EXACT_ARGS"),
        (bench_deep_recursive, "CALL_PY_EXACT_ARGS"),
        (recursive_add, "CALL_PY_EXACT_ARGS"),
        (shallow_activation_generator, "CALL_PY_EXACT_ARGS"),
        (deep_activation_generator, "CALL_PY_EXACT_ARGS"),
    ]
    for function, opname in expected:
        require_instruction(function, opname)
    return ";".join(function.__name__ + ":" + opname for function, opname in expected)


def main():
    if sys.implementation.name != "rustpython":
        raise RuntimeError("run this benchmark with a release RustPython binary")

    samples = parse_positive_int("samples", DEFAULT_SAMPLES)
    iterations = parse_positive_int("iterations", DEFAULT_ITERATIONS)
    deep_iterations = parse_positive_int(
        "deep-iterations", DEFAULT_DEEP_ITERATIONS
    )
    deep_depth = parse_positive_int("deep-depth", DEFAULT_DEEP_DEPTH)

    cases = [
        ("inline", bench_inline, iterations, 0, 0),
        ("exact_function", bench_exact_function, iterations, 0, 1),
        ("exact_bound_method", bench_exact_bound_method, iterations, 0, 1),
        ("shallow_nested_steady", bench_shallow_nested, iterations, 0, 2),
        (
            "deep_recursive_steady",
            bench_deep_recursive,
            deep_iterations,
            deep_depth,
            deep_depth + 1,
        ),
        (
            "shallow_nested_activation",
            bench_shallow_activations,
            iterations,
            0,
            2,
        ),
        (
            "deep_recursive_activation",
            bench_deep_activations,
            deep_iterations,
            deep_depth,
            deep_depth + 1,
        ),
    ]
    results = {name: [] for name, _function, _iterations, _depth, _calls in cases}

    # Warm every bytecode path before collecting the interleaved samples.
    for _name, function, _iterations, depth, _calls in cases:
        function(100, depth)
    specializations = verify_specializations()

    print("benchmark=tailcall_baseline_v2")
    print("implementation=" + sys.implementation.name)
    print("version=" + sys.version.replace("\n", " "))
    print("samples=" + str(samples))
    print("iterations=" + str(iterations))
    print("deep_iterations=" + str(deep_iterations))
    print("deep_depth=" + str(deep_depth))
    print("specializations=" + specializations)
    print("round,case,iterations,total_ns,ns_per_iteration")

    for round_index in range(samples):
        offset = round_index % len(cases)
        interleaved = cases[offset:] + cases[:offset]
        for name, function, case_iterations, depth, _calls in interleaved:
            total_ns = function(case_iterations, depth)
            ns_per_iteration = total_ns / case_iterations
            results[name].append(ns_per_iteration)
            print(
                str(round_index + 1)
                + ","
                + name
                + ","
                + str(case_iterations)
                + ","
                + str(total_ns)
                + ","
                + ("%.3f" % ns_per_iteration)
            )

    inline_median = median(results["inline"])
    print("case,median_ns_per_iteration,delta_vs_inline_ns,python_calls_per_iteration")
    for name, _function, _iterations, _depth, calls in cases:
        case_median = median(results[name])
        print(
            name
            + ","
            + ("%.3f" % case_median)
            + ","
            + ("%.3f" % (case_median - inline_median))
            + ","
            + str(calls)
        )


if __name__ == "__main__":
    main()
