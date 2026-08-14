import select
import signal
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


# fileno() runs while the sequence is being read, and it can mutate the very
# list it was handed.
mutable_pair, other_end = socket.socketpair()


class MutatesTheList:
    def __init__(self, elements, fd):
        self.elements = elements
        self.fd = fd

    def fileno(self):
        self.elements.clear()
        self.elements.append(self)
        return self.fd


elements = []
elements.extend([MutatesTheList(elements, mutable_pair.fileno())] * 40)
assert select.select(elements, [], [], 0) == ([], [], [])
del mutable_pair, other_end

# poll() waits with signal handlers able to run, and a handler may register on
# the same poll object.
if hasattr(select, "poll") and hasattr(signal, "setitimer"):
    poller = select.poll()
    idle, idle_peer = socket.socketpair()
    poller.register(idle.fileno(), select.POLLIN)
    handled = []

    def register_from_handler(signum, frame):
        poller.register(idle_peer.fileno(), select.POLLIN)
        handled.append(signum)

    previous = signal.signal(signal.SIGALRM, register_from_handler)
    try:
        signal.setitimer(signal.ITIMER_REAL, 0.05)
        try:
            poller.poll(1000)
        except InterruptedError:
            pass
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)
    assert handled == [signal.SIGALRM], handled
    del idle, idle_peer
