"""Thread+process Condition notify must wake every waiter."""

import multiprocessing
import threading


def wait_on_condition(cond, started, woken):
    cond.acquire()
    started.release()
    cond.wait()
    woken.release()
    cond.release()


def join_workers(workers):
    for worker in workers:
        worker.join(timeout=5)
        assert not worker.is_alive()


def wait_sem(sem, msg):
    assert sem.acquire(timeout=5), msg


def notify_all_six(ctx):
    cond = ctx.Condition()
    started = ctx.Semaphore(0)
    woken = ctx.Semaphore(0)

    workers = []
    for _ in range(3):
        proc = ctx.Process(target=wait_on_condition, args=(cond, started, woken))
        proc.daemon = True
        proc.start()
        workers.append(proc)

        thread = threading.Thread(target=wait_on_condition, args=(cond, started, woken))
        thread.daemon = True
        thread.start()
        workers.append(thread)

    for i in range(6):
        wait_sem(started, f"waiter {i} did not enter Condition.wait()")

    cond.acquire()
    cond.notify_all()
    cond.release()

    for i in range(6):
        wait_sem(woken, f"waiter {i} did not wake")
    join_workers(workers)


def notify_one_then_the_other(ctx):
    cond = ctx.Condition()
    started = ctx.Semaphore(0)
    woken = ctx.Semaphore(0)

    proc = ctx.Process(target=wait_on_condition, args=(cond, started, woken))
    proc.daemon = True
    proc.start()
    thread = threading.Thread(target=wait_on_condition, args=(cond, started, woken))
    thread.daemon = True
    thread.start()

    wait_sem(started, "process did not enter Condition.wait()")
    wait_sem(started, "thread did not enter Condition.wait()")

    cond.acquire()
    cond.notify()
    cond.release()
    wait_sem(woken, "first waiter did not wake")

    cond.acquire()
    cond.notify()
    cond.release()
    wait_sem(woken, "second waiter did not wake")
    join_workers([proc, thread])


if __name__ == "__main__":
    multiprocessing.freeze_support()
    ctx = multiprocessing.get_context("spawn")
    # Enough repeats to surface a lost-wakeup race in CI snippets.
    for _ in range(20):
        notify_all_six(ctx)
        notify_one_then_the_other(ctx)
