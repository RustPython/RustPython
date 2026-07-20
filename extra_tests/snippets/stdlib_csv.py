import csv
import io

from testutils import assert_raises

for row in csv.reader(["one,two,three"]):
    [one, two, three] = row
    assert one == "one"
    assert two == "two"
    assert three == "three"


def f():
    iter = ["one,two,three", "four,five,six"]
    reader = csv.reader(iter)

    [one, two, three] = next(reader)
    [four, five, six] = next(reader)

    assert one == "one"
    assert two == "two"
    assert three == "three"
    assert four == "four"
    assert five == "five"
    assert six == "six"


f()


def test_delim():
    iter = ["one|two|three", "four|five|six"]
    reader = csv.reader(iter, delimiter="|")

    [one, two, three] = next(reader)
    [four, five, six] = next(reader)

    assert one == "one"
    assert two == "two"
    assert three == "three"
    assert four == "four"
    assert five == "five"
    assert six == "six"

    with assert_raises(TypeError):
        iter = ["one,,two,,three"]
        csv.reader(iter, delimiter=",,")


test_delim()


def test_quote_strings_and_notnull_writer():
    string_buf = io.StringIO()
    csv.writer(string_buf, quoting=csv.QUOTE_STRINGS).writerow(["x", 1, None, ""])
    assert string_buf.getvalue() == '"x",1,,""\r\n'

    notnull_buf = io.StringIO()
    csv.writer(notnull_buf, quoting=csv.QUOTE_NOTNULL).writerow(["x", 1, None, ""])
    assert notnull_buf.getvalue() == '"x","1",,""\r\n'

    for quoting in (csv.QUOTE_STRINGS, csv.QUOTE_NOTNULL):
        buf = io.StringIO()
        csv.writer(buf, quoting=quoting).writerow([None, None])
        assert buf.getvalue() == ",\r\n"

        with assert_raises(csv.Error):
            csv.writer(io.StringIO(), quoting=quoting).writerow([None])

        with assert_raises(TypeError):
            csv.writer(io.StringIO(), quoting=quoting, quotechar=None)


test_quote_strings_and_notnull_writer()


def test_quote_none_writer_without_quotechar():
    no_quotechar_buf = io.StringIO()
    csv.writer(
        no_quotechar_buf,
        quoting=csv.QUOTE_NONE,
        quotechar=None,
        escapechar="\\",
    ).writerow(["a,b", 'x"y'])
    assert no_quotechar_buf.getvalue() == 'a\\,b,x"y\r\n'

    default_quotechar_buf = io.StringIO()
    csv.writer(
        default_quotechar_buf,
        quoting=csv.QUOTE_NONE,
        escapechar="\\",
    ).writerow(["a,b", 'x"y'])
    assert default_quotechar_buf.getvalue() == 'a\\,b,x\\"y\r\n'

    escapechar_buf = io.StringIO()
    csv.writer(
        escapechar_buf,
        quoting=csv.QUOTE_NONE,
        quotechar=None,
        escapechar="\\",
    ).writerow(["a\\b"])
    assert escapechar_buf.getvalue() == "a\\\\b\r\n"

    linebreak_buf = io.StringIO()
    csv.writer(
        linebreak_buf,
        quoting=csv.QUOTE_NONE,
        quotechar=None,
        escapechar="\\",
    ).writerow(["a\rb", "c\nd"])
    assert linebreak_buf.getvalue() == "a\\\rb,c\\\nd\r\n"

    with assert_raises(csv.Error):
        csv.writer(io.StringIO(), quoting=csv.QUOTE_NONE, quotechar=None).writerow(
            ["a,b"]
        )

    with assert_raises(csv.Error):
        csv.writer(
            io.StringIO(),
            quoting=csv.QUOTE_NONE,
            quotechar=None,
            escapechar="\\",
        ).writerow([None])

    two_empty_buf = io.StringIO()
    csv.writer(
        two_empty_buf,
        quoting=csv.QUOTE_NONE,
        quotechar=None,
        escapechar="\\",
    ).writerow([None, ""])
    assert two_empty_buf.getvalue() == ",\r\n"


test_quote_none_writer_without_quotechar()


def test_quote_none_reader_skipinitialspace_escapechar():
    reader = csv.reader(
        ["a,  b,\\ c,d"],
        quoting=csv.QUOTE_NONE,
        escapechar="\\",
        skipinitialspace=True,
    )
    assert list(reader) == [["a", "b", " c", "d"]]


test_quote_none_reader_skipinitialspace_escapechar()


