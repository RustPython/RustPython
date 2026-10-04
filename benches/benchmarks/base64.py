# Adapted from python/pyperformance 1.14.0 (base64).
"""Exercise the base64 module's primary public APIs."""

import base64
import random


random_source = random.Random(12345)
data_tiny = random_source.randbytes(20)
data_small = random_source.randbytes(127)
data_medium = random_source.randbytes(3072)
data_large = random_source.randbytes(9000)
cases = (
    (data_tiny, 45),
    (data_small, 7),
    (data_medium, 1),
    (data_large, 1),
)


def exercise(encode, decode):
    for data, count in cases:
        encoded = encode(data)
        for _ in range(count):
            assert decode(encode(data)) == data
            assert decode(encoded) == data


exercise(base64.b64encode, base64.b64decode)
exercise(base64.urlsafe_b64encode, base64.urlsafe_b64decode)
exercise(base64.b32encode, base64.b32decode)
exercise(base64.b16encode, base64.b16decode)
exercise(base64.a85encode, base64.a85decode)
exercise(base64.b85encode, base64.b85decode)
