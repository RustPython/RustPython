import unittest
import subprocess
import sys
import os

# These should go into a test_sys.py file, but given these are the only
# ones being tested currently that seems unnecessary.


class SysExecutableTest(unittest.TestCase):

    # This is a copy of test.test_sys.SysModuleTest.test_executable from cpython.
    def cpython_tests(self):
        # sys.executable should be absolute
        self.assertEqual(os.path.abspath(sys.executable), sys.executable)

        # Issue #7774: Ensure that sys.executable is an empty string if argv[0]
        # has been set to a non existent program name and Python is unable to
        # retrieve the real program name

        # For a normal installation, it should work without 'cwd'
        # argument. For test runs in the build directory, see #7774.
        python_dir = os.path.dirname(os.path.realpath(sys.executable))
        p = subprocess.Popen(
            [
                "nonexistent",
                "-c",
                'import sys; print(sys.executable.encode("ascii", "backslashreplace"))',
            ],
            executable=sys.executable,
            stdout=subprocess.PIPE,
            cwd=python_dir,
        )
        stdout = p.communicate()[0]
        executable = stdout.strip().decode("ASCII")
        p.wait()
        self.assertIn(
            executable,
            ["b''", repr(sys.executable.encode("ascii", "backslashreplace"))],
        )

    def test_no_follow_symlink(self):
        paths = [os.path.abspath("test_symlink"), "./test_symlink"]
        for path in paths:
            with self.subTest(path=path):
                os.symlink(sys.executable, path)
                command = [
                    path,
                    "-c",
                    "import sys; print(sys.executable, end='')",
                ]
                try:
                    process = subprocess.run(command, capture_output=True)
                finally:
                    os.remove(path)
                self.assertEqual(
                    os.path.abspath(path), process.stdout.decode("utf-8")
                )


if __name__ == "__main__":
    unittest.main()
