"""Transfers that wait for a peer must not hold the buffer they were given.

A pipe or a socket answers when the other end does, which may be never. The
buffer is exported for the whole call, so it cannot be resized meanwhile, but
nothing else about it changes: another thread can still read it, write to it,
and the interpreter can still stop the world. An implementation that holds the
buffer's storage for the duration of the wait takes all of that away, and a
thread parked on that storage never reaches a safepoint, so a collection that
wants every thread stopped ends up waiting for the peer too.
"""

import gc
import os
import socket
import threading
import time

# The peer acts after DELAY; the checks below have to finish well inside it.
DELAY = 1.0
SLACK = DELAY / 2


def measure(buf, expected_len, writable):
    """Time each operation on `buf` that does not need the peer, separately, so
    a failure names the one that waited rather than the group."""
    elapsed = {}

    def timed(name, operation):
        start = time.monotonic()
        value = operation()
        elapsed[name] = time.monotonic() - start
        return value

    assert timed("len", lambda: len(buf)) == expected_len, len(buf)
    assert isinstance(timed("bytes", lambda: bytes(buf)), bytes)
    if writable:
        timed("setitem", lambda: buf.__setitem__(0, buf[0]))
    timed("gc.collect", gc.collect)
    return elapsed


def run(buf, blocking_call, release_peer, writable):
    started = threading.Event()
    expected_len = len(buf)
    result = []

    def transfer():
        started.set()
        result.append(blocking_call(buf))

    def peer():
        time.sleep(DELAY)
        release_peer()

    threads = [threading.Thread(target=transfer), threading.Thread(target=peer)]
    for t in threads:
        t.start()
    started.wait()
    time.sleep(0.2)  # the transfer is now waiting on its peer

    elapsed = measure(buf, expected_len, writable)
    waited = ["%s %.2fs" % item for item in elapsed.items() if item[1] >= SLACK]
    assert not waited, "waited on the peer: " + ", ".join(waited)

    # The transfer is still in flight, so its export is still held and the
    # buffer cannot be resized. An operating system that took the whole
    # transfer without a peer leaves nothing here to observe.
    assert not result, "the transfer finished without its peer"
    try:
        buf.append(0)
    except BufferError:
        pass
    else:
        raise AssertionError("append during an export should raise BufferError")

    for t in threads:
        t.join()
    return result[0]


# --- reading: the buffer is written into, so nothing else may touch it at all


read_fd, write_fd = os.pipe()
pipe = open(read_fd, "rb", buffering=0)
try:
    target = bytearray(16)
    n = run(target, pipe.readinto, lambda: os.write(write_fd, b"pipe"), writable=False)
    assert n == 4, n
    assert bytes(target[:4]) == b"pipe", bytes(target)
finally:
    pipe.close()
    os.close(write_fd)

if hasattr(socket, "socketpair"):
    left, right = socket.socketpair()
    try:
        target = bytearray(16)
        n = run(target, left.recv_into, lambda: right.send(b"socket"), writable=False)
        assert n == 6, n
        assert bytes(target[:6]) == b"socket", bytes(target)
    finally:
        left.close()
        right.close()


# --- writing: the buffer is only read, so it stays writable meanwhile


read_fd, write_fd = os.pipe()
sink = open(write_fd, "wb", buffering=0)
try:
    # More than any pipe will hold, so the write cannot finish on its own.
    source = bytearray(4 * 1024 * 1024)
    drained = []

    def drain():
        with open(read_fd, "rb", buffering=0) as f:
            while True:
                chunk = f.read(1 << 16)
                if not chunk:
                    break
                drained.append(len(chunk))

    reader = threading.Thread(target=drain, daemon=True)
    # One unbuffered write() reports what it transferred, which a signal can
    # cut short, so the reader is measured against that rather than the source.
    written = run(source, sink.write, reader.start, writable=True)
    sink.close()
    reader.join()
    assert sum(drained) == written, (sum(drained), written)
finally:
    if not sink.closed:
        sink.close()

if hasattr(socket, "socketpair"):
    left, right = socket.socketpair()
    try:
        # How much a connection holds before it makes the sender wait is the
        # operating system's to decide, and asking for a small send buffer does
        # not settle it -- a socketpair is already connected, and on Windows it
        # is a loopback pair whose receiver has a window of its own. So fill it
        # until it refuses rather than guess a size that outruns it.
        left.setblocking(False)
        filled = 0
        while True:
            try:
                filled += left.send(bytes(1 << 16))
            except (BlockingIOError, InterruptedError):
                break
        left.setblocking(True)

        source = bytearray(1 << 16)
        received = []

        def receive():
            wanted = filled + len(source)
            while sum(received) < wanted:
                chunk = right.recv(1 << 16)
                if not chunk:
                    break
                received.append(len(chunk))

        reader = threading.Thread(target=receive, daemon=True)
        run(source, left.sendall, reader.start, writable=True)
        reader.join()
        assert sum(received) == filled + len(source), (
            sum(received),
            filled,
            len(source),
        )
    finally:
        left.close()
        right.close()

print("ok")
