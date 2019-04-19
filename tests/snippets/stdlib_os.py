import os 

from testutils import assert_raises

fd = os.open('README.md', 0)
assert fd > 0

os.close(fd)
assert_raises(OSError, lambda: os.read(fd, 10))
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
assert ENV_KEY not in os.environ
assert os.getenv(ENV_KEY) == None

if os.name == "posix":
	os.putenv(ENV_KEY, ENV_VALUE)
	os.unsetenv(ENV_KEY)
	assert os.getenv(ENV_KEY) == None


if os.name == "nt":
	assert os.sep == "\\"
else:
	assert os.sep == "/"

class TestWithTempDir():
	def __enter__(self):
		if os.name == "nt":
			base_folder = os.environ["TEMP"]
		else:
			base_folder = "/tmp"
		name = base_folder + os.sep + "test_os"
		os.mkdir(name)
		self.name = name
		return name

	def __exit__(self, exc_type, exc_val, exc_tb):
		for f in os.listdir(self.name):
			# Currently don't support dir delete.
			os.remove(self.name + os.sep + f)
		os.rmdir(self.name)


FILE_NAME = "test1"
CONTENT = b"testing"
CONTENT2 = b"rustpython"
CONTENT3 = b"BOYA"

with TestWithTempDir() as tmpdir:
	fname = tmpdir + os.sep + FILE_NAME
	open(fname, "wb")
	fd = os.open(fname, 1)
	assert os.write(fd, CONTENT2) == len(CONTENT2)
	assert os.write(fd, CONTENT3) == len(CONTENT3)
	os.close(fd)

	fd = os.open(fname, 0)
	assert os.read(fd, len(CONTENT2)) == CONTENT2
	assert os.read(fd, len(CONTENT3)) == CONTENT3
	os.close(fd)
