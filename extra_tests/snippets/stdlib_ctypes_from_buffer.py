"""from_buffer char-array .value writes must mutate the exporter."""

import ctypes

buf = (ctypes.c_char * 20)()
view = (ctypes.c_char * 20).from_buffer(buf)
view.value = b"hello"
assert buf.value == b"hello"
view.value *= 2
assert buf.value == b"hellohello"
assert view.value == b"hellohello"
