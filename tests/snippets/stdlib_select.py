from testutils import assert_raises

import select
import sys
import socket


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

is_win = sys.platform.startswith("win")

if is_win:
    assert_raises(OSError, select.select, [0], [], [])

recvr = socket.socket()

recvr.bind(("127.0.0.1", 9988))

recvr.listen()

recvr.settimeout(10.0)

sendr = socket.socket()

sendr.connect(("127.0.0.1", 9988))
sendr.send(b"aaaa")

rres, wres, xres = select.select([recvr], [sendr], [])

if not is_win:
    assert recvr in rres

assert sendr in wres
