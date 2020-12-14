import sys


def expect_cannot_fit_index_error(s, index):
    try:
        s[index]
    except IndexError as error:
        assert str(error) == "cannot fit 'int' into an index-sized integer"
    else:
        assert False


MAX_INDEX = sys.maxsize + 1
MIN_INDEX = -(MAX_INDEX + 1)

test_str = "test"
expect_cannot_fit_index_error(test_str, MIN_INDEX)
expect_cannot_fit_index_error(test_str, MAX_INDEX)

test_list = [0, 1, 2, 3]
expect_cannot_fit_index_error(test_list, MIN_INDEX)
expect_cannot_fit_index_error(test_list, MAX_INDEX)
