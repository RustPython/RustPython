import pvm_host

_seed_prefix = b"N"
_counter = 0
_buffer = b""
_buffer_pos = 0


def _next_block():
    global _counter
    domain = b"random" + _seed_prefix + _counter.to_bytes(8, "little")
    _counter += 1
    return pvm_host.randomness(domain)


def _reset_state():
    global _counter, _buffer, _buffer_pos
    _counter = 0
    _buffer = b""
    _buffer_pos = 0


def _coerce_seed(a):
    if a is None:
        return b"N"
    if isinstance(a, (bytes, bytearray)):
        return b"B" + bytes(a)
    if isinstance(a, str):
        return b"S" + a.encode("utf-8")
    if isinstance(a, int):
        if a == 0:
            data = b"\x00"
        else:
            bits = a.bit_length()
            if a < 0:
                bits += 1
            data = a.to_bytes((bits + 7) // 8, "big", signed=True)
        return b"I" + data
    raise TypeError("seed must be int, bytes, bytearray, str, or None")


def seed(a=None, version=2):
    global _seed_prefix
    _seed_prefix = _coerce_seed(a)
    _reset_state()


def getstate():
    return (1, _seed_prefix, _counter, _buffer, _buffer_pos)


def setstate(state):
    if not isinstance(state, tuple) or len(state) != 5:
        raise ValueError("state must be a 5-item tuple from getstate()")
    version, seed_prefix, counter, buffer_data, buffer_pos = state
    if version != 1:
        raise ValueError("unsupported state version")
    if isinstance(seed_prefix, bytearray):
        seed_prefix = bytes(seed_prefix)
    if not isinstance(seed_prefix, (bytes, bytearray)):
        raise TypeError("seed_prefix must be bytes")
    if not isinstance(counter, int):
        raise TypeError("counter must be int")
    if isinstance(buffer_data, bytearray):
        buffer_data = bytes(buffer_data)
    if not isinstance(buffer_data, (bytes, bytearray)):
        raise TypeError("buffer must be bytes")
    if not isinstance(buffer_pos, int):
        raise TypeError("buffer_pos must be int")
    if buffer_pos < 0 or buffer_pos > len(buffer_data):
        raise ValueError("buffer_pos out of range")
    global _seed_prefix, _counter, _buffer, _buffer_pos
    _seed_prefix = bytes(seed_prefix)
    _counter = counter
    _buffer = bytes(buffer_data)
    _buffer_pos = buffer_pos


def _compact_buffer():
    global _buffer, _buffer_pos
    if _buffer_pos <= 0:
        return
    if _buffer_pos >= len(_buffer):
        _buffer = b""
        _buffer_pos = 0
        return
    _buffer = _buffer[_buffer_pos :]
    _buffer_pos = 0


def _fill(n):
    global _buffer, _buffer_pos
    if _buffer_pos > 0:
        _compact_buffer()
    needed = n - (len(_buffer) - _buffer_pos)
    while needed > 0:
        _buffer += _next_block()
        needed = n - (len(_buffer) - _buffer_pos)


def _randbytes(n):
    global _buffer_pos
    if n <= 0:
        return b""
    _fill(n)
    start = _buffer_pos
    end = start + n
    _buffer_pos = end
    return _buffer[start:end]


def _randbelow(n):
    if n <= 0:
        raise ValueError("n must be > 0")
    k = n.bit_length()
    while True:
        r = getrandbits(k)
        if r < n:
            return r


def getrandbits(k):
    if k < 0:
        raise ValueError("number of bits must be non-negative")
    if k == 0:
        return 0
    nbytes = (k + 7) // 8
    value = int.from_bytes(_randbytes(nbytes), "big")
    return value >> (nbytes * 8 - k)


def random():
    return getrandbits(53) / (1 << 53)


def randbytes(n):
    return _randbytes(n)


def randint(a, b):
    if a > b:
        raise ValueError("empty range for randint()")
    return randrange(a, b + 1, 1)


def randrange(start, stop=None, step=1):
    if stop is None:
        if start > 0:
            return _randbelow(start)
        raise ValueError("empty range for randrange()")
    if step == 0:
        raise ValueError("step must not be zero")
    width = stop - start
    if step == 1:
        if width > 0:
            return start + _randbelow(width)
        raise ValueError("empty range for randrange()")
    if step > 0:
        n = (width + step - 1) // step
    else:
        n = (width + step + 1) // step
    if n <= 0:
        raise ValueError("empty range for randrange()")
    return start + step * _randbelow(n)


def choice(seq):
    if not seq:
        raise IndexError("cannot choose from an empty sequence")
    return seq[_randbelow(len(seq))]


def shuffle(x):
    for i in range(len(x) - 1, 0, -1):
        j = _randbelow(i + 1)
        x[i], x[j] = x[j], x[i]


def uniform(a, b):
    return a + (b - a) * random()


def sample(population, k):
    if k < 0:
        raise ValueError("sample size must be non-negative")
    pool = list(population)
    n = len(pool)
    if k > n:
        raise ValueError("sample larger than population")
    result = []
    for i in range(k):
        j = _randbelow(n - i)
        result.append(pool[j])
        pool[j] = pool[n - i - 1]
    return result
