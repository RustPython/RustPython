import subprocess
import time
import sys
import signal

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
	test_output = b"test\n"
else:
	# windows
	test_output = b"test\r\n"

assert p.stdout.read() == test_output

p = subprocess.Popen(["sleep", "2"])
p.terminate()
p.wait()
assert p.returncode == -signal.SIGTERM

p = subprocess.Popen(["sleep", "2"])
p.kill()
p.wait()
assert p.returncode == -signal.SIGKILL

p = subprocess.Popen(["echo", "test"], stdout=subprocess.PIPE)
(stdout, stderr) = p.communicate()
assert stdout == test_output
