# __abs__

assert abs(complex(3, 4)) == 5
assert abs(complex(3, -4)) == 5
assert abs(complex(1.5, 2.5)) == 2.9154759474226504

# __eq__

assert complex(1, -1) == complex(1, -1)
assert complex(1, 0) == 1
assert not complex(1, 1) == 1
assert complex(1, 0) == 1.0
assert not complex(1, 1) == 1.0
assert not complex(1, 0) == 1.5
assert bool(complex(1, 0))
assert not complex(1, 2) == complex(1, 1)
# Currently broken - see issue #419
# assert complex(1, 2) != 'foo'
assert complex(1, 2).__eq__('foo') == NotImplemented

# __neg__

assert -complex(1, -1) == complex(-1, 1)
assert -complex(0, 0) == complex(0, 0)
