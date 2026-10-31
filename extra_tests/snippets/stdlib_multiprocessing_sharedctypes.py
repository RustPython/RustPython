"""Shared ctypes char array .value writes must be visible across processes."""

import multiprocessing
from ctypes import Structure, c_double, c_int, c_longlong


class Foo(Structure):
    _fields_ = [("x", c_int), ("y", c_double), ("z", c_longlong)]


def double(foo, string):
    foo.x *= 2
    foo.y *= 2
    string.value *= 2


if __name__ == "__main__":
    multiprocessing.freeze_support()
    methods = ["spawn"]
    if "forkserver" in multiprocessing.get_all_start_methods():
        methods.append("forkserver")
    if "fork" in multiprocessing.get_all_start_methods():
        methods.append("fork")
    for method in methods:
        ctx = multiprocessing.get_context(method)
        foo = ctx.Value(Foo, 3, 2.0)
        string = ctx.Array("c", 20)
        string.value = b"hello"
        proc = ctx.Process(target=double, args=(foo, string))
        proc.start()
        proc.join(timeout=10)
        assert not proc.is_alive(), method
        assert foo.x == 6, (method, foo.x)
        assert abs(foo.y - 4.0) < 1e-9, (method, foo.y)
        assert string.value == b"hellohello", (method, string.value)
