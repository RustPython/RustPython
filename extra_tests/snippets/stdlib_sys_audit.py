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


def check_fileio_reinitialization_audit():
    import io
    import os
    import tempfile

    active = False
    events = []
    stream = None

    class Denied(Exception):
        pass

    def hook(event, args):
        if active and event == "open":
            events.append(args)
            if stream is not None:
                assert stream.closed
                assert stream.mode == "rb+"
            raise Denied

    sys.addaudithook(hook)
    with tempfile.TemporaryDirectory() as directory:
        path = os.path.join(directory, "existing")
        with open(path, "wb") as output:
            output.write(b"retained")
        try:
            active = True
            try:
                io.FileIO(path)
            except Denied:
                pass
            else:
                raise AssertionError("omitted-mode audit denial ignored")
            assert events[-1][1] == "r"
            active = False
            for closefd in (True, False):
                fd = os.open(path, os.O_RDWR)
                stream = io.FileIO(fd, "r+", closefd=closefd)
                active = True
                try:
                    stream.__init__(path)
                except Denied:
                    pass
                else:
                    raise AssertionError("reinitialization audit denial ignored")
                active = False
                assert stream.closed
                assert events[-1][1] == "r"
                assert events[-1][2] & os.O_RDWR
                try:
                    os.fstat(fd)
                except OSError:
                    assert closefd
                else:
                    assert not closefd
                    os.close(fd)
                stream.close()
                stream = None
            assert open(path, "rb").read() == b"retained"
        finally:
            active = False
            if stream is not None:
                stream.close()


check_fileio_reinitialization_audit()


