import multiprocessing
import os
import threading
import time


def import_in_thread(module_name):
    outcome = {}
    error = {}

    def worker():
        try:
            module = __import__(module_name, fromlist=["*"])
            outcome["name"] = module.__name__
        except Exception as exc:
            error["exc"] = exc

    thread = threading.Thread(target=worker)
    thread.start()
    thread.join(timeout=5)
    assert not thread.is_alive(), "thread did not finish in time"
    if "exc" in error:
        raise error["exc"]

    assert outcome["name"] == module_name


def run_exec(code):
    result = {}
    error = {}

    def worker():
        try:
            scope = {"__builtins__": __builtins__}
            exec(code, scope, scope)  # noqa: S102 - intentional threaded exec regression test
            result["scope"] = scope
        except Exception as exc:
            error["exc"] = exc

    thread = threading.Thread(target=worker)
    thread.start()
    thread.join(timeout=5)
    assert not thread.is_alive(), "thread did not finish in time"
    if "exc" in error:
        raise error["exc"]
    return result["scope"]


def child_process():
    return None


def start_fork_process_after_thread():
    if not hasattr(os, "fork"):
        return

    import_in_thread("multiprocessing.connection")

    ctx = multiprocessing.get_context("fork")
    process = ctx.Process(target=child_process)
    process.start()
    process.join(timeout=10)
    assert process.exitcode == 0, process.exitcode


def thread_join_ordering():
    output = []

    def thread_function(name):
        output.append((name, 0))
        time.sleep(2.0)
        output.append((name, 1))

    output.append((0, 0))
    x = threading.Thread(target=thread_function, args=(1,))
    output.append((0, 1))
    x.start()
    output.append((0, 2))
    x.join()
    output.append((0, 3))

    assert len(output) == 6, output
    # CPython has [(1, 0), (0, 2)] for the middle 2, but we have [(0, 2), (1, 0)]
    # TODO: maybe fix this, if it turns out to be a problem?
    # assert output == [(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (0, 3)]


def thread_exit_without_join():
    # Regression for https://github.com/RustPython/RustPython/issues/7813:
    # a thread started without ``.join()`` must exit cleanly even when the
    # captured target callable drops during teardown (which can fire
    # weakref callbacks that re-enter the VM).
    output = []

    def runner():
        output.append("runner done")

    threading.Thread(target=runner).start()
    time.sleep(1)
    output.append("main done")
    assert "runner done" in output, output
    assert "main done" in output, output


thread_join_ordering()
thread_exit_without_join()

import_in_thread("functools")
import_in_thread("tempfile")
import_in_thread("multiprocessing.connection")
start_fork_process_after_thread()

scope = run_exec("import functools")
assert scope["functools"].__name__ == "functools"

scope = run_exec("from collections import namedtuple")
assert scope["namedtuple"].__name__ == "namedtuple"

scope = run_exec("module = __import__('multiprocessing.connection', fromlist=['*'])")
assert scope["module"].__name__ == "multiprocessing.connection"
