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
assert -1.0 == json.loads("-1.0")
assert "str" == json.loads('"str"')
assert True is json.loads('true')
assert False is json.loads('false')
assert None is json.loads('null')
assert [] == json.loads('[]')
assert ['a'] == json.loads('["a"]')
assert [['a'], 'b'] == json.loads('[["a"], "b"]')

class String(str): pass

assert "string" == json.loads(String('"string"'))
assert '"string"' == json.dumps(String("string"))

# TODO: Uncomment and test once int/float construction is supported
# class Int(int): pass
# class Float(float): pass

# TODO: Uncomment and test once sequence/dict subclasses are supported by
# json.dumps
# class List(list): pass
# class Tuple(tuple): pass
# class Dict(dict): pass
