from test.support import import_helper, threading_helper
syslog = import_helper.import_module("syslog") #skip if not supported
from test import support
import sys
import threading
import time
import unittest

# XXX(nnorwitz): This test sucks.  I don't know of a platform independent way
# to verify that the messages were really logged.
# The only purpose of this test is to verify the code doesn't crash or leak.

class Test(unittest.TestCase):

    def tearDown(self):
        syslog.closelog()

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_openlog(self):
        syslog.openlog('python')
        # Issue #6697.
        self.assertRaises(UnicodeEncodeError, syslog.openlog, '\uD800')

    def test_syslog(self):
        syslog.openlog('python')
        syslog.syslog('test message from python test_syslog')
        syslog.syslog(syslog.LOG_ERR, 'test error from python test_syslog')

    def test_syslog_implicit_open(self):
        syslog.closelog() # Make sure log is closed
        syslog.syslog('test message from python test_syslog')
        syslog.syslog(syslog.LOG_ERR, 'test error from python test_syslog')

    def test_closelog(self):
        syslog.openlog('python')
        syslog.closelog()
        syslog.closelog()  # idempotent operation

    def test_setlogmask(self):
        mask = syslog.LOG_UPTO(syslog.LOG_WARNING)
        oldmask = syslog.setlogmask(mask)
        self.assertEqual(syslog.setlogmask(0), mask)
        self.assertEqual(syslog.setlogmask(oldmask), mask)

    # TODO: RUSTPYTHON; AssertionError: 12 is not false
    @unittest.expectedFailure
    def test_log_mask(self):
        mask = syslog.LOG_UPTO(syslog.LOG_WARNING)
        self.assertTrue(mask & syslog.LOG_MASK(syslog.LOG_WARNING))
        self.assertTrue(mask & syslog.LOG_MASK(syslog.LOG_ERR))
        self.assertFalse(mask & syslog.LOG_MASK(syslog.LOG_INFO))

    def test_openlog_noargs(self):
        syslog.openlog()
        syslog.syslog('test message from python test_syslog')

    # TODO: RUSTPYTHON; AttributeError: module 'sys' has no attribute 'getswitchinterval'
    @unittest.expectedFailure
    @threading_helper.requires_working_threading()
    def test_syslog_threaded(self):
        start = threading.Event()
        stop = False
        def opener():
            start.wait(10)
            i = 1
            while not stop:
                syslog.openlog(f'python-test-{i}')  # new string object
                i += 1
        def logger():
            start.wait(10)
            while not stop:
                syslog.syslog('test message from python test_syslog')

        orig_si = sys.getswitchinterval()
        support.setswitchinterval(1e-9)
        try:
            threads = [threading.Thread(target=opener)]
            threads += [threading.Thread(target=logger) for k in range(10)]
            with threading_helper.start_threads(threads):
                start.set()
                time.sleep(0.1)
                stop = True
        finally:
            sys.setswitchinterval(orig_si)


if __name__ == "__main__":
    unittest.main()
