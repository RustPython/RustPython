"""RLock must validate a timeout before updating recursion ownership."""

import _thread
import unittest


class LockPrecedenceTests(unittest.TestCase):
    def test_reentrant_errors_do_not_increment_count(self):
        for args, error in [
            ((False, 0), ValueError),
            ((True, -2), ValueError),
            ((True, _thread.TIMEOUT_MAX + 1), OverflowError),
            ((True, float("inf")), OverflowError),
            ((True, float("nan")), ValueError),
        ]:
            with self.subTest(args=args):
                lock = _thread.RLock()
                lock.acquire()
                with self.assertRaises(error):
                    lock.acquire(*args)
                self.assertEqual(lock._recursion_count(), 1)
                lock.release()
                self.assertFalse(lock.locked())

    def test_valid_reentrant_modes(self):
        lock = _thread.RLock()
        for args in [(), (False,), (True, 0), (True, 0.001), (False, -1)]:
            self.assertTrue(lock.acquire(*args))
        self.assertEqual(lock._recursion_count(), 5)
        for _ in range(5):
            lock.release()
        self.assertFalse(lock.locked())


if __name__ == "__main__":
    unittest.main()
