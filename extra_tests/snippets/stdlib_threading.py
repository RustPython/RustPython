import multiprocessing
import os
import threading


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
