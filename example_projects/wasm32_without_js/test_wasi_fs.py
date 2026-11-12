"""WASI preview1 filesystem contract tests for the no-JS guest and wasm32-wasip1."""

from __future__ import annotations

import argparse
import ast
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

_PARSER = argparse.ArgumentParser(add_help=False)
_PARSER.add_argument("--runtime", type=Path)
_PARSER.add_argument("--module", type=Path)
_PARSER.add_argument("--wasmer", type=Path)
_PARSER.add_argument("--wasi-module", type=Path)
_EARLY, _ = _PARSER.parse_known_args()
RUNTIME_MODE = _EARLY.runtime is not None
WASMER_MODE = _EARLY.wasmer is not None

REPO_ROOT = Path(__file__).resolve().parents[2]
LIB_DIR = REPO_ROOT / "Lib"


class WasiFsTests(unittest.TestCase):
    runtime: Path | None = None
    module: Path | None = None
    wasmer: Path | None = None
    wasi_module: Path | None = None

    def run_snippet(
        self,
        source,
        *,
        dirs=(),
        ro_dirs=(),
        marker="wasi-fs-ok",
        extra_runtime_args=(),
    ):
        wrapped = (
            f"{source}\n{marker!r}\n"
            if RUNTIME_MODE
            else f"{source}\nprint({marker!r})\n"
        )
        dirs = _map_mounts(dirs)
        ro_dirs = _map_mounts(ro_dirs)
        if RUNTIME_MODE:
            command = [str(self.runtime), str(self.module), *extra_runtime_args]
            for host, guest in dirs:
                command += ["--dir", _mount_spec(host, guest)]
            for host, guest in ro_dirs:
                command += ["--ro-dir", _mount_spec(host, guest)]
            command += ["-c", wrapped]
            env = None
        else:
            if ro_dirs:
                self.skipTest("wasmer mode has no read-only preopen flag")
            command = [str(self.wasmer), "run"]
            for host, guest in dirs:
                command += ["--volume", f"{host}:{guest}"]
            guest_pwd = dirs[0][1] if dirs else _guest_path_for_index(0)
            command += [
                "--env",
                f"PWD={guest_pwd}",
                "--env",
                f"RUSTPYTHONPATH={LIB_DIR}",
                str(self.wasi_module),
                "--",
                "-c",
                wrapped,
            ]
            env = os.environ.copy()
        result = subprocess.run(
            command,
            capture_output=True,
            encoding="utf-8",
            timeout=120,
            env=env,
        )
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn(marker, result.stdout)
        return result

    @unittest.skipUnless(
        RUNTIME_MODE and WASMER_MODE, "requires both wasm modules for comparison"
    )
    def test_errno_numbers_match_wasip1(self):
        # Compare exports, not hard-coded errno numbers or platform errno from
        # the Python interpreter running this harness.
        expression = "{name: getattr(errno, name) for name in dir(errno) if name.startswith('E') and isinstance(getattr(errno, name), int)}"
        with tempfile.TemporaryDirectory() as directory:
            commands = (
                [
                    str(self.runtime),
                    str(self.module),
                    "--dir",
                    directory,
                    "-c",
                    "import errno\n" + expression,
                ],
                [
                    str(self.wasmer),
                    "run",
                    str(self.wasi_module),
                    "--",
                    "-c",
                    "import errno; print(" + expression + ")",
                ],
            )
            tables = []
            for command in commands:
                result = subprocess.run(
                    command, capture_output=True, encoding="utf-8", timeout=120
                )
                self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
                tables.append(ast.literal_eval(result.stdout.strip()))
            guest, wasi = tables
            self.assertNotIn("ESHUTDOWN", guest)
            self.assertLessEqual(guest.keys(), wasi.keys())
            for name in guest:
                self.assertEqual(guest[name], wasi[name], name)
            for name in ("ENOENT", "EACCES", "ENOTCAPABLE", "EINVAL"):
                self.assertIn(name, guest)
                self.assertIn(name, wasi)

    def test_errno_exception_classes(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_snippet(
                """
import errno
for name, cls in (
    ('ENOENT', FileNotFoundError),
    ('EACCES', PermissionError),
    ('EAGAIN', BlockingIOError),
    ('EPIPE', BrokenPipeError),
    ('EINVAL', OSError),
    ('ENOTSUP', OSError),
):
    value = getattr(errno, name)
    assert type(OSError(value, 'contract')) is cls, name
    assert getattr(errno, errno.errorcode[value]) == value, name
assert errno.EAGAIN == errno.EWOULDBLOCK
""",
                dirs=[(directory, "/")],
            )

    def test_read_write_seek_truncate_and_buffering(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lent file"
            path.write_bytes(b"host data")
            (Path(directory) / "buffered").touch()
            self.run_snippet(
                """
import _io
raw = _io.FileIO('lent file', 'rb+')
assert raw.seekable()
assert not raw.isatty()
assert raw.read() == b'host data'
assert raw.read() == b''
assert raw.seek(0) == 0
buf = bytearray(4)
assert raw.readinto(buf) == 4
assert buf == b'host'
assert raw.tell() == 4
assert raw.seek(-4, 1) == 0
assert raw.write(b'WASM') == 4
assert raw.truncate(4) == 4
raw.close()
with _io.FileIO('lent file', 'rb') as raw:
    assert raw.read() == b'WASM'

raw = _io.FileIO('buffered', 'wb')
data = b'x' * 200000
n = raw.write(data)
assert isinstance(n, int) and 0 < n <= len(data)
assert raw.seek(0) == 0
raw.truncate(0)
with _io.BufferedWriter(raw) as buffered:
    assert buffered.write(data) == len(data)
with _io.BufferedReader(_io.FileIO('buffered', 'rb')) as buffered:
    buffered.seek(0)
    assert buffered.read() == data
""",
                dirs=[(directory, "/")],
                marker="fileio-ok",
            )
            self.assertEqual(path.read_bytes(), b"WASM")
            self.assertEqual((Path(directory) / "buffered").read_bytes(), b"x" * 200000)

    @unittest.skipUnless(RUNTIME_MODE, "no-JS runner caps a transfer at 64 KiB")
    def test_runner_caps_raw_write_at_64kib(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "short").touch()
            self.run_snippet(
                """
import _io
raw = _io.FileIO('short', 'wb')
assert raw.write(b'x' * 200000) == 65536
""",
                dirs=[(directory, "/")],
                marker="short-ok",
            )

    @unittest.skipUnless(RUNTIME_MODE, "no-JS guest has no stdio descriptors")
    def test_no_stdio_descriptors(self):
        with tempfile.TemporaryDirectory() as directory:
            self.run_snippet(
                """
import _io, errno, sys
assert 'posix' not in sys.builtin_module_names
assert 'nt' not in sys.builtin_module_names
for fd in (0, 1, 2, 9999):
    try:
        _io.FileIO(fd, 'rb')
    except OSError as e:
        assert e.errno == errno.EBADF, (fd, e)
    else:
        assert False, fd
try:
    _io.FileIO(3, 'rb')
except OSError as e:
    assert e.errno == errno.EISDIR, e
else:
    assert False
""",
                dirs=[(directory, "/")],
                marker="nostdio-ok",
            )

    @unittest.skipUnless(RUNTIME_MODE, "read-only preopen is a runner flag")
    def test_readonly_preopen_denies_write_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.py").write_bytes(b"answer = 42\n")
            self.run_snippet(
                """
import _io, errno
with _io.FileIO('source.py', 'rb') as f:
    assert f.read() == b'answer = 42\\n'
    assert f.fileno() >= 3
for mode in ('w', 'a', 'x', 'r+'):
    try:
        _io.FileIO('source.py', mode)
    except PermissionError as e:
        assert e.errno in (errno.ENOTCAPABLE, errno.EACCES, errno.EPERM, errno.EROFS), (mode, e)
    else:
        assert False, mode
""",
                ro_dirs=[(directory, "/")],
                marker="readonly-fs-ok",
            )
            self.assertEqual((root / "source.py").read_bytes(), b"answer = 42\n")

    def test_missing_file_and_escapes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.py").write_bytes(b"answer = 42\n")
            host_outside = str(root / "source.py")
            self.run_snippet(
                f"""
import _io, errno
RUNTIME = {RUNTIME_MODE!r}
DotDot = PermissionError if RUNTIME else FileNotFoundError
try:
    _io.FileIO('missing', 'r')
except FileNotFoundError as e:
    assert e.errno == errno.ENOENT, e
else:
    assert False

def denied(path, cls):
    try:
        _io.FileIO(path, 'r')
    except cls:
        return
    else:
        assert False, path

denied('../escape', DotDot)
denied({host_outside!r}, FileNotFoundError)
""",
                dirs=[(directory, "/")],
                marker="errors-ok",
            )

    @unittest.skipUnless(os.name == "posix", "Unix symlink and byte-path contract")
    def test_confined_symlinks_and_byte_paths(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            tempfile.TemporaryDirectory() as other,
        ):
            root = Path(directory)
            (root / "inside").write_bytes(b"inside")
            (root / "café").write_bytes(b"coffee")
            (Path(other) / "secret").write_bytes(b"secret")
            (root / "internal").symlink_to("inside")
            (root / "external").symlink_to(Path(other) / "secret")
            (root / "external_dir").symlink_to(other, target_is_directory=True)
            have_bytes_name = True
            try:
                with open(os.fsencode(directory) + b"/raw-\xff", "wb") as file:
                    file.write(b"bytes")
            except OSError:
                have_bytes_name = False
            byte_name_check = (
                """
try:
    _io.FileIO(b'raw-\\xff', 'r')
except Exception as e:
    assert isinstance(e, OSError)
else:
    assert False
"""
                if have_bytes_name
                else ""
            )
            self.run_snippet(
                f"""
import _io
RUNTIME = {RUNTIME_MODE!r}
Escape = PermissionError if RUNTIME else OSError
with _io.FileIO('internal', 'r') as f:
    assert f.read() == b'inside'
with _io.FileIO('café'.encode(), 'r') as f:
    assert f.read() == b'coffee'
{byte_name_check}
def denied(path):
    try:
        _io.FileIO(path, 'r')
    except Escape:
        return
    else:
        assert False, path

denied('external')
denied('external_dir/secret')
""",
                dirs=[(directory, "/")],
                marker="symlink-ok",
            )

    def test_bytes_identity_precedence_and_descriptor_rights(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "inside").write_bytes(b"unchanged")
            self.run_snippet(
                f"""
import _io, errno
RUNTIME = {RUNTIME_MODE!r}
missing_str = 'no-such-file'
try:
    _io.FileIO(missing_str, 'r')
except OSError as e:
    assert e.errno == errno.ENOENT
    assert e.filename is missing_str
else:
    assert False

missing = b'missing-\\xff'
try:
    _io.FileIO(missing, 'r')
except OSError as e:
    assert e.filename is missing
else:
    assert False

class PathLike:
    calls = 0
    def __fspath__(self):
        self.calls += 1
        return missing
pathlike = PathLike()
try:
    _io.FileIO(pathlike, 'r')
except OSError as e:
    assert e.filename is pathlike
else:
    assert False
assert pathlike.calls == 1

name = 'inside'
try:
    _io.FileIO(name, 'r')
except OSError:
    assert False
else:
    pass

try:
    _io.FileIO(b'bad\\0path', '?')
except ValueError as e:
    assert 'null' in str(e)
else:
    assert False

class PathFailure(Exception):
    pass
class FailingPath:
    def __fspath__(self):
        raise PathFailure('path first')
try:
    _io.FileIO(FailingPath(), '?')
except PathFailure:
    pass
else:
    assert False

opener_calls = []
def opener(name, flags):
    opener_calls.append((name, flags))
    raise AssertionError('must validate path first')
try:
    _io.FileIO(b'bad\\0path', 'r', opener=opener)
except ValueError as e:
    assert 'null' in str(e)
else:
    assert False
assert not opener_calls

before = pathlike.calls
class OpenerSeen(Exception):
    pass
def lend(name, flags):
    assert name is pathlike
    raise OpenerSeen
try:
    _io.FileIO(pathlike, 'r', opener=lend)
except OpenerSeen:
    pass
else:
    assert False
assert pathlike.calls == before + 1

if RUNTIME:
    with _io.FileIO('inside', 'r') as original:
        writer = _io.FileIO(original.fileno(), 'r+', closefd=False)
        try:
            writer.write(b'changed')
        except OSError:
            pass
        else:
            assert False, 'readonly descriptor accepted write'
        try:
            writer.truncate(0)
        except OSError:
            pass
        else:
            assert False, 'readonly descriptor accepted truncate'
        writer.close()
        original.seek(0)
        assert original.read() == b'unchanged'
""",
                dirs=[(directory, "/")],
                marker="bytes-contract-ok",
            )
            self.assertEqual((root / "inside").read_bytes(), b"unchanged")

    @unittest.skipUnless(RUNTIME_MODE, "fail-closed imports are a runner check")
    def test_wasi_imports_require_a_preopen(self):
        result = subprocess.run(
            [str(self.runtime), str(self.module), "-c", "1"],
            capture_output=True,
            encoding="utf-8",
            timeout=120,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unexpected import: wasi_snapshot_preview1.", result.stderr)


def _guest_path_for_index(index):
    """Guest path for the Nth test mount.

    The no-JS runner's current directory is fixed at `/`, so the first mount
    stays there. Wasmer mounts at `/work`, `/work2`, ... so writes reach the
    host: a volume on guest `/` does not persist to the host.
    """
    if RUNTIME_MODE:
        return "/" if index == 0 else f"/work{index + 1}"
    return "/work" if index == 0 else f"/work{index + 1}"


def _map_mounts(mounts):
    return [
        (host, _guest_path_for_index(index)) for index, (host, _) in enumerate(mounts)
    ]


def _mount_spec(host, guest):
    if guest in ("/", ""):
        return str(host)
    return f"{host}::{guest}"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--module", type=Path)
    parser.add_argument("--wasmer", type=Path)
    parser.add_argument("--wasi-module", type=Path)
    args, remaining = parser.parse_known_args()
    if (args.runtime is None) != (args.module is None):
        parser.error("provide both --runtime and --module")
    if (args.wasmer is None) != (args.wasi_module is None):
        parser.error("provide both --wasmer and --wasi-module")
    if args.runtime is None and args.wasmer is None:
        parser.error("provide --runtime/--module and/or --wasmer/--wasi-module")
    if args.runtime is not None:
        WasiFsTests.runtime = args.runtime.resolve(strict=True)
        WasiFsTests.module = args.module.resolve(strict=True)
    if args.wasmer is not None:
        WasiFsTests.wasmer = args.wasmer.resolve(strict=True)
        WasiFsTests.wasi_module = args.wasi_module.resolve(strict=True)
    unittest.main(argv=[__file__, *remaining])
