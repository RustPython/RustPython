"""Interactive input read-ahead belongs to the terminal, not its caller thread."""

import os
import sys

if os.name == "posix" and hasattr(os, "openpty"):
    import errno
    import select
    import subprocess
    import time

    def read_until(fd, marker, buffers, closed):
        deadline = time.monotonic() + 20
        while marker not in buffers[fd]:
            remaining = deadline - time.monotonic()
            assert remaining > 0, (marker, buffers)
            # Drain both terminals, even while waiting for the other one's
            # prompt. On BSD, unread output can block tcsetattr(TCSADRAIN)
            # when the first input restores its terminal mode.
            ready, _, _ = select.select(
                [stream for stream in buffers if stream not in closed],
                [],
                [],
                remaining,
            )
            assert ready, (marker, buffers)
            for stream in ready:
                try:
                    chunk = os.read(stream, 4096)
                except OSError as error:
                    # Linux PTY masters report EIO after their last slave closes.
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                if chunk:
                    buffers[stream].extend(chunk)
                else:
                    closed.add(stream)
            assert fd not in closed or marker in buffers[fd], (marker, buffers)
        end = buffers[fd].index(marker) + len(marker)
        del buffers[fd][:end]

    def exercise(code, switch_terminal=False, output_terminal=False):
        master, slave = os.openpty()
        other_master, other_slave = os.openpty()
        env = dict(os.environ, TERM="xterm")
        # Do not use pty.fork()/setsid(): those deliberately select RustPython's
        # stdio fallback instead of the interactive line editor.
        process = subprocess.Popen(
            [sys.executable, "-c", code, str(other_slave)],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            pass_fds=(other_slave,),
            env=env,
        )
        os.close(slave)
        os.close(other_slave)
        buffers = {master: bytearray(), other_master: bytearray()}
        closed = set()
        try:
            read_until(master, b"FIRST>", buffers, closed)
            os.write(master, b"one\ntwo\n")
            if switch_terminal:
                read_until(other_master, b"SECOND>", buffers, closed)
                os.write(other_master, b"fresh\n")
                read_until(other_master, b"RESULT=fresh", buffers, closed)
            elif output_terminal:
                read_until(other_master, b"RESULT=two", buffers, closed)
            else:
                read_until(master, b"RESULT=two", buffers, closed)
            assert process.wait(timeout=20) == 0
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(other_master)

    exercise(
        """
import threading
values = []
def first():
    values.append(input('FIRST>'))
t = threading.Thread(target=first)
t.start()
t.join()
assert values == ['one'], values
print('RESULT=' + input('SECOND>'), flush=True)
"""
    )
    exercise(
        """
import os, sys
assert input('FIRST>') == 'one'
fd = int(sys.argv[1])
os.dup2(fd, 0)
os.dup2(fd, 1)
os.dup2(fd, 2)
os.close(fd)
print('RESULT=' + input('SECOND>'), flush=True)
""",
        switch_terminal=True,
    )

    # Presentation changes must not discard bytes belonging to unchanged stdin.
    exercise(
        """
import os
assert input('FIRST>') == 'one'
os.environ['TERM'] = 'vt100'
print('RESULT=' + input('SECOND>'), flush=True)
"""
    )
    exercise(
        """
import os, sys
assert input('FIRST>') == 'one'
fd = int(sys.argv[1])
os.dup2(fd, 1)
os.dup2(fd, 2)
os.close(fd)
print('RESULT=' + input('SECOND>'), flush=True)
""",
        output_terminal=True,
    )

    # The result audit runs after native reading and may start another input.
    # It must not inherit an active reader borrow or lose the retained next line.
    exercise(
        """
import sys
results = []
def audit(event, args):
    if event == 'builtins.input/result' and not results:
        results.append(args[0])
        results.append(input('SECOND>'))
sys.addaudithook(audit)
assert input('FIRST>') == 'one'
assert results == ['one', 'two'], results
print('RESULT=' + results[1], flush=True)
"""
    )