def test_quote_minimal_writer_lineterminator():
    # https://github.com/RustPython/RustPython/issues/8302
    # QUOTE_MINIMAL must quote '\r' and '\n' regardless of the line terminator.
    buf = io.StringIO()
    writer = csv.writer(buf, lineterminator="!")
    writer.writerow(["a", "b"])
    writer.writerow([1, 2])
    writer.writerow(["\r", "\n"])
    assert buf.getvalue() == 'a,b!1,2!"\r","\n"!'

    nul = io.StringIO()
    csv.writer(nul, lineterminator="\0").writerow(["\r", "\n"])
    assert nul.getvalue() == '"\r","\n"\0'

    crlf = io.StringIO()
    csv.writer(crlf, lineterminator="!").writerow(["\r\n"])
    assert crlf.getvalue() == '"\r\n"!'

    # the terminator character itself still triggers quoting
    term = io.StringIO()
    csv.writer(term, lineterminator="!").writerow(["a!b", "c"])
    assert term.getvalue() == '"a!b",c!'

    # default terminator behavior is unchanged
    default = io.StringIO()
    csv.writer(default).writerow(["\r", "\n"])
    assert default.getvalue() == '"\r","\n"\r\n'


test_quote_minimal_writer_lineterminator()


def test_multichar_lineterminator():
    # https://github.com/RustPython/RustPython/issues/8322
    # The writer must store and emit a full multi-character line terminator.
    for lineterminator in "\r\n", "\n", "\r", "!@#", "\0":
        buf = io.StringIO()
        writer = csv.writer(buf, lineterminator=lineterminator)
        writer.writerow(["a", "b"])
        writer.writerow([1, 2])
        writer.writerow(["\r", "\n"])
        assert buf.getvalue() == (
            f'a,b{lineterminator}1,2{lineterminator}"\r","\n"{lineterminator}'
        ), (lineterminator, buf.getvalue())

    # A field is quoted when it contains any byte of the terminator (QUOTE_MINIMAL).
    for field, expected in [
        ("a@b", '"a@b",x!@#'),
        ("a!b", '"a!b",x!@#'),
        ("a#b", '"a#b",x!@#'),
        ("abc", "abc,x!@#"),
    ]:
        buf = io.StringIO()
        csv.writer(buf, lineterminator="!@#").writerow([field, "x"])
        assert buf.getvalue() == expected, (field, buf.getvalue())

    # The csv-core-backed QUOTE_ALL / QUOTE_NONNUMERIC paths emit the full
    # terminator too, and keep the state machine correct across rows.
    allq = io.StringIO()
    writer = csv.writer(allq, lineterminator="!@#", quoting=csv.QUOTE_ALL)
    writer.writerow(["a", "b"])
    writer.writerow(["c", "d"])
    assert allq.getvalue() == '"a","b"!@#"c","d"!@#', allq.getvalue()

    nonnum = io.StringIO()
    csv.writer(nonnum, lineterminator="!@#", quoting=csv.QUOTE_NONNUMERIC).writerow(
        ["a", 1]
    )
    assert nonnum.getvalue() == '"a",1!@#', nonnum.getvalue()

    # QUOTE_NONE escapes any byte of the terminator.
    none = io.StringIO()
    csv.writer(
        none, lineterminator="!@#", quoting=csv.QUOTE_NONE, escapechar="\\"
    ).writerow(["a!b", "x"])
    assert none.getvalue() == "a\\!b,x!@#", none.getvalue()

    # register_dialect round-trips a multi-character terminator.
    csv.register_dialect("multichar_lt", delimiter=",", lineterminator="!@#")
    try:
        reg = io.StringIO()
        csv.writer(reg, dialect="multichar_lt").writerow(["a", "b"])
        assert reg.getvalue() == "a,b!@#", reg.getvalue()
    finally:
        csv.unregister_dialect("multichar_lt")

    # The dialect attribute reflects the full terminator.
    assert (
        csv.writer(io.StringIO(), lineterminator="!@#").dialect.lineterminator == "!@#"
    )

    # The reader ignores lineterminator (like CPython) and only splits on \r\n.
    assert list(csv.reader(io.StringIO("a,b!@#c,d!@#"), lineterminator="!@#")) == [
        ["a", "b!@#c", "d!@#"]
    ]


test_multichar_lineterminator()


def test_quote_minimal_writer_empty_fields():
    buf = io.StringIO()
    writer = csv.writer(buf)
    writer.writerow([""])
    writer.writerow([None])
    writer.writerow([])
    writer.writerow(["", ""])
    assert buf.getvalue() == '""\r\n""\r\n\r\n,\r\n'


test_quote_minimal_writer_empty_fields()


def test_reader_skipinitialspace_preserves_quoted_spaces():
    reader = csv.reader(['a, "b, c", d'], skipinitialspace=True)
    assert list(reader) == [["a", "b, c", "d"]]


test_reader_skipinitialspace_preserves_quoted_spaces()
