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
