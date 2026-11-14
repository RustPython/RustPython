"""Actual WASM guest path opening through an explicit read-only root."""

import argparse
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


class HostFsTests(unittest.TestCase):
    runtime: Path
    module: Path

    def run_guest(self, root, source):
        return subprocess.run(
            [
                str(self.runtime),
                str(self.module),
                "--host-root",
                str(root),
                "-c",
                source,
            ],
            capture_output=True,
            encoding="utf-8",
            timeout=120,
        )

    def test_readonly_open_and_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "source.py").write_bytes(b"answer = 42\n")
            result = self.run_guest(
                root,
                f"""
import _io, sys
assert 'posix' not in sys.builtin_module_names
with _io.FileIO('source.py', 'rb') as f:
    assert f.read() == b'answer = 42\\n'
    assert f.fileno() >= 3
for mode in ('w', 'a', 'x', 'r+'):
    try:
        _io.FileIO('source.py', mode)
    except OSError as e:
        assert e.errno == 30, (mode, e)
    else:
        assert False, mode
for path, expected in [('missing', 2), ('../escape', 13), ({str(root / "source.py")!r}, 13)]:
    try:
        _io.FileIO(path, 'r')
    except OSError as e:
        assert e.errno == expected, (path, e)
    else:
        assert False, path
'readonly-fs-ok'
""",
            )
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertEqual(result.stdout, "'readonly-fs-ok'\n")
            self.assertEqual((root / "source.py").read_bytes(), b"answer = 42\n")

    @unittest.skipUnless(os.name == "posix", "Unix symlink and byte-path contract")
    def test_confined_symlinks_and_byte_paths(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            tempfile.TemporaryDirectory() as other,
        ):
            root = Path(directory)
            (root / "inside").write_bytes(b"inside")
            (Path(other) / "secret").write_bytes(b"secret")
            (root / "internal").symlink_to("inside")
            (root / "external").symlink_to(Path(other) / "secret")
            (root / "external_dir").symlink_to(other, target_is_directory=True)
            with open(os.fsencode(directory) + b"/raw-\xff", "wb") as file:
                file.write(b"bytes")
            result = self.run_guest(
                root,
                """
import _io
with _io.FileIO('internal', 'r') as f:
    assert f.read() == b'inside'
with _io.FileIO(b'raw-\\xff', 'r') as f:
    assert f.read() == b'bytes'
for path in ('external', 'external_dir/secret'):
    try:
        _io.FileIO(path, 'r')
    except OSError as e:
        assert e.errno == 13, (path, e)
    else:
        assert False, path
'symlink-ok'
""",
            )
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_bytes_identity_precedence_and_descriptor_rights(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "inside").write_bytes(b"unchanged")
            result = self.run_guest(
                root,
                r"""
import _io
missing = b'missing-\xff'
try:
    _io.FileIO(missing, 'r')
except OSError as e:
    assert e.errno == 2
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
    assert e.errno == 2
    assert e.filename is pathlike
else:
    assert False
assert pathlike.calls == 1

try:
    _io.FileIO(b'bad\0path', '?')
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
    _io.FileIO(b'bad\0path', 'r', opener=opener)
except ValueError as e:
    assert 'null' in str(e)
else:
    assert False
assert not opener_calls

readonly_name = b'inside'
try:
    _io.FileIO(readonly_name, 'w')
except OSError as e:
    assert e.errno == 30
    assert e.filename is readonly_name
else:
    assert False

# The custom opener gets the original object, with one __fspath__ call.
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

# Changing FileIO mode around a borrowed descriptor cannot grant OS rights.
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
'bytes-contract-ok'
""",
            )
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertEqual(result.stdout, "'bytes-contract-ok'\n")
            self.assertEqual((root / "inside").read_bytes(), b"unchanged")

    def test_import_requires_explicit_root(self):
        with tempfile.NamedTemporaryFile() as file:
            for options in ([], ["--host-file", file.name]):
                result = subprocess.run(
                    [str(self.runtime), str(self.module), *options, "-c", "1"],
                    capture_output=True,
                    encoding="utf-8",
                    timeout=120,
                )
                self.assertEqual(result.returncode, 2)
                self.assertIn("unexpected import: rustpython_host_", result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    HostFsTests.runtime = args.runtime.resolve()
    HostFsTests.module = args.module.resolve()
    unittest.main(argv=[__file__, *remaining])
