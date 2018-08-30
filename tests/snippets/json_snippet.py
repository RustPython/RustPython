import json

def round_trip_test(obj):
    # serde_json and Python's json module produce slightly differently spaced
    # output; direct string comparison can't pass on both so we use this as a
    # proxy
    assert obj == json.loads(json.dumps(obj))

assert '"string"' == json.dumps("string")
assert "1" == json.dumps(1)
assert "1.0" == json.dumps(1.0)
assert "true" == json.dumps(True)
assert "false" == json.dumps(False)
assert 'null' == json.dumps(None)

assert '[]' == json.dumps([])
assert '[1]' == json.dumps([1])
assert '[[1]]' == json.dumps([[1]])
round_trip_test([1, "string", 1.0, True])

assert '[]' == json.dumps(())
assert '[1]' == json.dumps((1,))
assert '[[1]]' == json.dumps(((1,),))
# tuples don't round-trip through json
assert [1, "string", 1.0, True] == json.loads(json.dumps((1, "string", 1.0, True)))

assert '{}' == json.dumps({})
# TODO: uncomment once dict comparison is implemented
# round_trip_test({'a': 'b'})

assert 1 == json.loads("1")
assert -1 == json.loads("-1")
assert 1.0 == json.loads("1.0")
# TODO: uncomment once negative floats are implemented
# assert -1.0 == json.loads("-1.0")
assert "str" == json.loads('"str"')
# TODO: Use "is" once implemented
assert True == json.loads('true')
assert False == json.loads('false')
# TODO: uncomment once None comparison is implemented
# assert None == json.loads('null')
assert [] == json.loads('[]')
assert ['a'] == json.loads('["a"]')
assert [['a'], 'b'] == json.loads('[["a"], "b"]')
