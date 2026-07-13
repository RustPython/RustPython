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
