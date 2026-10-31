# Adapted from python/pyperformance 1.14.0 (json_dumps).

import json


empty = ({}, 2000)
simple_data = {
    "key1": 0,
    "key2": True,
    "key3": "value",
    "key4": "foo",
    "key5": "string",
}
simple = (simple_data, 1000)
nested_data = {
    "key1": 0,
    "key2": simple_data,
    "key3": "value",
    "key4": simple_data,
    "key5": simple_data,
    "key": "ąćż",
}
nested = (nested_data, 1000)
huge = ([nested_data] * 1000, 1)

for obj, count in (empty, simple, nested, huge):
    for _ in range(count):
        json.dumps(obj)
