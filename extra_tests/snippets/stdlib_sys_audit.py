import sys

seen = []
sys.addaudithook(lambda event, args: seen.append((event, args)))

code = compile("x = 1", "<test>", "exec")
exec(code, {})
assert ("exec", (code,)) in seen

# Built-in events go to the registered hooks, not through a reassigned `sys.audit`.
seen.clear()
replaced = []
sys.audit = lambda *args: replaced.append(args)
exec(code, {})
assert ("exec", (code,)) in seen
assert replaced == []

# Hooks are per interpreter, so they also fire in other threads.
import threading

seen.clear()
worker = threading.Thread(target=exec, args=(code, {}))
worker.start()
worker.join()
assert ("exec", (code,)) in seen

# Hooks added concurrently are each notified by every hook added after them.
N = 8
counts = [0] * N
lock = threading.Lock()


def make_hook(i):
    def hook(event, args):
        if event == "sys.addaudithook":
            with lock:
                counts[i] += 1

    return hook


barrier = threading.Barrier(N)


def add_hook(i):
    barrier.wait()
    sys.addaudithook(make_hook(i))


workers = [threading.Thread(target=add_hook, args=(i,)) for i in range(N)]
for w in workers:
    w.start()
for w in workers:
    w.join()
assert sorted(counts) == list(range(N)), counts


# Native open events must precede file access and custom openers, but follow
# FileIO's argument/path/mode validation.
def check_open_audit():
    import io
    import os
    import ssl
    import tempfile

    import _remote_debugging

    events = []
    active = False

    class Denied(Exception):
        pass

    def hook(event, args):
        if active and event == "open":
            events.append(args)
            raise Denied

    def opener(*args):
        raise AssertionError("opener ran before audit")

    sys.addaudithook(hook)
    with tempfile.TemporaryDirectory() as directory:
        path = os.path.join(directory, "not-created")
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        try:
            active = True
            for operation in (
                lambda: open(path, "wb", opener=opener),
                lambda: io.FileIO(path, "rb"),
                lambda: io.FileIO(path, "w", opener=opener),
                lambda: io.FileIO(path, "w", closefd=False),
                lambda: io.FileIO(2**30, "r", closefd=False),
                lambda: _remote_debugging.BinaryWriter(path, 1000, 0),
                lambda: _remote_debugging.BinaryReader(path),
                lambda: context.load_dh_params(path),
            ):
                previous = len(events)
                try:
                    operation()
                except Denied:
                    pass
                else:
                    raise AssertionError("native open ignored hook denial")
                assert len(events) == previous + 1
            assert events[0][:2] == (path, "w")
            assert events[1][:2] == (path, "rb")
            assert not os.path.exists(path)
            assert events[-3:] == [(path, "wb", 0), (path, "rb", 0), (path, "rb", 0)]

            previous = len(events)
            try:
                io.FileIO(path, "invalid-mode")
            except ValueError:
                pass
            else:
                raise AssertionError("invalid mode accepted")
            assert len(events) == previous

            class InvalidPath:
                def __fspath__(self):
                    raise LookupError("path conversion")

            for operation in (
                lambda: _remote_debugging.BinaryWriter(InvalidPath(), 1000, 0),
                lambda: _remote_debugging.BinaryReader(InvalidPath()),
                lambda: context.load_dh_params(InvalidPath()),
            ):
                try:
                    operation()
                except Denied:
                    pass
                else:
                    raise AssertionError("Py_fopen path conversion ran before audit")
            previous = len(events)
            try:
                _remote_debugging.BinaryWriter(path, 1000, 0, mode=5)
            except ValueError:
                pass
            else:
                raise AssertionError("invalid profiling mode accepted")
            assert len(events) == previous

            try:
                io.FileIO(InvalidPath())
            except LookupError:
                pass
            else:
                raise AssertionError("path conversion failure ignored")
            assert len(events) == previous
        finally:
            active = False


check_open_audit()
