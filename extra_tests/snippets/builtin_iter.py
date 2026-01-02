import queue
import threading


def make_iterator():
    holder = {}

    class Evil:
        def __getitem__(self, index):
            if index == 0:
                return 0
            raise IndexError

        def __len__(self):
            return holder["it"].__length_hint__()

    obj = Evil()
    holder["it"] = iter(obj)
    return holder["it"]


it = make_iterator()
q = queue.Queue()


def run():
    try:
        it.__length_hint__()
    except Exception as exc:  # noqa: BLE001
        q.put(exc)
    else:
        q.put(None)


t = threading.Thread(target=run, daemon=True)
t.start()
t.join(1)

assert not t.is_alive(), "iterator.__length_hint__ deadlocked"
err = q.get_nowait()
assert isinstance(err, RecursionError)
