"""_thread.get_native_id is the kernel thread id and changes after fork."""

import _thread
import os

nid = _thread.get_native_id()
assert isinstance(nid, int) and nid > 0
assert _thread.get_ident() != 0

if hasattr(os, "fork"):
    r, w = os.pipe()
    pid = os.fork()
    if pid == 0:
        os.write(w, str(_thread.get_native_id()).encode())
        os._exit(0)
    os.close(w)
    child_nid = int(os.read(r, 64).decode())
    os.close(r)
    _, status = os.waitpid(pid, 0)
    assert os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, status
    assert child_nid != nid, (nid, child_nid)
    assert child_nid > 0

print("ok")
