class VerifyBuilder:
    def __init__(self):
        self._data = {
            "mode": "none",
            "runners": 1,
            "threshold": 1,
            "checks": [],
        }

    def mode(self, value):
        self._data["mode"] = value
        return self

    def runners(self, value):
        self._data["runners"] = int(value)
        return self

    def threshold(self, value):
        self._data["threshold"] = int(value)
        return self

    def check(self, value):
        self._data["checks"].append(value)
        return self

    def build(self):
        return dict(self._data)


class Verify:
    @staticmethod
    def builder():
        return VerifyBuilder()

    @staticmethod
    def json_schema_valid(schema):
        return {"kind": "json_schema_valid", "schema": schema}

    @staticmethod
    def structured_match(fields):
        return {"kind": "structured_match", "fields": list(fields)}

    @staticmethod
    def majority_vote(field):
        return {"kind": "majority_vote", "field": field}
