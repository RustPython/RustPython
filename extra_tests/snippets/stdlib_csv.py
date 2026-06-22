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
