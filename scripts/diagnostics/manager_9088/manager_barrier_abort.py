# Source-extracted canonical manager cleanup diagnostic; execution awaits review.
import gc
import itertools
import multiprocessing
import multiprocessing.util as util
import operator
import os
import threading
import time
import unittest
import weakref
from multiprocessing import process
from multiprocessing.util import _run_finalizers

import test.support
from test import support

WAIT_ACTIVE_CHILDREN_TIMEOUT = 5.0


def setUpModule():
    print("module setup begin", flush=True)
    multiprocessing.set_start_method("spawn", force=True)
    util.get_temp_dir()
    print("module setup complete", flush=True)


class BaseMixin(object):
    @classmethod
    def setUpClass(cls):
        cls.dangling = (
            multiprocessing.process._dangling.copy(),
            threading._dangling.copy(),
        )

    @classmethod
    def tearDownClass(cls):
        # bpo-26762: Some multiprocessing objects like Pool create reference
        # cycles. Trigger a garbage collection to break these cycles.
        print("BaseMixin: before test.support.gc_collect()", flush=True)
        test.support.gc_collect()
        print("BaseMixin: after test.support.gc_collect()", flush=True)

        print(
            "BaseMixin: before processes = set(multiprocessing.process._dangling) - set(cls.dangling[0])",
            flush=True,
        )
        processes = set(multiprocessing.process._dangling) - set(cls.dangling[0])
        print(
            "BaseMixin: after processes = set(multiprocessing.process._dangling) - set(cls.dangling[0])",
            flush=True,
        )
        if processes:
            test.support.environment_altered = True
            support.print_warning(f"Dangling processes: {processes}")
        processes = None

        print(
            "BaseMixin: before threads = set(threading._dangling) - set(cls.dangling[1])",
            flush=True,
        )
        threads = set(threading._dangling) - set(cls.dangling[1])
        print(
            "BaseMixin: after threads = set(threading._dangling) - set(cls.dangling[1])",
            flush=True,
        )
        if threads:
            test.support.environment_altered = True
            support.print_warning(f"Dangling threads: {threads}")
        threads = None


class ManagerMixin(BaseMixin):
    TYPE = "manager"
    Process = multiprocessing.Process
    Event = property(operator.attrgetter("manager.Event"))
    Barrier = property(operator.attrgetter("manager.Barrier"))

    @classmethod
    def Pool(cls, *args, **kwds):
        return cls.manager.Pool(*args, **kwds)

    list = property(operator.attrgetter("manager.list"))
    dict = property(operator.attrgetter("manager.dict"))

    @classmethod
    def setUpClass(cls):
        print("ManagerMixin: before super().setUpClass()", flush=True)
        super().setUpClass()
        print("ManagerMixin: after super().setUpClass()", flush=True)
        print(
            "ManagerMixin: before cls.manager = multiprocessing.Manager()", flush=True
        )
        cls.manager = multiprocessing.Manager()
        print("ManagerMixin: after cls.manager = multiprocessing.Manager()", flush=True)

    @classmethod
    def tearDownClass(cls):
        # only the manager process should be returned by active_children()
        # but this can take a bit on slow machines, so wait a few seconds
        # if there are other children too (see #17395)
        timeout = WAIT_ACTIVE_CHILDREN_TIMEOUT
        print("ManagerMixin: before start_time = time.monotonic()", flush=True)
        start_time = time.monotonic()
        print("ManagerMixin: after start_time = time.monotonic()", flush=True)
        for _ in support.sleeping_retry(timeout, error=False):
            if len(multiprocessing.active_children()) <= 1:
                break
        else:
            dt = time.monotonic() - start_time
            support.environment_altered = True
            support.print_warning(
                f"multiprocessing.Manager still has "
                f"{multiprocessing.active_children()} "
                f"active children after {dt:.1f} seconds"
            )

        print("ManagerMixin: before gc.collect", flush=True)
        gc.collect()  # do garbage collection
        print("ManagerMixin: after gc.collect", flush=True)
        print("ManagerMixin: before _number_of_objects", flush=True)
        if cls.manager._number_of_objects() != 0:
            # This is not really an error since some tests do not
            # ensure that all processes which hold a reference to a
            # managed object have been joined.
            test.support.environment_altered = True
            support.print_warning(
                "Shared objects which still exist at manager shutdown:"
            )
            print("ManagerMixin: before _debug_info", flush=True)
            support.print_warning(cls.manager._debug_info())
            print("ManagerMixin: after _debug_info", flush=True)
        print("ManagerMixin: after _number_of_objects branch", flush=True)
        print("ManagerMixin: before cls.manager.shutdown()", flush=True)
        cls.manager.shutdown()
        print("ManagerMixin: after cls.manager.shutdown()", flush=True)
        print("ManagerMixin: before cls.manager.join()", flush=True)
        cls.manager.join()
        print("ManagerMixin: after cls.manager.join()", flush=True)
        cls.manager = None

        print("ManagerMixin: before super().tearDownClass()", flush=True)
        super().tearDownClass()
        print("ManagerMixin: after super().tearDownClass()", flush=True)


