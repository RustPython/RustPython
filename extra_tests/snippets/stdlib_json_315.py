"""Empty mapping subclasses must not supply JSON contents through items()."""

import json


def test_empty_mapping_subclass_ignores_items():
    for base in (dict, frozendict):

        class Mapping(base):
            def items(self):
                return [("override", 2)]

        assert json.dumps(Mapping()) == "{}"


if __name__ == "__main__":
    test_empty_mapping_subclass_ignores_items()
