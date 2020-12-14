import subprocess
import sys
import unittest

# This only tests that it prints something in order
# to avoid changing this test if the bytecode changes

# These tests start a new process instead of redirecting stdout because
# stdout is being written to by rust code, which currently can't be
# redirected by reassigning sys.stdout


class TestDis(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.setup = """
import dis
def tested_func(): pass
"""
        cls.command = (sys.executable, "-c")

    def test_dis(self):
        test_code = f"""
{self.setup}
dis.dis(tested_func)
dis.dis("x = 2; print(x)")
"""

        result = subprocess.run(
            self.command + (test_code,), capture_output=True
        )
        self.assertNotEqual("", result.stdout.decode())
        self.assertEqual("", result.stderr.decode())

    def test_disassemble(self):
        test_code = f"""
{self.setup}
dis.disassemble(tested_func)
"""
        result = subprocess.run(
            self.command + (test_code,), capture_output=True
        )
        # In CPython this would raise an AttributeError, not a
        # TypeError because dis is implemented in python in CPython and
        # as such the type mismatch wouldn't be caught immeadiately
        self.assertIn("TypeError", result.stderr.decode())

        test_code = f"""
{self.setup}
dis.disassemble(tested_func.__code__)
"""
        result = subprocess.run(
            self.command + (test_code,), capture_output=True
        )
        self.assertNotEqual("", result.stdout.decode())
        self.assertEqual("", result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