def check_winreg_audit_boundaries():
    if sys.platform != "win32":
        return
    import winreg

    class RegistryDenied(Exception):
        pass

    blocked = None
    events = []

    def hook(event, args):
        if event == blocked:
            events.append(args)
            raise RegistryDenied

    sys.addaudithook(hook)
    # Valid conversion reaches the hook before accessing an invalid native handle.
    negative_access = (1 << 32) - 1 if sys.maxsize > (1 << 32) else -1
    cases = [
        ("ConnectRegistry", (None, 0), "ConnectRegistry", (None, 0)),
        ("CreateKey", (0, "x"), "CreateKey", (0, "x", winreg.KEY_WRITE)),
        ("CreateKey", (0, None), "CreateKey", (0, None, winreg.KEY_WRITE)),
        ("CreateKeyEx", (0, None), "CreateKey", (0, None, winreg.KEY_WRITE)),
        ("CreateKeyEx", (0, "x", -1, -1), "CreateKey", (0, "x", negative_access)),
        ("OpenKey", (0, "x", -1, -1), "OpenKey", (0, "x", negative_access)),
        ("DeleteKeyEx", (0, "x", -1, -1), "DeleteKey", (0, "x", negative_access)),
        (
            "CreateKeyEx",
            (0, "x", 0, winreg.KEY_READ),
            "CreateKey",
            (0, "x", winreg.KEY_READ),
        ),
        ("DeleteKey", (0, "x"), "DeleteKey", (0, "x", 0)),
        (
            "DeleteKeyEx",
            (0, "x", winreg.KEY_WOW64_64KEY),
            "DeleteKey",
            (0, "x", winreg.KEY_WOW64_64KEY),
        ),
        ("DeleteValue", (0, None), "DeleteValue", (0, None)),
        ("EnumValue", (0, -1), "EnumValue", (0, -1)),
        ("LoadKey", (0, "x", "x.dat"), "LoadKey", (0, "x", "x.dat")),
        ("SaveKey", (0, "x.dat"), "SaveKey", (0, "x.dat")),
        (
            "SetValue",
            (0, "x", winreg.REG_SZ, "a\0b"),
            "SetValue",
            (0, "x", winreg.REG_SZ, "a\0b"),
        ),
        (
            "SetValueEx",
            (0, None, None, winreg.REG_SZ, "a\0b"),
            "SetValue",
            (0, None, winreg.REG_SZ, "a\0b"),
        ),
        ("DisableReflectionKey", (0,), "DisableReflectionKey", (0,)),
        ("EnableReflectionKey", (0,), "EnableReflectionKey", (0,)),
        ("QueryReflectionKey", (0,), "QueryReflectionKey", (0,)),
        (
            "ExpandEnvironmentStrings",
            ("%PATH%",),
            "ExpandEnvironmentStrings",
            ("%PATH%",),
        ),
    ]
    for name, args, event, expected in cases:
        blocked = "winreg." + event
        events.clear()
        try:
            getattr(winreg, name)(*args)
        except RegistryDenied:
            pass
        else:
            raise AssertionError(name)
        assert events == [expected], (name, events)
    # Argument Clinic rejects values outside C int before emitting the event.
    for name, event in (
        ("CreateKeyEx", "CreateKey"),
        ("OpenKey", "OpenKey"),
        ("DeleteKeyEx", "DeleteKey"),
    ):
        for invalid in (1 << 31, -(1 << 31) - 1):
            for parameter in ("access", "reserved"):
                blocked = "winreg." + event
                events.clear()
                try:
                    getattr(winreg, name)(0, "x", **{parameter: invalid})
                except OverflowError:
                    pass
                else:
                    raise AssertionError((name, parameter, invalid))
                assert events == []
    blocked = "winreg.SetValue"
    events.clear()
    for kind, value in (
        (winreg.REG_DWORD, "bad"),
        (winreg.REG_QWORD, "bad"),
        (winreg.REG_SZ, 0),
        (winreg.REG_EXPAND_SZ, 0),
        (winreg.REG_MULTI_SZ, "bad"),
        (winreg.REG_MULTI_SZ, ["ok", 0]),
    ):
        try:
            winreg.SetValueEx(0, None, None, kind, value)
        except ValueError as exc:
            assert str(exc) == "Could not convert the data to the specified type."
        else:
            raise AssertionError("data conversion must precede audit")
        assert events == []
    for kind, value in (
        (winreg.REG_DWORD, -1),
        (winreg.REG_DWORD, 1 << 32),
        (winreg.REG_QWORD, -1),
        (winreg.REG_QWORD, 1 << 64),
    ):
        try:
            winreg.SetValueEx(0, None, None, kind, value)
        except OverflowError:
            pass
        else:
            raise AssertionError("numeric overflow must precede audit")
        assert events == []
    try:
        winreg.SetValueEx(0, None, None, winreg.REG_BINARY, object())
    except TypeError:
        pass
    else:
        raise AssertionError("binary conversion must precede audit")
    assert events == []

    blocked = None
    key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Software")
    for count in (2, 4):
        try:
            key.__exit__(*([None] * count))
        except TypeError:
            pass
        else:
            raise AssertionError("incorrect context exit arity accepted")
        assert int(key) != 0
    try:
        handle = int(key)
        blocked = "winreg.PyHKEY.Detach"
        try:
            key.Detach()
        except RegistryDenied:
            pass
        else:
            raise AssertionError("detach veto ignored")
        assert int(key) == handle
        blocked = None
        assert key.Detach() == handle
        assert int(key) == 0
        winreg.CloseKey(handle)
        winreg.CloseKey(None)
        winreg.CloseKey(0)
    finally:
        blocked = None
        key.__exit__(None, None, None)
        winreg.CloseKey(key)

    # A reentrant hook may detach first; the outer call must return zero.
    key = winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Software")
    nested = []
    active = True

    def detach_again(event, args):
        nonlocal active
        if active and event == "winreg.PyHKEY.Detach":
            active = False
            nested.append(key.Detach())

    sys.addaudithook(detach_again)
    try:
        assert key.Detach() == 0
        winreg.CloseKey(nested[0])
    finally:
        active = False
        key.Close()

    # The temporary key must survive argument conversion and the hook.
    class Index:
        def __index__(self):
            import gc

            gc.collect()
            return -1

    try:
        winreg.EnumKey(winreg.OpenKey(winreg.HKEY_CURRENT_USER, "Software"), Index())
    except OSError as error:
        assert error.winerror == 259  # No more items, not invalid handle (6).
    else:
        raise AssertionError("out-of-range enumeration succeeded")

    # A result hook can reject the newly created owner; its handle must not leak.
    for open_key in (winreg.OpenKey, winreg.CreateKey, winreg.CreateKeyEx):
        blocked = "winreg.OpenKey/result"
        events.clear()
        try:
            open_key(winreg.HKEY_CURRENT_USER, "Software")
        except RegistryDenied:
            pass
        else:
            raise AssertionError("result veto ignored")
        blocked = None
        rejected_handle = events[-1][0]
        try:
            # Closing probes validity without requiring KEY_QUERY_VALUE, which
            # CreateKeyEx's default KEY_WRITE handle does not grant.
            winreg.CloseKey(rejected_handle)
        except OSError as error:
            assert error.winerror == 6
        else:
            # CPython fixed result-audit handle cleanup in 3.15.1. The CI
            # reference is still 3.15.0; RustPython must always close it.
            # A successful close has also cleaned up that old reference leak.
            if not (
                sys.implementation.name == "cpython" and sys.version_info < (3, 15, 1)
            ):
                raise AssertionError("rejected registry handle leaked")


