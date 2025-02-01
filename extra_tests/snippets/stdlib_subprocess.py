import subprocess
import time
import sys
import signal

from testutils import assert_raises

is_unix = not sys.platform.startswith("win")
if is_unix:
    def echo(text):
        return ["echo", text]
    def sleep(secs):
        return ["sleep", str(secs)]
else:
    def echo(text):
        return ["cmd", "/C", f"echo {text}"]
    def sleep(secs):
        # TODO: make work in a non-unixy environment (something with timeout.exe?)
        return ["sleep", str(secs)]

p = subprocess.Popen(echo("test"))

time.sleep(0.1)

assert p.returncode is None

assert p.poll() == 0
assert p.returncode == 0

p = subprocess.Popen(echo("test"), stdout=subprocess.PIPE)
p.wait()


assert p.stdout.read().strip() == b"test"
