import signal
import sys
import time

from testutils import assert_raises

assert_raises(TypeError, lambda: signal.signal(signal.SIGINT, 2))

signals = []


def handler(signum, frame):
    signals.append(signum)


signal.signal(signal.SIGILL, signal.SIG_IGN)
assert signal.getsignal(signal.SIGILL) is signal.SIG_IGN

old_signal = signal.signal(signal.SIGILL, signal.SIG_DFL)
assert old_signal is signal.SIG_IGN
assert signal.getsignal(signal.SIGILL) is signal.SIG_DFL


# Ignoring a synchronously raised signal must not invoke a callback, including
# on WASI where libc uses a function pointer rather than integer 1 for SIG_IGN.
synchronous_signals = []


def synchronous_handler(signum, frame):
    synchronous_signals.append(signum)


previous = signal.signal(signal.SIGILL, synchronous_handler)
try:
    signal.raise_signal(signal.SIGILL)
    assert synchronous_signals == [signal.SIGILL]
    assert signal.signal(signal.SIGILL, signal.SIG_IGN) is synchronous_handler
    signal.raise_signal(signal.SIGILL)
    assert synchronous_signals == [signal.SIGILL]
    assert signal.getsignal(signal.SIGILL) is signal.SIG_IGN
finally:
    signal.signal(signal.SIGILL, previous)

if sys.platform == "wasi":
    # wasi-libc rejects SIGKILL (9) and SIGSTOP (19), even though they are not
    # exported by RustPython's minimal WASI signal module.
    for uncatchable in (9, 19):
        previous = signal.getsignal(uncatchable)
        assert_raises(OSError, signal.signal, uncatchable, signal.SIG_IGN)
        assert signal.getsignal(uncatchable) is previous


# unix, and not WASI
if "win" not in sys.platform and sys.platform != "wasi":
    signal.signal(signal.SIGALRM, handler)
    assert signal.getsignal(signal.SIGALRM) is handler

    signal.alarm(1)
    time.sleep(2.0)
    assert signals == [signal.SIGALRM]

    signal.signal(signal.SIGALRM, signal.SIG_IGN)
    signal.alarm(1)
    time.sleep(2.0)

    assert signals == [signal.SIGALRM]

    signal.signal(signal.SIGALRM, handler)
    signal.alarm(1)
    time.sleep(2.0)

    assert signals == [signal.SIGALRM, signal.SIGALRM]

    # A handler may call signal.signal(), and the usual reason is to disarm
    # itself. Reading the handler table while the handler runs used to be a
    # crash rather than a rearm.
    rearmed = []

    def rearm(signum, frame):
        rearmed.append(signum)
        signal.signal(signal.SIGALRM, signal.SIG_IGN)

    signal.signal(signal.SIGALRM, rearm)
    signal.raise_signal(signal.SIGALRM)
    assert rearmed == [signal.SIGALRM], rearmed
    assert signal.getsignal(signal.SIGALRM) is signal.SIG_IGN

    # The same goes for arming a different signal from inside a handler.
    armed = []

    def target(signum, frame):
        armed.append("target")

    def arm_other(signum, frame):
        armed.append("arm_other")
        signal.signal(signal.SIGUSR2, target)

    signal.signal(signal.SIGUSR1, arm_other)
    signal.raise_signal(signal.SIGUSR1)
    assert armed == ["arm_other"], armed
    assert signal.getsignal(signal.SIGUSR2) is target

    signal.raise_signal(signal.SIGUSR2)
    assert armed == ["arm_other", "target"], armed

    signal.signal(signal.SIGALRM, signal.SIG_DFL)
    signal.signal(signal.SIGUSR1, signal.SIG_DFL)
    signal.signal(signal.SIGUSR2, signal.SIG_DFL)


def check_windows_wakeup_reporting():
    if sys.platform != "win32":
        return

    import errno
    import socket

    def fill_socket(write):
        # A nonblocking Windows send can briefly fail before TCP has filled
        # the receiving socket. Require repeated one-byte failures to settle.
        deadline = time.monotonic() + 5.0
        written = 0
        blocked = 0
        for _ in range(4096):
            assert time.monotonic() < deadline, "socket fill timed out"
            assert written < 16 * 1024 * 1024, "socket fill exceeded byte bound"
            try:
                written += write.send(b"x" * 65536)
            except BlockingIOError:
                try:
                    written += write.send(b"x")
                except BlockingIOError:
                    blocked += 1
                    if blocked == 4:
                        return
                    time.sleep(0.050)
                    continue
            blocked = 0
        raise AssertionError("socket did not fill within the attempt bound")

    # Use numeric loopback addresses and bounded connection/receive calls.
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.settimeout(5.0)
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as write:
            write.settimeout(5.0)
            write.connect(listener.getsockname())
            read, _ = listener.accept()
            with read:
                read.settimeout(5.0)
                read.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
                write.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 4096)
                write.setblocking(False)
                old_handler = signal.getsignal(signal.SIGINT)
                old_hook = sys.unraisablehook
                old_fd = signal.set_wakeup_fd(-1)
                errors = []
                delivered = []

                def raise_wakeup_signal():
                    delivered.clear()
                    # Reinstall for each raise: Windows CRT handlers can reset.
                    signal.signal(
                        signal.SIGINT, lambda signum, frame: delivered.append(signum)
                    )
                    signal.raise_signal(signal.SIGINT)
                    assert delivered == [signal.SIGINT]

                try:
                    sys.unraisablehook = errors.append
                    signal.set_wakeup_fd(write.fileno())
                    raise_wakeup_signal()
                    assert read.recv(1) == bytes([signal.SIGINT])
                    assert not errors

                    for options in (
                        {},
                        {"warn_on_full_buffer": True},
                        {"warn_on_full_buffer": False},
                        {},
                    ):
                        fill_socket(write)
                        signal.set_wakeup_fd(write.fileno(), **options)
                        raise_wakeup_signal()
                        if options.get("warn_on_full_buffer", True):
                            assert len(errors) == 1, errors
                            error = errors.pop()
                            assert error.exc_type is BlockingIOError
                            assert error.exc_value.winerror == errno.WSAEWOULDBLOCK
                            assert error.exc_value.errno in (
                                errno.EAGAIN,
                                errno.EWOULDBLOCK,
                            )
                            assert error.object is None
                            assert error.exc_traceback is error.exc_value.__traceback__
                            assert error.err_msg == (
                                "Exception ignored while trying to send "
                                "to the signal wakeup fd"
                            )
                        else:
                            assert not errors

                    # False suppresses a full buffer, not other send failures.
                    signal.set_wakeup_fd(write.fileno(), warn_on_full_buffer=False)
                    write.close()
                    raise_wakeup_signal()
                    assert len(errors) == 1, errors
                    assert errors[0].exc_type is OSError
                    assert errors[0].exc_value.winerror == errno.WSAENOTSOCK
                    assert errors[0].err_msg == (
                        "Exception ignored while trying to send to the signal wakeup fd"
                    )
                finally:
                    signal.set_wakeup_fd(old_fd)
                    signal.signal(signal.SIGINT, old_handler)
                    sys.unraisablehook = old_hook


check_windows_wakeup_reporting()