check_winreg_audit_boundaries()


def check_winapi_audit_boundaries():
    if sys.platform != "win32":
        return
    import _winapi

    class Denied(Exception):
        pass

    class Name(str):
        pass

    blocked = None
    seen = []

    def hook(event, args):
        if event == blocked:
            seen.append(args)
            raise Denied

    sys.addaudithook(hook)
    cases = [
        ("CreateEventW", (0, -1, 256, Name("audit-event")), (-1, 256, "audit-event")),
        ("CreateMutexW", (0, 2, None), (2, None)),
        (
            "CreateFile",
            (Name("missing-audit-file"), 0, 0, 0, 3, 0, 0),
            ("missing-audit-file", 0, 0, 3, 0),
        ),
        (
            "CreateNamedPipe",
            (Name(r"\\.\pipe\audit-boundary"), 3, 8, 1, 0, 0, 0, 0),
            (r"\\.\pipe\audit-boundary", 3, 8),
        ),
        ("CreatePipe", (None, 0), ()),
        (
            "CreateJunction",
            (Name("missing-audit-source"), Name("missing-audit-destination")),
            ("missing-audit-source", "missing-audit-destination"),
        ),
        ("OpenEventW", (1, 2, Name("missing-audit-event")), (1, "missing-audit-event")),
        (
            "OpenMutexW",
            (1, -1, Name("missing-audit-mutex")),
            (1, "missing-audit-mutex"),
        ),
        ("OpenProcess", (1, 2, 0), (0, 1)),
        ("TerminateProcess", (0, 123), (0, 123)),
        (
            "CopyFile2",
            (Name("missing-audit-source"), Name("missing-audit-destination"), 0),
            ("missing-audit-source", "missing-audit-destination", 0),
        ),
    ]
    for name, args, expected in cases:
        blocked = "_winapi." + name
        seen.clear()
        try:
            getattr(_winapi, name)(*args)
        except Denied:
            pass
        else:
            raise AssertionError(name)
        assert seen == [expected], (name, seen)
        assert all(
            type(actual) is type(want) for actual, want in zip(seen[0], expected)
        )

    if sys.implementation.name == "rustpython":
        # CPython's Unicode-format audit path is unsafe for this sentinel.
        accesses = []

        class Unused:
            def __getattribute__(self, name):
                accesses.append(name)
                raise AssertionError("CreateProcess processed arguments before audit")

        command_line = object()
        blocked = "_winapi.CreateProcess"
        seen.clear()
        try:
            _winapi.CreateProcess(
                None, command_line, None, None, 0, 0, Unused(), None, Unused()
            )
        except Denied:
            pass
        else:
            raise AssertionError("CreateProcess audit did not deny the call")
        assert seen == [(None, command_line, None)]
        assert seen[0][1] is command_line
        assert not accesses

    # Unicode conversion must run before a later integer converter or audit.
    calls = []

    class Index:
        def __index__(self):
            calls.append("index")
            return 0

    blocked = "_winapi.CreateFile"
    seen.clear()
    try:
        _winapi.CreateFile("bad\0name", Index(), 0, 0, 3, 0, 0)
    except ValueError:
        pass
    else:
        raise AssertionError("embedded NUL accepted")
    assert not calls and not seen

    # BOOL arguments use C int conversion, not truth testing.
    class Truth:
        def __bool__(self):
            raise AssertionError("BOOL must not call __bool__")

    blocked = "_winapi.CreateEventW"
    seen.clear()
    try:
        _winapi.CreateEventW(0, Truth(), 0, None)
    except TypeError:
        pass
    else:
        raise AssertionError("BOOL accepted a truth-only object")
    assert not seen

    # CreateJunction performs its invalid-prefix check before the audit.
    blocked = "_winapi.CreateJunction"
    seen.clear()
    try:
        _winapi.CreateJunction(r"\??\invalid", "unused")
    except OSError as error:
        assert error.winerror == 87
    else:
        raise AssertionError("invalid junction prefix accepted")
    assert not seen
    blocked = None


check_winapi_audit_boundaries()
