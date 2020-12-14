import os
import time
import stat
import sys

from testutils import assert_raises

fd = os.open('README.md', os.O_RDONLY)
assert fd > 0

os.close(fd)
assert_raises(OSError, lambda: os.read(fd, 10))
assert_raises(FileNotFoundError,
              lambda: os.open('DOES_NOT_EXIST', os.O_RDONLY))
assert_raises(FileNotFoundError,
              lambda: os.open('DOES_NOT_EXIST', os.O_WRONLY))
assert_raises(FileNotFoundError,
              lambda: os.rename('DOES_NOT_EXIST', 'DOES_NOT_EXIST 2'))

# sendfile only supports in_fd as non-socket on linux and solaris
if hasattr(os, "sendfile") and sys.platform.startswith("linux"):
    src_fd = os.open('README.md', os.O_RDONLY)
    dest_fd = os.open('destination.md', os.O_RDWR | os.O_CREAT)
    src_len = os.stat('README.md').st_size

    bytes_sent = os.sendfile(dest_fd, src_fd, 0, src_len)
    assert src_len == bytes_sent

    os.lseek(dest_fd, 0, 0)
    assert os.read(src_fd, src_len) == os.read(dest_fd, bytes_sent)
    os.close(src_fd)
    os.close(dest_fd)

try:
    os.open('DOES_NOT_EXIST', 0)
except OSError as err:
    assert err.errno == 2

assert os.O_RDONLY == 0
assert os.O_WRONLY == 1
assert os.O_RDWR == 2

ENV_KEY = "TEST_ENV_VAR"
ENV_VALUE = "value"

assert os.getenv(ENV_KEY) is None
assert ENV_KEY not in os.environ
assert os.getenv(ENV_KEY, 5) == 5
os.environ[ENV_KEY] = ENV_VALUE
assert ENV_KEY in os.environ
assert os.getenv(ENV_KEY) == ENV_VALUE
del os.environ[ENV_KEY]
assert ENV_KEY not in os.environ
assert os.getenv(ENV_KEY) is None

if os.name == "posix":
    os.putenv(ENV_KEY, ENV_VALUE)
    os.unsetenv(ENV_KEY)
    assert os.getenv(ENV_KEY) is None

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
    assert os.altsep is None
    assert os.pathsep == ":"

assert os.fspath("Testing") == "Testing"
assert os.fspath(b"Testing") == b"Testing"
assert_raises(TypeError, lambda: os.fspath([1, 2, 3]))


class TestWithTempDir():
    def __enter__(self):
        if os.name == "nt":
            base_folder = os.environ["TEMP"]
        else:
            base_folder = "/tmp"

        name = os.path.join(base_folder,
                            "rustpython_test_os_" + str(int(time.time())))

        while os.path.isdir(name):
            name = name + "_"

        os.mkdir(name)
        self.name = name
        return name

    def __exit__(self, exc_type, exc_val, exc_tb):
        pass


class TestWithTempCurrentDir():
    def __enter__(self):
        self.prev_cwd = os.getcwd()

    def __exit__(self, exc_type, exc_val, exc_tb):
        os.chdir(self.prev_cwd)


FILE_NAME = "test1"
FILE_NAME2 = "test2"
FILE_NAME3 = "test3"
SYMLINK_FILE = "symlink"
SYMLINK_FOLDER = "symlink1"
FOLDER = "dir1"
CONTENT = b"testing"
CONTENT2 = b"rustpython"
CONTENT3 = b"BOYA"

