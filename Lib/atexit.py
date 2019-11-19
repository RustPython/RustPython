# Dummy implementation of atexit


def register(func, *args, **kwargs):
    return func


def unregister(func):
    pass
