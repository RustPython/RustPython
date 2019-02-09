# __abs__

assert complex(3, 4).__abs__() == 5
assert complex(3, -4).__abs__() == 5
assert complex(1.5, 2.5).__abs__() == 2.9154759474226504

# __eq__

assert complex(1, -1).__eq__(complex(1, -1))
assert complex(1, 0).__eq__(1)
assert not complex(1, 1).__eq__(1)
assert complex(1, 0).__eq__(1.0)
assert not complex(1, 1).__eq__(1.0)
assert not complex(1, 0).__eq__(1.5)
assert complex(1, 0).__eq__(True)
assert not complex(1, 2).__eq__(complex(1, 1))
assert complex(1, 2).__eq__('foo') == NotImplemented

# __neg__

assert complex(1, -1).__neg__() == complex(-1, 1)
assert complex(0, 0).__neg__() == complex(0, 0)