class BaseTestCase(object):
    ALLOWED_TYPES = ("processes", "manager", "threads")
    # If not empty, limit which start method suites run this class.
    START_METHODS: set[str] = set()
    start_method = None  # set by install_tests_in_module_dict()

    def assertTimingAlmostEqual(self, a, b):
        if CHECK_TIMINGS:
            self.assertAlmostEqual(a, b, 1)

    def assertReturnsIfImplemented(self, value, func, *args):
        try:
            res = func(*args)
        except NotImplementedError:
            pass
        else:
            return self.assertEqual(value, res)

    # For the sanity of Windows users, rather than crashing or freezing in
    # multiple ways.
    def __reduce__(self, *args):
        raise NotImplementedError("shouldn't try to pickle a test case")

    __reduce_ex__ = __reduce__


def _wait():
    # A crude wait/yield function not relying on synchronization primitives.
    time.sleep(0.01)


class Bunch(object):
    """
    A bunch of threads.
    """

    def __init__(self, namespace, f, args, n, wait_before_exit=False):
        """
        Construct a bunch of `n` threads running the same function `f`.
        If `wait_before_exit` is True, the threads won't terminate until
        do_finish() is called.
        """
        self.f = f
        self.args = args
        self.n = n
        self.started = namespace.DummyList()
        self.finished = namespace.DummyList()
        self._can_exit = namespace.Event()
        if not wait_before_exit:
            self._can_exit.set()

        threads = []
        for i in range(n):
            p = namespace.Process(target=self.task)
            p.daemon = True
            print("Bunch: before p.start()", flush=True)
            p.start()
            print("Bunch: after p.start()", flush=True)
            threads.append(p)

        def finalize(threads):
            for p in threads:
                print("Bunch: before p.join()", flush=True)
                p.join()
                print("Bunch: after p.join()", flush=True)

        self._finalizer = weakref.finalize(self, finalize, threads)

    def task(self):
        pid = os.getpid()
        self.started.append(pid)
        try:
            print("Bunch: before self.f(*self.args)", flush=True)
            self.f(*self.args)
            print("Bunch: after self.f(*self.args)", flush=True)
        finally:
            self.finished.append(pid)
            self._can_exit.wait(30)
            assert self._can_exit.is_set()

    def wait_for_started(self):
        while len(self.started) < self.n:
            _wait()

    def wait_for_finished(self):
        while len(self.finished) < self.n:
            _wait()

    def do_finish(self):
        self._can_exit.set()

    def close(self):
        print("Bunch: before self._finalizer()", flush=True)
        self._finalizer()
        print("Bunch: after self._finalizer()", flush=True)


