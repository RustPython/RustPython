from contextlib import contextmanager

@contextmanager
def try_catch(*args, **kwargs):
    try:
        yield
    except RuntimeError:
        pass

# ---

with try_catch():
    raise RuntimeError()
