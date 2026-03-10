"""Minimal reproduction of multiprocessing Manager + fork failure."""

import multiprocessing
import os
import sys
import time
import traceback

import pytest

pytestmark = pytest.mark.skipif(not hasattr(os, "fork"), reason="requires os.fork")


def test_basic_manager():
    """Test Manager without fork - does it work at all?"""
    print("=== Test 1: Basic Manager (no fork) ===")
    ctx = multiprocessing.get_context("fork")
    manager = ctx.Manager()
    try:
        ev = manager.Event()
        print(f"  Event created: {ev}")
        ev.set()
        print(f"  Event set, is_set={ev.is_set()}")
        assert ev.is_set()
        print("  PASS")
    finally:
        manager.shutdown()


def test_manager_with_process():
    """Test Manager shared between parent and child process."""
    print("\n=== Test 2: Manager with forked child ===")
    ctx = multiprocessing.get_context("fork")
    manager = ctx.Manager()
    try:
        result = manager.Value("i", 0)
        ev = manager.Event()

        def child_fn():
            try:
                ev.set()
                result.value = 42
            except Exception as e:
                print(f"  CHILD ERROR: {e}", file=sys.stderr)
                traceback.print_exc()
                sys.exit(1)

        print(f"  Starting child process...")
        process = ctx.Process(target=child_fn)
        process.start()
        print(f"  Waiting for child (pid={process.pid})...")
        process.join(timeout=10)

        if process.exitcode != 0:
            print(f"  FAIL: child exited with code {process.exitcode}")
            return False

        print(f"  Child done. result={result.value}, event={ev.is_set()}")
        assert result.value == 42
        assert ev.is_set()
        print("  PASS")
        return True
    finally:
        manager.shutdown()


def test_manager_server_alive_after_fork():
    """Test that Manager server survives after forking a child."""
    print("\n=== Test 3: Manager server alive after fork ===")
    ctx = multiprocessing.get_context("fork")
    manager = ctx.Manager()
    try:
        ev = manager.Event()

        # Fork a child that does nothing with the manager
        pid = os.fork()
        if pid == 0:
            # Child - exit immediately
            os._exit(0)

        # Parent - wait for child
        os.waitpid(pid, 0)

        # Now try to use the manager in the parent
        print(f"  After fork, trying to use Manager in parent...")
        ev.set()
        print(f"  ev.is_set() = {ev.is_set()}")
        assert ev.is_set()
        print("  PASS")
        return True
    finally:
        manager.shutdown()


def test_manager_server_alive_after_fork_with_child_usage():
    """Test that Manager server survives when child also uses it."""
    print("\n=== Test 4: Manager server alive after fork + child usage ===")
    ctx = multiprocessing.get_context("fork")
    manager = ctx.Manager()
    try:
        child_ev = manager.Event()
        parent_ev = manager.Event()

        def child_fn():
            try:
                child_ev.set()
            except Exception as e:
                print(f"  CHILD ERROR: {e}", file=sys.stderr)
                traceback.print_exc()
                sys.exit(1)

        process = ctx.Process(target=child_fn)
        process.start()
        process.join(timeout=10)

        if process.exitcode != 0:
            print(f"  FAIL: child exited with code {process.exitcode}")
            return False

        # Now use manager in parent AFTER child is done
        print(f"  Child done. Trying parent usage...")
        parent_ev.set()
        print(f"  child_ev={child_ev.is_set()}, parent_ev={parent_ev.is_set()}")
        assert child_ev.is_set()
        assert parent_ev.is_set()
        print("  PASS")
        return True
    finally:
        manager.shutdown()


if __name__ == "__main__":
    test_basic_manager()

    passed = 0
    total = 10
    for i in range(total):
        print(f"\n--- Iteration {i + 1}/{total} ---")
        ok = True
        ok = ok and test_manager_with_process()
        ok = ok and test_manager_server_alive_after_fork()
        ok = ok and test_manager_server_alive_after_fork_with_child_usage()
        if ok:
            passed += 1
        else:
            print(f"  FAILED on iteration {i + 1}")

    print(f"\n=== Results: {passed}/{total} passed ===")
    sys.exit(0 if passed == total else 1)
