import select
import socket
import sys

from testutils import assert_raises

TOO_MANY_SELECT_FDS = 4096


class Nope:
    pass


class Almost:
    def fileno(self):
        return "fileno"


assert_raises(TypeError, select.select, 1, 2, 3)
assert_raises(TypeError, select.select, [Nope()], [], [])
assert_raises(TypeError, select.select, [Almost()], [], [])
assert_raises(TypeError, select.select, [], [], [], "not a number")
assert_raises(ValueError, select.select, [], [], [], -1)

if sys.platform in ["win32", "cygwin"]:
    assert_raises(OSError, select.select, [0], [], [])

recvr = socket.socket()

recvr.bind(("127.0.0.1", 9988))

recvr.listen()

recvr.settimeout(10.0)

sendr = socket.socket()

sendr.connect(("127.0.0.1", 9988))
sendr.send(b"aaaa")

rres, wres, xres = select.select([recvr], [sendr], [])

if "win" not in sys.platform:
    assert recvr in rres

assert sendr in wres

# Too many descriptors for select.select()
if sys.platform != "win32":
    import resource

    soft_max_fds, hard_max_fds = resource.getrlimit(resource.RLIMIT_NOFILE)
    if soft_max_fds != resource.RLIM_INFINITY:
        # 100 additional fds should be enough for interpreter needs
        need_fds = TOO_MANY_SELECT_FDS + 100

        soft_max_fds = max(soft_max_fds, need_fds)
        if hard_max_fds != resource.RLIM_INFINITY:
            assert hard_max_fds >= soft_max_fds, (
                "Not enough file descriptors for this test"
            )
        resource.setrlimit(resource.RLIMIT_NOFILE, (soft_max_fds, hard_max_fds))
sockets = [s for _ in range(TOO_MANY_SELECT_FDS // 2) for s in socket.socketpair()]
assert_raises(ValueError, select.select, sockets, [], [], 0)
if sys.platform != "win32":
    # Try to overflow descriptor bit mask on *nix with a single item
    max_fd = -1
    max_fd_sock = None
    sockets.reverse()
    for sock in sockets:
        if sock.fileno() > max_fd:
            max_fd = sock.fileno()
            max_fd_sock = sock
    assert_raises(ValueError, select.select, [max_fd_sock], [], [], 0)
del sockets
a, b = socket.socketpair()
# CPython disallows this on *nix systems too.
assert_raises(ValueError, select.select, [a] * TOO_MANY_SELECT_FDS, [], [], 0)
del a, b
