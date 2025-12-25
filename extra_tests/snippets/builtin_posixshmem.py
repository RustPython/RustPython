import os
import sys

if os.name != "posix":
    sys.exit(0)

import _posixshmem

name = f"/rp_posixshmem_{os.getpid()}"
fd = _posixshmem.shm_open(name, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
os.close(fd)
_posixshmem.shm_unlink(name)
