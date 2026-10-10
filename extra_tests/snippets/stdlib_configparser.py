import configparser

for allow_no_value in (False, True):
    for data in (
        "foo bar=baz",
        "foo   bar=baz",
        "foo=bar=baz",
        "foo = bar=baz",
        "foo\t \t=\t \tbar=baz",
    ):
        parser = configparser.RawConfigParser(
            delimiters=(" ", "="), allow_no_value=allow_no_value
        )
        parser.read_string(f"[all]\n{data}")
        assert parser.options("all") == ["foo"]
        assert parser.get("all", "foo") == "bar=baz"
