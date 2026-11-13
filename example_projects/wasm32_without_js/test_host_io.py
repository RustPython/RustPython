"""Contract tests for opt-in, real host descriptor I/O in the WASM guest."""

import argparse
import subprocess
import tempfile
import unittest
from pathlib import Path


class HostIoTests(unittest.TestCase):
    runtime: Path
    module: Path

    def run_guest(self, path, source):
        return subprocess.run(
            [
                str(self.runtime),
                str(self.module),
                "--host-file",
                str(path),
                "-c",
                source,
            ],
            capture_output=True,
            encoding="utf-8",
            timeout=120,
        )

    def test_real_file_operations_and_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lent file"
            path.write_bytes(b"host data")
            result = self.run_guest(
                path,
                """
import _io, sys
assert 'posix' not in sys.builtin_module_names
assert 'nt' not in sys.builtin_module_names
for fd in (0, 1, 2, 4, 9999):
    try:
        _io.FileIO(fd, 'rb')
    except OSError as e:
        assert e.errno == 9, (fd, e)
    else:
        assert False, fd
raw = _io.FileIO(3, 'rb+', closefd=False)
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
raw = _io.FileIO(3, 'rb', closefd=True)
assert raw.read() == b''
assert raw.seek(0) == 0
assert raw.read() == b'WASM'
raw.close()
try:
    _io.FileIO(3, 'rb')
except OSError as e:
    assert e.errno == 9
else:
    assert False
'fileio-ok'
""",
            )
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertEqual(result.stdout, "'fileio-ok'\n")
            self.assertEqual(path.read_bytes(), b"WASM")

    def test_short_transfers_with_buffering(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "buffered"
            path.touch()
            result = self.run_guest(
                path,
                """
import _io
raw = _io.FileIO(3, 'wb', closefd=False)
data = b'x' * 200000
assert raw.write(data) == 65536
assert raw.seek(0) == 0
raw.truncate(0)
with _io.BufferedWriter(raw) as buffered:
    assert buffered.write(data) == len(data)
with _io.BufferedReader(_io.FileIO(3, 'rb')) as buffered:
    buffered.seek(0)
    assert buffered.read() == data
'buffered-ok'
""",
            )
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertEqual(path.read_bytes(), b"x" * 200000)

    def test_capability_requires_explicit_opt_in(self):
        result = subprocess.run(
            [str(self.runtime), str(self.module), "-c", "1"],
            capture_output=True,
            encoding="utf-8",
            timeout=120,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("unexpected import: rustpython_host_io_v1.", result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--module", type=Path, required=True)
    args, remaining = parser.parse_known_args()
    HostIoTests.runtime = args.runtime.resolve()
    HostIoTests.module = args.module.resolve()
    unittest.main(argv=[__file__, *remaining])