with TestWithTempDir() as tmpdir:
    fname = os.path.join(tmpdir, FILE_NAME)
    fd = os.open(fname, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
    assert os.write(fd, CONTENT2) == len(CONTENT2)
    os.close(fd)

    fd = os.open(fname, os.O_WRONLY | os.O_APPEND)
    assert os.write(fd, CONTENT3) == len(CONTENT3)
    os.close(fd)

    assert_raises(FileExistsError,
                  lambda: os.open(fname, os.O_WRONLY | os.O_CREAT | os.O_EXCL))

    fd = os.open(fname, os.O_RDONLY)
    assert os.read(fd, len(CONTENT2)) == CONTENT2
    assert os.read(fd, len(CONTENT3)) == CONTENT3
    os.close(fd)

    fname3 = os.path.join(tmpdir, FILE_NAME3)
    os.rename(fname, fname3)
    assert os.path.exists(fname) is False
    assert os.path.exists(fname3) is True

    fd = os.open(fname3, 0)
    assert os.read(fd, len(CONTENT2) + len(CONTENT3)) == CONTENT2 + CONTENT3
    os.close(fd)

    assert not os.isatty(fd)

    # TODO: get os.lseek working on windows
    if os.name != 'nt':
        fd = os.open(fname3, 0)
        assert os.read(fd, len(CONTENT2)) == CONTENT2
        assert os.read(fd, len(CONTENT3)) == CONTENT3
        os.lseek(fd, len(CONTENT2), os.SEEK_SET)
        assert os.read(fd, len(CONTENT3)) == CONTENT3
        os.close(fd)

    os.rename(fname3, fname)
    assert os.path.exists(fname3) is False
    assert os.path.exists(fname) is True

    # wait a little bit to ensure that the file times aren't the same
    time.sleep(0.1)

    fname2 = os.path.join(tmpdir, FILE_NAME2)
    with open(fname2, "wb"):
        pass
    folder = os.path.join(tmpdir, FOLDER)
    os.mkdir(folder)

    symlink_file = os.path.join(tmpdir, SYMLINK_FILE)
    os.symlink(fname, symlink_file)
    symlink_folder = os.path.join(tmpdir, SYMLINK_FOLDER)
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
            assert stat.S_ISDIR(dir_entry.stat().st_mode) is True
            dirs.add(dir_entry.name)
        if dir_entry.is_dir(follow_symlinks=False):
            assert stat.S_ISDIR(dir_entry.stat().st_mode) is True
            dirs_no_symlink.add(dir_entry.name)
        if dir_entry.is_file():
            files.add(dir_entry.name)
            assert stat.S_ISREG(dir_entry.stat().st_mode) is True
        if dir_entry.is_file(follow_symlinks=False):
            files_no_symlink.add(dir_entry.name)
            assert stat.S_ISREG(dir_entry.stat().st_mode) is True
        if dir_entry.is_symlink():
            symlinks.add(dir_entry.name)

    assert names == set(
        [FILE_NAME, FILE_NAME2, FOLDER, SYMLINK_FILE, SYMLINK_FOLDER])
    assert paths == set([fname, fname2, folder, symlink_file, symlink_folder])
    assert dirs == set([FOLDER, SYMLINK_FOLDER])
    assert dirs_no_symlink == set([FOLDER])
    assert files == set([FILE_NAME, FILE_NAME2, SYMLINK_FILE])
    assert files_no_symlink == set([FILE_NAME, FILE_NAME2])
    assert symlinks == set([SYMLINK_FILE, SYMLINK_FOLDER])

    # Stat
    stat_res = os.stat(fname)
    print(stat_res.st_mode)
    assert stat.S_ISREG(stat_res.st_mode) is True
    print(stat_res.st_ino)
    print(stat_res.st_dev)
    print(stat_res.st_nlink)
    print(stat_res.st_uid)
    print(stat_res.st_gid)
    print(stat_res.st_size)
    assert stat_res.st_size == len(CONTENT2) + len(CONTENT3)
    print(stat_res.st_atime)
    print(stat_res.st_ctime)
    print(stat_res.st_mtime)
    # test that it all of these times are greater than the 10 May 2019,
    # when this test was written
    assert stat_res.st_atime > 1557500000
    assert stat_res.st_ctime > 1557500000
    assert stat_res.st_mtime > 1557500000

    bytes_stats_res = os.stat(fname.encode())

    stat_file2 = os.stat(fname2)
    print(stat_file2.st_ctime)
    assert stat_file2.st_ctime > stat_res.st_ctime

    # wait a little bit to ensures that the access/modify time will change
    time.sleep(0.1)

    old_atime = stat_res.st_atime
    old_mtime = stat_res.st_mtime

    fd = os.open(fname, os.O_RDWR)
    os.write(fd, CONTENT)
    os.fsync(fd)

    # wait a little bit to ensures that the access/modify time is different
    time.sleep(0.1)

    os.read(fd, 1)
    os.fsync(fd)
    os.close(fd)

    # retrieve update file stats
    stat_res = os.stat(fname)
    print(stat_res.st_atime)
    print(stat_res.st_ctime)
    print(stat_res.st_mtime)
    if os.name != "nt":
        # access time on windows has a resolution ranging from 1 hour to 1 day
        # https://docs.microsoft.com/en-gb/windows/desktop/api/minwinbase/ns-minwinbase-filetime
        assert stat_res.st_atime > old_atime, "Access time should be update"
        assert stat_res.st_atime > stat_res.st_mtime
    assert stat_res.st_mtime > old_mtime, "Modified time should be update"

    # stat default is follow_symlink=True
    os.stat(fname).st_ino == os.stat(symlink_file).st_ino
    os.stat(fname).st_mode == os.stat(symlink_file).st_mode

    os.stat(fname, follow_symlinks=False).st_ino == os.stat(
        symlink_file, follow_symlinks=False).st_ino
    os.stat(fname, follow_symlinks=False).st_mode == os.stat(
        symlink_file, follow_symlinks=False).st_mode

    # os.chmod
    if os.name != "nt":
        os.chmod(fname, 0o666)
        assert oct(os.stat(fname).st_mode) == '0o100666'

# os.chown
    if os.name != "nt":
        # setup
        root_in_posix = False
        if hasattr(os, 'geteuid'):
            root_in_posix = (os.geteuid() == 0)
        try:
            import pwd
            all_users = [u.pw_uid for u in pwd.getpwall()]
        except (ImportError, AttributeError):
            all_users = []

        fname1 = os.path.join(tmpdir, FILE_NAME)
        fname2 = os.path.join(tmpdir, FILE_NAME2)
        fd = os.open(fname2, os.O_RDONLY)

        # test chown without root permissions
        if not root_in_posix and len(all_users) > 1:
            uid_1, uid_2 = all_users[:2]
            gid = os.stat(fname1).st_gid
            assert_raises(PermissionError,
                          lambda: os.chown(fname1, uid_1, gid))
            assert_raises(PermissionError,
                          lambda: os.chown(fname1, uid_2, gid))

        # test chown with root perm and file name
        if root_in_posix and len(all_users) > 1:
            uid_1, uid_2 = all_users[:2]
            gid = os.stat(fname1).st_gid
            os.chown(fname1, uid_1, gid)
            uid = os.stat(fname1).st_uid
            assert uid == uid_1
            os.chown(fname1, uid_2, gid)
            uid = os.stat(fname1).st_uid
            assert uid == uid_2

        # test chown with root perm and file descriptor
        if root_in_posix and len(all_users) > 1:
            uid_1, uid_2 = all_users[:2]
            gid = os.stat(fd).st_gid
            os.chown(fd, uid_1, gid)
            uid = os.stat(fd).st_uid
            assert uid == uid_1
            os.chown(fd, uid_2, gid)
            uid = os.stat(fd).st_uid
            assert uid == uid_2

        # test gid change
        if hasattr(os, 'getgroups'):
            groups = os.getgroups()
            if len(groups) > 1:
                gid_1, gid_2 = groups[:2]
                uid = os.stat(fname1).st_uid

                os.chown(fname1, uid, gid_1)
                gid = os.stat(fname1).st_gid
                assert gid == gid_1

                os.chown(fname1, uid, gid_2)
                gid = os.stat(fname1).st_gid
                assert gid == gid_2

        # teardown
        os.close(fd)

    # os.path
    assert os.path.exists(fname) is True
    assert os.path.exists("NO_SUCH_FILE") is False
    assert os.path.isfile(fname) is True
    assert os.path.isdir(folder) is True
    assert os.path.isfile(folder) is False
    assert os.path.isdir(fname) is False

    assert os.path.basename(fname) == FILE_NAME
    assert os.path.dirname(fname) == tmpdir

    with TestWithTempCurrentDir():
        os.chdir(tmpdir)
        assert os.path.realpath(os.getcwd()) == os.path.realpath(tmpdir)
        assert os.path.exists(FILE_NAME)

# supports
assert isinstance(os.supports_fd, set)
assert isinstance(os.supports_dir_fd, set)
assert isinstance(os.supports_follow_symlinks, set)

# get pid
assert isinstance(os.getpid(), int)

# unix
if "win" not in sys.platform:
    assert isinstance(os.getegid(), int)
    assert isinstance(os.getgid(), int)
    assert isinstance(os.getsid(os.getpid()), int)
    assert isinstance(os.getuid(), int)
    assert isinstance(os.geteuid(), int)
    assert isinstance(os.getppid(), int)
    assert isinstance(os.getpgid(os.getpid()), int)

    if os.getuid() != 0:
        assert_raises(PermissionError, lambda: os.setgid(42))
        assert_raises(PermissionError, lambda: os.setegid(42))
        assert_raises(PermissionError, lambda: os.setpgid(os.getpid(), 42))
        assert_raises(PermissionError, lambda: os.setuid(42))
        assert_raises(PermissionError, lambda: os.seteuid(42))
        assert_raises(PermissionError, lambda: os.setreuid(42, 42))
        assert_raises(PermissionError, lambda: os.setresuid(42, 42, 42))

    # pty
    a, b = os.openpty()
    assert isinstance(a, int)
    assert isinstance(b, int)
    assert isinstance(os.ttyname(b), str)
    assert_raises(OSError, lambda: os.ttyname(9999))
    os.close(b)
    os.close(a)

    # os.get_blocking, os.set_blocking
    # TODO: windows support should be added for below functions
    # os.pipe,
    # os.set_inheritable, os.get_inheritable,
    rfd, wfd = os.pipe()
    try:
        os.write(wfd, CONTENT2)
        assert os.read(rfd, len(CONTENT2)) == CONTENT2
        assert not os.get_inheritable(rfd)
        assert not os.get_inheritable(wfd)
        os.set_inheritable(rfd, True)
        os.set_inheritable(wfd, True)
        assert os.get_inheritable(rfd)
        assert os.get_inheritable(wfd)
        os.set_inheritable(rfd, True)
        os.set_inheritable(wfd, True)
        os.set_inheritable(rfd, True)
        os.set_inheritable(wfd, True)
        assert os.get_inheritable(rfd)
        assert os.get_inheritable(wfd)

        assert os.get_blocking(rfd)
        assert os.get_blocking(wfd)
        os.set_blocking(rfd, False)
        os.set_blocking(wfd, False)
        assert not os.get_blocking(rfd)
        assert not os.get_blocking(wfd)
        os.set_blocking(rfd, True)
        os.set_blocking(wfd, True)
        os.set_blocking(rfd, True)
        os.set_blocking(wfd, True)
        assert os.get_blocking(rfd)
        assert os.get_blocking(wfd)
    finally:
        os.close(rfd)
        os.close(wfd)

# os.pipe2
if sys.platform.startswith('linux') or sys.platform.startswith('freebsd'):
    rfd, wfd = os.pipe2(0)
    try:
        os.write(wfd, CONTENT2)
        assert os.read(rfd, len(CONTENT2)) == CONTENT2
        assert os.get_inheritable(rfd)
        assert os.get_inheritable(wfd)
        assert os.get_blocking(rfd)
        assert os.get_blocking(wfd)
    finally:
        os.close(rfd)
        os.close(wfd)
    rfd, wfd = os.pipe2(os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        os.write(wfd, CONTENT2)
        assert os.read(rfd, len(CONTENT2)) == CONTENT2
        assert not os.get_inheritable(rfd)
        assert not os.get_inheritable(wfd)
        assert not os.get_blocking(rfd)
        assert not os.get_blocking(wfd)
    finally:
        os.close(rfd)
        os.close(wfd)

with TestWithTempDir() as tmpdir:
    for i in range(0, 4):
        file_name = os.path.join(tmpdir, 'file' + str(i))
        with open(file_name, 'w') as f:
            f.write('test')

    expected_files = ['file0', 'file1', 'file2', 'file3']

    dir_iter = os.scandir(tmpdir)
    collected_files = [dir_entry.name for dir_entry in dir_iter]

    assert set(collected_files) == set(expected_files)

    with assert_raises(StopIteration):
        next(dir_iter)

    dir_iter.close()

    expected_files_bytes = [(file.encode(), os.path.join(tmpdir,
                                                         file).encode())
                            for file in expected_files]

    dir_iter_bytes = os.scandir(tmpdir.encode())
    collected_files_bytes = [(dir_entry.name, dir_entry.path)
                             for dir_entry in dir_iter_bytes]

    assert set(collected_files_bytes) == set(expected_files_bytes)

    dir_iter_bytes.close()

    collected_files = os.listdir(tmpdir)
    assert set(collected_files) == set(expected_files)

    collected_files = os.listdir(tmpdir.encode())
    assert set(collected_files) == set(
        [file.encode() for file in expected_files])

    with TestWithTempCurrentDir():
        os.chdir(tmpdir)
        with os.scandir() as dir_iter:
            collected_files = [dir_entry.name for dir_entry in dir_iter]
            assert set(collected_files) == set(expected_files)

# system()
if "win" not in sys.platform:
    assert os.system('ls') == 0
    assert os.system('{') != 0

    for arg in [None, 1, 1.0, TabError]:
        assert_raises(TypeError, os.system, arg)

if sys.platform.startswith("win"):
	winver = sys.getwindowsversion()

	# the biggest value of wSuiteMask (https://docs.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-osversioninfoexa#members).
	all_masks = 0x00000004 | 0x00000400 | 0x00004000 | 0x00000080 | 0x00000002 | 0x00000040 | 0x00000200 | \
		0x00000100 | 0x00000001 | 0x00000020 | 0x00002000 | 0x00000010 | 0x00008000 | 0x00020000

	# We really can't test if the results are correct, so it just checks for meaningful value
	assert winver.major > 0
	assert winver.minor >= 0
	assert winver.build > 0
	assert winver.platform == 2
	assert isinstance(winver.service_pack, str)
	assert 0 <= winver.suite_mask <= all_masks
	assert 1 <= winver.product_type <= 3

	# XXX if platform_version is implemented correctly, this'll break on compatiblity mode or a build without manifest
	assert winver.major == winver.platform_version[0]
	assert winver.minor == winver.platform_version[1]
	assert winver.build == winver.platform_version[2]


