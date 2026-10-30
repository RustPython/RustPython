"""RLock must validate a timeout before updating recursion ownership."""

import _thread
import unittest


class LockPrecedenceTests(unittest.TestCase):
    def test_reentrant_errors_do_not_increment_count(self):
        for args, error in [
            ((False, 0), ValueError),
            ((True, _thread.TIMEOUT_MAX + 1), OverflowError),
        ]:
            with self.subTest(args=args):
                lock = _thread.RLock()
                lock.acquire()
                with self.assertRaises(error):
                    lock.acquire(*args)
                self.assertEqual(lock._recursion_count(), 1)
                lock.release()
                self.assertFalse(lock.locked())


if __name__ == "__main__":
    unittest.main()
