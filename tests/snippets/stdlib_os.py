import os 

from testutils import assert_raises

fd = os.open('README.md', 0)
assert fd > 0

os.close(fd)
assert_raises(OSError, lambda: os.read(fd, 10))

FNAME = "test_file_that_no_one_will_have_on_disk"
CONTENT = b"testing"
CONTENT2 = b"rustpython"
CONTENT3 = b"BOYA"

class TestWithFile():
	def __enter__(self):
		open(FNAME, "wb")
		return FNAME

	def __exit__(self, exc_type, exc_val, exc_tb):
		os.remove(FNAME)


with TestWithFile() as fname:
	fd = os.open(fname, 1)
	assert os.write(fd, CONTENT2) == len(CONTENT2)
	assert os.write(fd, CONTENT3) == len(CONTENT3)
	os.close(fd)

	fd = os.open(fname, 0)
	assert os.read(fd, len(CONTENT2)) == CONTENT2
	assert os.read(fd, len(CONTENT3)) == CONTENT3
	os.close(fd)


assert_raises(FileNotFoundError, lambda: os.open('DOES_NOT_EXIST', 0))


assert os.O_RDONLY == 0
assert os.O_WRONLY == 1
assert os.O_RDWR == 2

ENV_KEY = "TEST_ENV_VAR"
ENV_VALUE = "value"

assert os.getenv(ENV_KEY) == None
assert ENV_KEY not in os.environ
assert os.getenv(ENV_KEY, 5) == 5
os.environ[ENV_KEY] = ENV_VALUE
assert ENV_KEY in os.environ
assert os.getenv(ENV_KEY) == ENV_VALUE
del os.environ[ENV_KEY]
os.unsetenv(ENV_KEY)
assert ENV_KEY not in os.environ
assert os.getenv(ENV_KEY) == None
