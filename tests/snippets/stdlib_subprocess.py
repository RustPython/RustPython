import subprocess
import time

p = subprocess.Popen(["echo", "test"])

time.sleep(0.1)

assert p.returncode is None

assert p.poll() == 0
assert p.returncode == 0
