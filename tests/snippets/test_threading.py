import threading
import time

output = []


def thread_function(name):
    output.append("Thread %s: starting" % name)
    time.sleep(2.0)
    output.append("Thread %s: finishing" % name)


output.append("Main    : before creating thread")
x = threading.Thread(target=thread_function, args=(1, ))
output.append("Main    : before running thread")
x.start()
output.append("Main    : wait for the thread to finish")
x.join()
output.append("Main    : all done")

assert len(output) == 6, output
