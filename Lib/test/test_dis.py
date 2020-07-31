import unittest
import dis
from test.support import captured_stdout


# This only tests that it prints something in order
# to avoid changing this test if the bytecode changes
class TestDis(unittest.TestCase):
    def test_dis(self):
        test_cases = (self.test_dis, "x = 2; print(x)")

        for case in test_cases:
            with self.subTest(case=case):
                with captured_stdout() as stdout:
                    dis.dis(case)
                self.assertNotEqual("", stdout.getvalue())

    def test_disassemble(self):
        # In CPython this would raise an AttributeError, not a
        # TypeError because dis is implemented in python in CPython and
        # as such the type mismatch wouldn't be caught immeadiately
        with self.assertRaises(TypeError):
            dis.disassemble(self.test_disassemble)
        with captured_stdout() as stdout:
            dis.dis(self.test_disassemble.__code__)
        self.assertNotEqual("", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
