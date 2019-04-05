import os 

from testutils import assert_raises

fd = os.open('README.md', 0)
assert fd > 0

assert len(os.read(fd, 10)) == 10
assert len(os.read(fd, 5)) == 5

assert_raises(OSError, lambda: os.read(fd + 1, 10))
os.close(fd)
assert_raises(OSError, lambda: os.read(fd, 10))

assert_raises(FileNotFoundError, lambda: os.open('DOES_NOT_EXIST', 0))


assert os.O_RDONLY == 0
assert os.O_WRONLY == 1
assert os.O_RDWR == 2
