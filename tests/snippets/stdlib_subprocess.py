import subprocess
import time

from testutils import assertRaises

p = subprocess.Popen(["echo", "test"])

time.sleep(0.1)

assert p.returncode is None

assert p.poll() == 0
assert p.returncode == 0

p = subprocess.Popen(["sleep", "2"])

assert p.poll() is None

with assertRaises(subprocess.TimeoutExpired):
	assert p.wait(1)

p.wait()

assert p.returncode == 0
