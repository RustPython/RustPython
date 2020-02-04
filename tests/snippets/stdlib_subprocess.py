import subprocess
import time
import sys
import signal

from testutils import assert_raises

p = subprocess.Popen(["echo", "test"])

time.sleep(0.1)

assert p.returncode is None

assert p.poll() == 0
assert p.returncode == 0

p = subprocess.Popen(["sleep", "2"])

assert p.poll() is None

with assert_raises(subprocess.TimeoutExpired):
	assert p.wait(1)

p.wait()

assert p.returncode == 0

p = subprocess.Popen(["echo", "test"], stdout=subprocess.PIPE)
p.wait()

is_unix = "win" not in sys.platform or "darwin" in sys.platform

assert p.stdout.read().strip() == b"test"

p = subprocess.Popen(["sleep", "2"])
p.terminate()
p.wait()
if is_unix:
	assert p.returncode == -signal.SIGTERM
else:
	assert p.returncode == 1

p = subprocess.Popen(["sleep", "2"])
p.kill()
p.wait()
if is_unix:
	assert p.returncode == -signal.SIGKILL
else:
	assert p.returncode == 1

p = subprocess.Popen(["echo", "test"], stdout=subprocess.PIPE)
(stdout, stderr) = p.communicate()
assert stdout.strip() == b"test"

p = subprocess.Popen(["sleep", "5"], stdout=subprocess.PIPE)
with assert_raises(subprocess.TimeoutExpired):
	p.communicate(timeout=1)
