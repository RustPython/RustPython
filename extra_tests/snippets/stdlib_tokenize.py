import io
import tokenize

# Pending f-string parts must use the correct source line after CRLF lines.
source = 'pass\r\npass\r\nf"a}}b"\r\n'
tokens = list(tokenize.generate_tokens(io.StringIO(source).readline))
parts = [tok for tok in tokens if tok.type == tokenize.FSTRING_MIDDLE]
assert parts
assert all(tok.line == 'f"a}}b"\r\n' for tok in parts)

# Each doubled brace ends a middle token, with positions in source characters.
for prefix, middle_type in (
    ("f", tokenize.FSTRING_MIDDLE),
    ("t", tokenize.TSTRING_MIDDLE),
):
    for body, expected in (
        (
            "a{{b}}c",
            [("a{", (1, 2), (1, 4)), ("b}", (1, 5), (1, 7)), ("c", (1, 8), (1, 9))],
        ),
        (
            "é{{a}}b",
            [("é{", (1, 2), (1, 4)), ("a}", (1, 5), (1, 7)), ("b", (1, 8), (1, 9))],
        ),
        (
            "x{{{{y",
            [("x{", (1, 2), (1, 4)), ("{", (1, 5), (1, 6)), ("y", (1, 7), (1, 8))],
        ),
        (
            "x}}}}y",
            [("x}", (1, 2), (1, 4)), ("}", (1, 5), (1, 6)), ("y", (1, 7), (1, 8))],
        ),
    ):
        source = prefix + '"' + body + '"'
        tokens = tokenize.generate_tokens(io.StringIO(source).readline)
        actual = [(t.string, t.start, t.end) for t in tokens if t.type == middle_type]
        assert actual == expected, (source, actual)

    source = prefix + '"""a\n{{b}}c"""'
    tokens = tokenize.generate_tokens(io.StringIO(source).readline)
    actual = [(t.string, t.start, t.end) for t in tokens if t.type == middle_type]
    assert actual == [
        ("a\n{", (1, 4), (2, 1)),
        ("b}", (2, 2), (2, 4)),
        ("c", (2, 5), (2, 6)),
    ], actual
