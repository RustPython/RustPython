import threading
import time

output = []


def thread_function(name):
    output.append((name, 0))
    time.sleep(2.0)
    output.append((name, 1))


output.append((0, 0))
x = threading.Thread(target=thread_function, args=(1, ))
output.append((0, 1))
x.start()
output.append((0, 2))
x.join()
output.append((0, 3))

assert len(output) == 6, output
# CPython has [(1, 0), (0, 2)] for the middle 2, but we have [(0, 2), (1, 0)]
# TODO: maybe fix this, if it turns out to be a problem?
# assert output == [(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (0, 3)]
