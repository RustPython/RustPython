import _csv
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

    # A field that itself contains a line-break byte must be kept intact: the
    # csv-core path drops only the trailing record terminator, not a byte from
    # the field data.
    embedded = io.StringIO()
    csv.writer(embedded, lineterminator="!@#", quoting=csv.QUOTE_ALL).writerow(
        ["x\ny", "z"]
    )
    assert embedded.getvalue() == '"x\ny","z"!@#', embedded.getvalue()

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
    assert list(csv.reader(io.StringIO("a,b\r\nc,d\r\n"), lineterminator="!@#")) == [
        ["a", "b"],
        ["c", "d"],
    ]


test_multichar_lineterminator()


def test_empty_lineterminator():
    class EmptyLineTerminator(csv.excel):
        lineterminator = ""

    keyword = io.StringIO()
    csv.writer(keyword, lineterminator="").writerows([["a", "b"], ["c", "d"]])
    assert keyword.getvalue() == "a,bc,d"

    dialect = io.StringIO()
    csv.writer(dialect, dialect=EmptyLineTerminator).writerows([["a", "b"], ["c", "d"]])
    assert dialect.getvalue() == "a,bc,d"

    source = "a,b\r\nc,d\n"
    assert list(csv.reader(io.StringIO(source), lineterminator="")) == [
        ["a", "b"],
        ["c", "d"],
    ]
    assert list(csv.reader(io.StringIO(source), dialect=EmptyLineTerminator)) == [
        ["a", "b"],
        ["c", "d"],
    ]


test_empty_lineterminator()


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

# The reader is built with the default dialect, which it has to look up itself.

with assert_raises(StopIteration):
    next(_csv.reader([]))


def test_quote_nonnumeric_writer():
    class CustomInt:
        def __int__(self):
            return 42

        def __str__(self):
            return "42"

    class NumberWithDelimiter:
        def __int__(self):
            return 1

        def __str__(self):
            return "1,2"

    buf = io.StringIO()
    writer = csv.writer(buf, quoting=csv.QUOTE_NONNUMERIC)
    writer.writerow(["123", 123, "a"])
    writer.writerow([12.5, "12.5", True, False, None])
    writer.writerow([CustomInt(), NumberWithDelimiter()])
    writer.writerow([""])
    writer.writerow([None])
    writer.writerow([])
    writer.writerow(["", ""])
    assert buf.getvalue() == (
        '"123",123,"a"\r\n'
        '12.5,"12.5",True,False,""\r\n'
        '42,"1,2"\r\n'
        '""\r\n'
        '""\r\n'
        "\r\n"
        '"",""\r\n'
    ), repr(buf.getvalue())

    # QUOTE_NONNUMERIC with space delimiter and skipinitialspace
    sp_buf = io.StringIO()
    csv.writer(
        sp_buf,
        delimiter=" ",
        skipinitialspace=True,
        quoting=csv.QUOTE_NONNUMERIC,
    ).writerow(["a", "", "b"])
    assert sp_buf.getvalue() == '"a" "" "b"\r\n', repr(sp_buf.getvalue())


test_quote_nonnumeric_writer()


def test_dialect_truth_conversion():
    def registered(dialect, **kwargs):
        name = "truth_conversion"
        try:
            csv.register_dialect(name, dialect, **kwargs)
            return csv.get_dialect(name)
        finally:
            if name in csv.list_dialects():
                csv.unregister_dialect(name)

    constructors = (
        lambda dialect, **kwargs: csv.reader([], dialect, **kwargs).dialect,
        lambda dialect, **kwargs: csv.writer(io.StringIO(), dialect, **kwargs).dialect,
        _csv.Dialect,
        registered,
    )

    class TruthError:
        def __init__(self, error):
            self.error = error
            self.calls = 0

        def __bool__(self):
            self.calls += 1
            raise self.error

    class InvalidTruth:
        def __bool__(self):
            return 1

    for construct in constructors:
        for field in ("strict", "doublequote", "skipinitialspace"):
            for error_type in (RuntimeError, AttributeError, TypeError):
                for use_class in (False, True):
                    error = error_type("dialect truth conversion")
                    value = TruthError(error)

                    class Dialect(csv.excel):
                        def __new__(cls):
                            raise AssertionError(
                                "dialect classes must not be instantiated"
                            )

                    dialect = Dialect if use_class else csv.excel()
                    setattr(dialect, field, value)
                    try:
                        construct(dialect)
                    except error_type as caught:
                        assert caught is error
                    else:
                        raise AssertionError("dialect truth error was suppressed")
                    assert value.calls == 1

                    # An explicit option replaces the attribute before conversion.
                    result = construct(dialect, **{field: True})
                    assert getattr(result, field) is True
                    assert value.calls == 1

                value = TruthError(error)
                try:
                    construct(None, **{field: value})
                except error_type as caught:
                    assert caught is error
                else:
                    raise AssertionError("keyword truth error was replaced")
                assert value.calls == 1

            for value in (None, False, True, 0, 1, [], [1]):
                dialect = csv.excel()
                setattr(dialect, field, value)
                assert getattr(construct(dialect), field) is bool(value)
                assert getattr(construct(None, **{field: value}), field) is bool(value)

            dialect = csv.excel()
            setattr(dialect, field, InvalidTruth())
            with assert_raises(TypeError):
                construct(dialect)
            with assert_raises(TypeError):
                construct(None, **{field: InvalidTruth()})

    # csv.Dialect's Python validation wrapper translates TypeError to csv.Error.
    for error_type in (RuntimeError, AttributeError, TypeError):
        error = error_type("direct Python dialect validation")
        value = TruthError(error)

        class Dialect(csv.excel):
            strict = value

        expected = csv.Error if error_type is TypeError else error_type
        try:
            Dialect()
        except expected as caught:
            assert str(caught) == str(error)
            if error_type is not TypeError:
                assert caught is error
        else:
            raise AssertionError("Python dialect validation swallowed the error")
        assert value.calls == 1


