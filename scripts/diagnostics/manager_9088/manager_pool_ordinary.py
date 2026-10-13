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


def sqr(x, wait=0.0, event=None):
    if event is None:
        time.sleep(wait)
    else:
        event.wait(wait)
    return x * x


def mul(x, y):
    return x * y


class _TestPool(BaseTestCase):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        print("Pool: before cls.pool = cls.Pool(4)", flush=True)
        cls.pool = cls.Pool(4)
        print("Pool: after cls.pool = cls.Pool(4)", flush=True)

    @classmethod
    def tearDownClass(cls):
        print("Pool: before cls.pool.terminate()", flush=True)
        cls.pool.terminate()
        print("Pool: after cls.pool.terminate()", flush=True)
        print("Pool: before cls.pool.join()", flush=True)
        cls.pool.join()
        print("Pool: after cls.pool.join()", flush=True)
        cls.pool = None
        super().tearDownClass()

    def test_apply(self):
        papply = self.pool.apply
        self.assertEqual(papply(sqr, (5,)), sqr(5))
        self.assertEqual(papply(sqr, (), {"x": 3}), sqr(x=3))

    def test_map(self):
        pmap = self.pool.map
        self.assertEqual(pmap(sqr, list(range(10))), list(map(sqr, list(range(10)))))
        self.assertEqual(
            pmap(sqr, list(range(100)), chunksize=20), list(map(sqr, list(range(100))))
        )

    def test_starmap(self):
        psmap = self.pool.starmap
        tuples = list(zip(range(10), range(9, -1, -1)))
        self.assertEqual(psmap(mul, tuples), list(itertools.starmap(mul, tuples)))
        tuples = list(zip(range(100), range(99, -1, -1)))
        self.assertEqual(
            psmap(mul, tuples, chunksize=20), list(itertools.starmap(mul, tuples))
        )

    def test_starmap_async(self):
        tuples = list(zip(range(100), range(99, -1, -1)))
        self.assertEqual(
            self.pool.starmap_async(mul, tuples).get(),
            list(itertools.starmap(mul, tuples)),
        )

    def test_map_async(self):
        self.assertEqual(
            self.pool.map_async(sqr, list(range(10))).get(),
            list(map(sqr, list(range(10)))),
        )

    def test_empty_iterable(self):
        # See Issue 12157
        p = self.Pool(1)

        self.assertEqual(p.map(sqr, []), [])
        self.assertEqual(list(p.imap(sqr, [])), [])
        self.assertEqual(list(p.imap_unordered(sqr, [])), [])
        self.assertEqual(p.map_async(sqr, []).get(), [])

        p.close()
        p.join()


class WithManagerTestPool(_TestPool, ManagerMixin, unittest.TestCase):
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
