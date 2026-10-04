"""Closing a listening socket after fork must not hang.

A thread blocked in accept() must not keep the socket lock across the
syscall. After fork the child copies that lock; close() in the child
would wait forever for a thread that does not exist there.
"""

import os
import socket
import threading
import time

if not hasattr(os, "fork"):
    print("skipped (no fork)")
    raise SystemExit(0)

listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
listener.bind(("127.0.0.1", 0))
listener.listen(1)
started = threading.Event()


def accept_forever():
    started.set()
    try:
        listener.accept()
    except OSError:
        pass


t = threading.Thread(target=accept_forever, daemon=True)
t.start()
assert started.wait(5)

# Give accept() time to enter the syscall.
time.sleep(0.05)

pid = os.fork()
if pid == 0:
    listener.close()
    os._exit(0)

_, status = os.waitpid(pid, 0)
assert os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, status

# Wake the parent accept thread. Closing the fd from another thread is
# not a portable way to interrupt accept().
addr = listener.getsockname()
with socket.create_connection(addr, timeout=2):
    pass
t.join(timeout=5)
assert not t.is_alive()
listener.close()
print("ok")