test_dialect_truth_conversion()


def test_dialect_lookup_and_conversion_order():
    fields = (
        "delimiter",
        "doublequote",
        "escapechar",
        "lineterminator",
        "quotechar",
        "quoting",
        "skipinitialspace",
        "strict",
    )
    defaults = (",", True, None, "\r\n", '"', csv.QUOTE_MINIMAL, False, False)
    constructors = (
        lambda dialect, **kwargs: csv.reader([], dialect, **kwargs).dialect,
        lambda dialect, **kwargs: csv.writer(io.StringIO(), dialect, **kwargs).dialect,
        _csv.Dialect,
    )

    for construct in constructors:
        for error_type in (AttributeError, RuntimeError, KeyboardInterrupt):
            events = []

            class Missing:
                def __getattribute__(self, name):
                    events.append(name)
                    raise error_type(name)

            if error_type is AttributeError:
                result = construct(Missing())
                assert tuple(getattr(result, field) for field in fields) == defaults
                assert events == list(fields)
            else:
                with assert_raises(error_type) as caught:
                    construct(Missing())
                assert caught.exception.args == ("delimiter",)
                assert events == ["delimiter"]
        for dialect in (None, object(), _csv.Dialect(csv.excel)):
            result = construct(dialect)
            assert tuple(getattr(result, field) for field in fields) == defaults

        events = []

        class Truth:
            def __init__(self, name):
                self.name = name

            def __bool__(self):
                events.append("bool:" + self.name)
                return False

        values = dict(zip(fields, defaults))
        for field in ("doublequote", "skipinitialspace", "strict"):
            values[field] = Truth(field)

        class Dialect:
            def __getattribute__(self, name):
                events.append(name)
                return values[name]

        construct(Dialect())
        assert events == list(fields) + [
            "bool:doublequote",
            "bool:skipinitialspace",
            "bool:strict",
        ]

        events.clear()
        construct(Dialect(), strict=True, doublequote=True)
        assert events == [
            field for field in fields if field not in ("strict", "doublequote")
        ] + ["bool:skipinitialspace"]

        events.clear()
        construct(
            None,
            strict=Truth("strict"),
            skipinitialspace=Truth("skipinitialspace"),
            doublequote=Truth("doublequote"),
        )
        assert events == ["bool:doublequote", "bool:skipinitialspace", "bool:strict"]

        # Collect every attribute before starting conversions, even on failure.
        events.clear()
        values["delimiter"] = "too long"
        with assert_raises(TypeError):
            construct(Dialect())
        assert events == list(fields)

        error = RuntimeError("truth conversion wins")

        class BadTruth:
            def __bool__(self):
                raise error

        # Conversion order differs from option-validation order.
        for options in (
            {"doublequote": BadTruth(), "escapechar": "too long"},
            {"strict": BadTruth(), "quoting": 42},
            {"strict": BadTruth(), "quotechar": None, "quoting": csv.QUOTE_ALL},
            {"strict": BadTruth(), "delimiter": "\n"},
        ):
            try:
                construct(None, **options)
            except RuntimeError as caught:
                assert caught is error
            else:
                raise AssertionError("conversion order did not preserve truth error")
        for options in (
            {"delimiter": "too long", "doublequote": BadTruth()},
            {"lineterminator": None, "skipinitialspace": BadTruth()},
            {"quoting": None, "strict": BadTruth()},
            {"unknown_option": None, "strict": BadTruth()},
        ):
            with assert_raises(TypeError):
                construct(None, **options)

        class IntegerSubclass(int):
            pass

        class Index:
            def __index__(self):
                raise AssertionError("quoting must not call __index__")

        for value in (True, IntegerSubclass(0), Index()):
            with assert_raises(TypeError):
                construct(None, quoting=value)
        with assert_raises(OverflowError):
            construct(None, quoting=2**40, strict=BadTruth())

        # None quotechar disables quoting only when quoting was not supplied.
        assert construct(None, quotechar=None).quoting == csv.QUOTE_NONE
        with assert_raises(TypeError):
            construct(csv.excel, quotechar=None)
        with assert_raises(TypeError):
            construct(None, quotechar=None, quoting=csv.QUOTE_MINIMAL)

    assert _csv.Dialect().strict is False
    assert _csv.Dialect(strict=True).strict is True
    for constructor, arg in ((csv.reader, []), (csv.writer, io.StringIO())):
        with assert_raises(TypeError):
            constructor(arg, None, None, strict=BadTruth())


test_dialect_lookup_and_conversion_order()
