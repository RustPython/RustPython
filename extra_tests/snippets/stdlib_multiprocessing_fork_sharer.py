"""Fork after the resource sharer thread is running must not hang the child.

DupFd starts a daemon thread blocked in Listener.accept(). The child of a
later fork used to stall in _ResourceSharer._afterfork while closing that
listener.
"""

import multiprocessing
import os


def child():
    print("child-ok", flush=True)


if __name__ == "__main__":
    multiprocessing.freeze_support()
    if not hasattr(os, "fork") or "fork" not in multiprocessing.get_all_start_methods():
        print("skipped (no fork)")
        raise SystemExit(0)

    from multiprocessing.resource_sharer import DupFd

    DupFd(os.dup(0))
    ctx = multiprocessing.get_context("fork")
    proc = ctx.Process(target=child)
    proc.daemon = True
    proc.start()
    proc.join(timeout=8)
    assert proc.exitcode == 0, proc.exitcode
    print("ok")