class _TestBarrier(BaseTestCase):
    N = 5
    defaultTimeout = 30.0  # XXX Slow Windows buildbots need generous timeout

    def setUp(self):
        print(
            "Barrier: before self.barrier = self.Barrier(self.N, timeout=self.defaultTimeout)",
            flush=True,
        )
        self.barrier = self.Barrier(self.N, timeout=self.defaultTimeout)
        print(
            "Barrier: after self.barrier = self.Barrier(self.N, timeout=self.defaultTimeout)",
            flush=True,
        )

    def tearDown(self):
        print("Barrier: before self.barrier.abort()", flush=True)
        self.barrier.abort()
        print("Barrier: after self.barrier.abort()", flush=True)
        self.barrier = None

    def DummyList(self):
        if self.TYPE == "threads":
            return []
        elif self.TYPE == "manager":
            return self.manager.list()
        else:
            return _DummyList()

    def run_threads(self, f, args):
        print("Barrier: before b = Bunch(self, f, args, self.N-1)", flush=True)
        b = Bunch(self, f, args, self.N - 1)
        print("Barrier: after b = Bunch(self, f, args, self.N-1)", flush=True)
        try:
            print("Barrier: before f(*args)", flush=True)
            f(*args)
            print("Barrier: after f(*args)", flush=True)
            print("Barrier: before b.wait_for_finished()", flush=True)
            b.wait_for_finished()
            print("Barrier: after b.wait_for_finished()", flush=True)
        finally:
            print("Barrier: before b.close()", flush=True)
            b.close()
            print("Barrier: after b.close()", flush=True)

    @classmethod
    def _test_abort_f(cls, barrier, results1, results2):
        try:
            print("abort worker", os.getpid(), "before i = barrier.wait()", flush=True)
            i = barrier.wait()
            print("abort worker", os.getpid(), "after i = barrier.wait()", flush=True)
            if i == cls.N // 2:
                raise RuntimeError
            print("abort worker", os.getpid(), "before barrier.wait()", flush=True)
            barrier.wait()
            print("abort worker", os.getpid(), "after barrier.wait()", flush=True)
            print(
                "abort worker", os.getpid(), "before results1.append(True)", flush=True
            )
            results1.append(True)
            print(
                "abort worker", os.getpid(), "after results1.append(True)", flush=True
            )
        except threading.BrokenBarrierError:
            print(
                "abort worker", os.getpid(), "before results2.append(True)", flush=True
            )
            results2.append(True)
            print(
                "abort worker", os.getpid(), "after results2.append(True)", flush=True
            )
        except RuntimeError:
            print("abort worker", os.getpid(), "before barrier.abort()", flush=True)
            barrier.abort()
            print("abort worker", os.getpid(), "after barrier.abort()", flush=True)

    def test_abort(self):
        """
        Test that an abort will put the barrier in a broken state
        """
        results1 = self.DummyList()
        results2 = self.DummyList()
        self.run_threads(self._test_abort_f, (self.barrier, results1, results2))
        self.assertEqual(len(results1), 0)
        self.assertEqual(len(results2), self.N - 1)
        self.assertTrue(self.barrier.broken)


class WithManagerTestBarrier(_TestBarrier, ManagerMixin, unittest.TestCase):
    start_method = "spawn"


def _cleanup_tests():
    """Cleanup multiprocessing resources when multiprocessing tests
    completed."""

    from test import support

    # cleanup multiprocessing
    print("module cleanup: before process._cleanup()", flush=True)
    process._cleanup()
    print("module cleanup: after process._cleanup()", flush=True)

    # Stop the ForkServer process if it's running
    from multiprocessing import forkserver

    print("module cleanup: before forkserver._forkserver._stop()", flush=True)
    forkserver._forkserver._stop()
    print("module cleanup: after forkserver._forkserver._stop()", flush=True)

    # Stop the ResourceTracker process if it's running
    from multiprocessing import resource_tracker

    print(
        "module cleanup: before resource_tracker._resource_tracker._stop()", flush=True
    )
    resource_tracker._resource_tracker._stop()
    print(
        "module cleanup: after resource_tracker._resource_tracker._stop()", flush=True
    )

    # bpo-37421: Explicitly call _run_finalizers() to remove immediately
    # temporary directories created by multiprocessing.util.get_temp_dir().
    print("module cleanup: before _run_finalizers()", flush=True)
    _run_finalizers()
    print("module cleanup: after _run_finalizers()", flush=True)
    print("module cleanup: before support.gc_collect()", flush=True)
    support.gc_collect()
    print("module cleanup: after support.gc_collect()", flush=True)

    print("module cleanup: before support.reap_children()", flush=True)
    support.reap_children()
    print("module cleanup: after support.reap_children()", flush=True)


def tearDownModule():
    _cleanup_tests()


class ProgressResult(unittest.TextTestResult):
    def startTest(self, test):
        print("METHOD begin " + test.id(), flush=True)
        super().startTest(test)

    def stopTest(self, test):
        print("METHOD end " + test.id(), flush=True)
        super().stopTest(test)


if __name__ == "__main__":
    unittest.main(
        verbosity=2,
        testRunner=unittest.TextTestRunner(resultclass=ProgressResult, verbosity=2),
    )
