"""Use the migrated urllib.parse normally, or execute the exact candidate file.

The optional path loads a real source module; it never adapts its contents.
"""

import sys

if len(sys.argv) > 1:
    import urllib

    source_path = sys.argv[1]
    parse = type(sys)("urllib.parse")
    parse.__file__ = source_path
    parse.__package__ = "urllib"
    sys.modules["urllib.parse"] = parse
    urllib.parse = parse
    with open(source_path, encoding="utf-8") as source:
        exec(compile(source.read(), source_path, "exec"), parse.__dict__)
else:
    import urllib.parse as parse

import copy
import pickle
from collections import namedtuple


class NamedResult(namedtuple("NamedResult", "first second")):
    __slots__ = ("_keep_empty",)


named = NamedResult(1, 2)
named._keep_empty = True
assert tuple(named) == (1, 2) and named._keep_empty is True
assert not hasattr(named, "__dict__")
print("PASS namedtuple_subclass_slot")

count = 0
for function in (parse.urlsplit, parse.urlparse, parse.urldefrag):
    for raw_url in (
        "https://example.test/path",
        "https://example.test/path?",
        "https://example.test/path#",
        "https://example.test/path?#",
    ):
        for url in (raw_url, raw_url.encode()):
            for missing_as_none in (False, True):
                result = function(url, missing_as_none=missing_as_none)
                assert result._keep_empty is missing_as_none
                assert not hasattr(result, "__dict__")
                assert isinstance(result, tuple)
                if missing_as_none:
                    assert result.geturl() == url
                converted = result.encode() if isinstance(url, str) else result.decode()
                restored = (
                    converted.decode() if isinstance(url, str) else converted.encode()
                )
                assert restored == result and restored.geturl() == result.geturl()
                assert converted._keep_empty is missing_as_none
                assert restored._keep_empty is missing_as_none
                replaced = result._replace()
                assert replaced == result and replaced.geturl() == result.geturl()
                assert replaced._keep_empty is missing_as_none
                replaced = result.__replace__()
                assert replaced == result and replaced.geturl() == result.geturl()
                assert replaced._keep_empty is missing_as_none
                for clone in (copy.copy(result), copy.deepcopy(result)):
                    assert clone == result and clone.geturl() == result.geturl()
                    assert getattr(clone, "_keep_empty", False) is missing_as_none
                for protocol in range(pickle.HIGHEST_PROTOCOL + 1):
                    clone = pickle.loads(pickle.dumps(result, protocol=protocol))
                    assert clone == result and clone.geturl() == result.geturl()
                    assert getattr(clone, "_keep_empty", False) is missing_as_none
                count += 1
print("PASS urllib_result_roundtrip_copy_pickle", count)
