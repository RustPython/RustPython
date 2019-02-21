import os 

from testutils import assert_raises

assert os.open('README.md', 0) > 0


assert_raises(FileNotFoundError, lambda: os.open('DOES_NOT_EXIST', 0))


assert os.O_RDONLY == 0
assert os.O_WRONLY == 1
assert os.O_RDWR == 2
