import os
import unittest
import uuid


@unittest.skipUnless(os.name == "posix", "requires POSIX shared memory")
class PosixShmemTests(unittest.TestCase):
    def test_shm_open_and_unlink(self):
        import _posixshmem

        name = f"/rustpython_posixshmem_{uuid.uuid4().hex}"
        fd = _posixshmem.shm_open(name, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600)
        try:
            os.close(fd)
        finally:
            _posixshmem.shm_unlink(name)


if __name__ == "__main__":
    unittest.main()
