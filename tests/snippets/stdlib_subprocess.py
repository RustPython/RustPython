import subprocess
import time
import sys

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

p = subprocess.Popen(["echo", "test"], stdout=subprocess.PIPE)
p.wait()

if "win" not in sys.platform:
	# unix
	assert p.stdout.read() == b"test\n"
else:
	# windows
	assert p.stdout.read() == b"test\r\n"
