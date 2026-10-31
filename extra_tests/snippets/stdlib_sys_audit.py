import sys

seen = []
sys.addaudithook(lambda event, args: seen.append((event, args)))

code = compile("x = 1", "<test>", "exec")
exec(code, {})
assert ("exec", (code,)) in seen

# Built-in events go to the registered hooks, not through a reassigned `sys.audit`.
seen.clear()
replaced = []
sys.audit = lambda *args: replaced.append(args)
exec(code, {})
assert ("exec", (code,)) in seen
assert replaced == []

# Hooks are per interpreter, so they also fire in other threads.
import threading

seen.clear()
worker = threading.Thread(target=exec, args=(code, {}))
worker.start()
worker.join()
assert ("exec", (code,)) in seen

# Hooks added concurrently are each notified by every hook added after them.
N = 8
counts = [0] * N
lock = threading.Lock()


def make_hook(i):
    def hook(event, args):
        if event == "sys.addaudithook":
            with lock:
                counts[i] += 1

    return hook


barrier = threading.Barrier(N)


def add_hook(i):
    barrier.wait()
    sys.addaudithook(make_hook(i))


workers = [threading.Thread(target=add_hook, args=(i,)) for i in range(N)]
for w in workers:
    w.start()
for w in workers:
    w.join()
assert sorted(counts) == list(range(N)), counts
