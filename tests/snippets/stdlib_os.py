import os
import time

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

assert os.curdir == "."
assert os.pardir == ".."
assert os.extsep == "."

if os.name == "nt":
	assert os.sep == "\\"
	assert os.linesep == "\r\n"
	assert os.altsep == "/"
	assert os.pathsep == ";"
else:
	assert os.sep == "/"
	assert os.linesep == "\n"
	assert os.altsep == None
	assert os.pathsep == ":"

class TestWithTempDir():
	def __enter__(self):
		if os.name == "nt":
			base_folder = os.environ["TEMP"]
		else:
			base_folder = "/tmp"
		name = base_folder + os.sep + "rustpython_test_os_" + str(int(time.time()))
		os.mkdir(name)
		self.name = name
		return name

	def __exit__(self, exc_type, exc_val, exc_tb):
		# TODO: Delete temp dir
		pass


FILE_NAME = "test1"
FILE_NAME2 = "test2"
FOLDER = "dir1"
CONTENT = b"testing"
CONTENT2 = b"rustpython"
CONTENT3 = b"BOYA"

with TestWithTempDir() as tmpdir:
	fname = tmpdir + os.sep + FILE_NAME
	with open(fname, "wb"):
		pass
	fd = os.open(fname, 1)
	assert os.write(fd, CONTENT2) == len(CONTENT2)
	assert os.write(fd, CONTENT3) == len(CONTENT3)
	os.close(fd)

	fd = os.open(fname, 0)
	assert os.read(fd, len(CONTENT2)) == CONTENT2
	assert os.read(fd, len(CONTENT3)) == CONTENT3
	os.close(fd)

	fname2 = tmpdir + os.sep + FILE_NAME2
	with open(fname2, "wb"):
		pass
	folder = tmpdir + os.sep + FOLDER
	os.mkdir(folder)

	names = set()
	paths = set()
	dirs = set()
	files = set()
	for dir_entry in os.scandir(tmpdir):
		names.add(dir_entry.name)
		paths.add(dir_entry.path)
		if dir_entry.is_dir():
			dirs.add(dir_entry.name)
		if dir_entry.is_file():
			files.add(dir_entry.name)

	assert names == set([FILE_NAME, FILE_NAME2, FOLDER])
	assert paths == set([fname, fname2, folder])
	assert dirs == set([FOLDER])
	assert files == set([FILE_NAME, FILE_NAME2])

	# Stat
	stat_res = os.stat(fname)
	print(stat_res.st_mode)
	print(stat_res.st_ino)
	print(stat_res.st_dev)
	print(stat_res.st_nlink)
	print(stat_res.st_uid)
	print(stat_res.st_gid)
	print(stat_res.st_size)
	assert stat_res.st_size == len(CONTENT2) + len(CONTENT3)
