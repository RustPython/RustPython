import os
import time
import stat

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
SYMLINK_FILE = "symlink"
SYMLINK_FOLDER = "symlink1"
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

	symlink_file = tmpdir + os.sep + SYMLINK_FILE
	os.symlink(fname, symlink_file)
	symlink_folder = tmpdir + os.sep + SYMLINK_FOLDER
	os.symlink(folder, symlink_folder)

	names = set()
	paths = set()
	dirs = set()
	dirs_no_symlink = set()
	files = set()
	files_no_symlink = set()
	symlinks = set()
	for dir_entry in os.scandir(tmpdir):
		names.add(dir_entry.name)
		paths.add(dir_entry.path)
		if dir_entry.is_dir():
			assert stat.S_ISDIR(dir_entry.stat().st_mode) == True
			dirs.add(dir_entry.name)
		if dir_entry.is_dir(follow_symlinks=False):
			assert stat.S_ISDIR(dir_entry.stat().st_mode) == True
			dirs_no_symlink.add(dir_entry.name)
		if dir_entry.is_file():
			files.add(dir_entry.name)
			assert stat.S_ISREG(dir_entry.stat().st_mode) == True
		if dir_entry.is_file(follow_symlinks=False):
			files_no_symlink.add(dir_entry.name)
			assert stat.S_ISREG(dir_entry.stat().st_mode) == True
		if dir_entry.is_symlink():
			symlinks.add(dir_entry.name)

	assert names == set([FILE_NAME, FILE_NAME2, FOLDER, SYMLINK_FILE, SYMLINK_FOLDER])
	assert paths == set([fname, fname2, folder, symlink_file, symlink_folder])
	assert dirs == set([FOLDER, SYMLINK_FOLDER])
	assert dirs_no_symlink == set([FOLDER])
	assert files == set([FILE_NAME, FILE_NAME2, SYMLINK_FILE])
	assert files_no_symlink == set([FILE_NAME, FILE_NAME2])
	assert symlinks == set([SYMLINK_FILE, SYMLINK_FOLDER])

	# Stat
	stat_res = os.stat(fname)
	print(stat_res.st_mode)
	assert stat.S_ISREG(stat_res.st_mode) == True
	print(stat_res.st_ino)
	print(stat_res.st_dev)
	print(stat_res.st_nlink)
	print(stat_res.st_uid)
	print(stat_res.st_gid)
	print(stat_res.st_size)
	assert stat_res.st_size == len(CONTENT2) + len(CONTENT3)

	# stat default is follow_symlink=True
	os.stat(fname).st_ino == os.stat(symlink_file).st_ino
	os.stat(fname).st_mode == os.stat(symlink_file).st_mode

	os.stat(fname, follow_symlinks=False).st_ino == os.stat(symlink_file, follow_symlinks=False).st_ino
	os.stat(fname, follow_symlinks=False).st_mode == os.stat(symlink_file, follow_symlinks=False).st_mode

	# os.path
	assert os.path.exists(fname) == True
	assert os.path.exists("NO_SUCH_FILE") == False
	assert os.path.isfile(fname) == True
	assert os.path.isdir(folder) == True
	assert os.path.isfile(folder) == False
	assert os.path.isdir(fname) == False
