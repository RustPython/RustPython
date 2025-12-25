def test_except_star_with_plain_exception():
    try:
        raise ValueError("x")
    except* ValueError as err:
        assert isinstance(err, ExceptionGroup)
        assert err.exceptions == (ValueError("x"),)
    else:
        raise AssertionError("except* handler did not run")


def test_starred_index_builds_tuple():
    target = {}
    target[*"ab"] = 1
    assert list(target.items()) == [(("a", "b"), 1)]


if __name__ == "__main__":
    test_except_star_with_plain_exception()
    test_starred_index_builds_tuple()
