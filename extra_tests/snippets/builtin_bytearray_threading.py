"""Acquiring a bytearray export must be atomic with respect to resizing."""

import threading


def exercise_exports():
    payload = b"x" * 65536
    value = bytearray(payload)
    barrier = threading.Barrier(3, timeout=5)
    errors = []

    def worker(operation):
        try:
            for _ in range(256):
                barrier.wait()
                operation()
                barrier.wait()
        except BaseException as exc:
            errors.append(exc)
            barrier.abort()

    def resize():
        try:
            value.clear()
            value.extend(payload)
        except BufferError:
            pass

    def decode():
        result = value.decode("ascii")
        assert result in ("", "x" * len(payload))

    def export():
        with memoryview(value) as view:
            size = len(view)
            assert size in (0, len(payload))
            # The acquired export keeps its original length and contents even
            # if the resizing thread runs between these two reads.
            assert view.tobytes() == payload[:size]
            assert len(view) == size

    threads = [
        threading.Thread(target=worker, args=(operation,), daemon=True)
        for operation in (resize, decode, export)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=10)
    assert not any(thread.is_alive() for thread in threads), "worker did not finish"
    if errors:
        raise errors[0]


exercise_exports()
