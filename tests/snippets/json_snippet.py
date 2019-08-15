from testutils import assert_raises
import json
from io import StringIO

def round_trip_test(obj):
    # serde_json and Python's json module produce slightly differently spaced
    # output; direct string comparison can't pass on both so we use this as a
    # proxy
    return obj == json.loads(json.dumps(obj))

def json_dump(obj):
    f = StringIO()
    json.dump(obj, f)
    f.seek(0)
    return f.getvalue()

def json_load(obj):
    f = StringIO(obj)
    return json.load(f)

assert '"string"' == json.dumps("string")
assert '"string"' == json_dump("string")

assert "1" == json.dumps(1)
assert "1" == json_dump(1)

assert "1.0" == json.dumps(1.0)
assert "1.0" == json_dump(1.0)

assert "true" == json.dumps(True)
assert "true" == json_dump(True)

assert "false" == json.dumps(False)
assert "false" == json_dump(False)

assert 'null' == json.dumps(None)
assert 'null' == json_dump(None)

assert '[]' == json.dumps([])
assert '[]' == json_dump([])

assert '[1]' == json.dumps([1])
assert '[1]' == json_dump([1])

assert '[[1]]' == json.dumps([[1]])
assert '[[1]]' == json_dump([[1]])

assert round_trip_test([1, "string", 1.0, True])

assert '[]' == json.dumps(())
assert '[]' == json_dump(())

assert '[1]' == json.dumps((1,))
assert '[1]' == json_dump((1,))

assert '[[1]]' == json.dumps(((1,),))
assert '[[1]]' == json_dump(((1,),))
# tuples don't round-trip through json
assert [1, "string", 1.0, True] == json.loads(json.dumps((1, "string", 1.0, True)))

assert '{}' == json.dumps({})
assert '{}' == json_dump({})
assert round_trip_test({'a': 'b'})

# should reject non-str keys in jsons
assert_raises(json.JSONDecodeError, lambda: json.loads('{3: "abc"}'))
assert_raises(json.JSONDecodeError, lambda: json_load('{3: "abc"}'))

# should serialize non-str keys as strings
assert json.dumps({'3': 'abc'}) == json.dumps({3: 'abc'})

assert 1 == json.loads("1")
assert 1 == json_load("1")

assert -1 == json.loads("-1")
assert -1 == json_load("-1")

assert 1.0 == json.loads("1.0")
assert 1.0 == json_load("1.0")

assert -1.0 == json.loads("-1.0")
assert -1.0 == json_load("-1.0")

assert "str" == json.loads('"str"')
assert "str" == json_load('"str"')

assert True is json.loads('true')
assert True is json_load('true')

assert False is json.loads('false')
assert False is json_load('false')

assert None is json.loads('null')
assert None is json_load('null')

assert [] == json.loads('[]')
assert [] == json_load('[]')

assert ['a'] == json.loads('["a"]')
assert ['a'] == json_load('["a"]')

assert [['a'], 'b'] == json.loads('[["a"], "b"]')
assert [['a'], 'b'] == json_load('[["a"], "b"]')

class String(str): pass

assert "string" == json.loads(String('"string"'))
assert "string" == json_load(String('"string"'))

assert '"string"' == json.dumps(String("string"))
assert '"string"' == json_dump(String("string"))

class Int(int): pass
class Float(float): pass

assert '1' == json.dumps(Int(1))
assert '1' == json_dump(Int(1))

assert '0.5' == json.dumps(Float(0.5))
assert '0.5' == json_dump(Float(0.5))

class List(list): pass
class Tuple(tuple): pass
class Dict(dict): pass

assert '[1]' == json.dumps(List([1]))
assert '[1]' == json_dump(List([1]))

assert json.dumps((1, "string", 1.0, True)) == json.dumps(Tuple((1, "string", 1.0, True)))
assert json_dump((1, "string", 1.0, True)) == json_dump(Tuple((1, "string", 1.0, True)))

assert json.dumps({'a': 'b'}) == json.dumps(Dict({'a': 'b'}))
assert json_dump({'a': 'b'}) == json_dump(Dict({'a': 'b'}))

# big ints should not crash VM
# TODO: test for correct output when actual serialization implemented and doesn’t throw
try:
  json.dumps(7**500)
except:
  pass
