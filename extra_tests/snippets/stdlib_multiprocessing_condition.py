"""Thread+process Condition.notify_all must wake every waiter."""

import multiprocessing
import threading


def wait_on_condition(cond, started, woken):
    cond.acquire()
    started.release()
    cond.wait()
    woken.release()
    cond.release()


def main():
    ctx = multiprocessing.get_context("spawn")
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

    for worker in workers:
        worker.join(timeout=5)
        assert not worker.is_alive()


if __name__ == "__main__":
    multiprocessing.freeze_support()
    for _ in range(5):
        main()
