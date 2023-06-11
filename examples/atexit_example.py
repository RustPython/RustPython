import atexit
import sys

def myexit():
    sys.exit(2)

atexit.register(myexit)
