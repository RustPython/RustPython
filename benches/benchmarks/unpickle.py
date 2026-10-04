# Adapted from python/pyperformance 1.14.0 (unpickle_pure_python).

import datetime
import pickle


profile = {
    "age": 18,
    "birthday": datetime.date(1980, 5, 7),
    "country": "BR",
    "encrypted_id": "G9urXXAJwjE",
    "flags": 412317970704,
    "friend_count": 0,
    "locale_preference": "pt_BR",
    "tags": ["a", "b", "c", "d", "e", "f", "g"],
    "username": "collinwinter",
}
record = ([*range(20)], 60)
group = [dict(profile, id=index) for index in range(3)]
list_data = [[list(range(10)), list(range(10))] for _ in range(10)]
protocol = pickle.HIGHEST_PROTOCOL
dumps = pickle._dumps
loads = pickle._loads

for obj, count in (
    (profile, 20),
    (record, 20),
    (group, 20),
    (list_data, 10),
):
    encoded = dumps(obj, protocol)
    for _ in range(count):
        assert loads(encoded) == obj
