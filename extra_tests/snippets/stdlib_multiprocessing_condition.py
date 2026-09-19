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

    for _ in range(6):
        started.acquire()

    cond.acquire()
    cond.notify_all()
    cond.release()

    for _ in range(6):
        woken.acquire()
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

    started.acquire()
    started.acquire()

    cond.acquire()
    cond.notify()
    cond.release()
    woken.acquire()

    cond.acquire()
    cond.notify()
    cond.release()
    woken.acquire()
    join_workers([proc, thread])


if __name__ == "__main__":
    multiprocessing.freeze_support()
    ctx = multiprocessing.get_context("spawn")
    # Enough repeats to surface a lost-wakeup race in CI snippets.
    for _ in range(20):
        notify_all_six(ctx)
        notify_one_then_the_other(ctx)
