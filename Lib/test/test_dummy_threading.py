from test import support
import unittest
import dummy_threading as _threading
import time

class DummyThreadingTestCase(unittest.TestCase):

    class TestThread(_threading.Thread):

        def run(self):
            global running
            global sema
            global mutex
            # Uncomment if testing another module, such as the real 'threading'
            # module.
            #delay = random.random() * 2
            delay = 0
            if support.verbose:
                print('task', self.name, 'will run for', delay, 'sec')
            sema.acquire()
            mutex.acquire()
            running += 1
            if support.verbose:
                print(running, 'tasks are running')
            mutex.release()
            time.sleep(delay)
            if support.verbose:
                print('task', self.name, 'done')
            mutex.acquire()
            running -= 1
            if support.verbose:
                print(self.name, 'is finished.', running, 'tasks are running')
            mutex.release()
            sema.release()

    def setUp(self):
        self.numtasks = 10
        global sema
        sema = _threading.BoundedSemaphore(value=3)
        global mutex
        mutex = _threading.RLock()
        global running
        running = 0
        self.threads = []

    def test_tasks(self):
        for i in range(self.numtasks):
            t = self.TestThread(name="<thread %d>"%i)
            self.threads.append(t)
            t.start()

        if support.verbose:
            print('waiting for all tasks to complete')
        for t in self.threads:
            t.join()
        if support.verbose:
            print('all tasks done')

if __name__ == '__main__':
    unittest.main()
