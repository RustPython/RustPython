# A call that binds badly says which function it was binding for, counts only
# the arguments the caller wrote, and agrees with itself about singular and
# plural. Every message here is the one CPython raises, so this file is run
# against both.
import array
import collections
import csv
import itertools
import operator
import typing
import weakref

from testutils import assert_raises


def check(exc_type, message, fn, *args, **kwargs):
    try:
        fn(*args, **kwargs)
    except exc_type as e:
        assert str(e) == message, f"{str(e)!r} != {message!r}"
    else:
        raise AssertionError(f"{fn} did not raise {exc_type.__name__}")


def test_the_message_names_the_function():
    check(TypeError, "find expected at least 1 argument, got 0", "".find)
    check(TypeError, "center expected at least 1 argument, got 0", "a".center)
    check(TypeError, "fromkeys expected at least 1 argument, got 0", dict.fromkeys)
    check(TypeError, "divmod expected 2 arguments, got 1", divmod, 1)
    check(TypeError, "insert expected 2 arguments, got 1", [].insert, 1)
    check(TypeError, "sorted expected 1 argument, got 0", sorted)
    check(TypeError, "min expected at least 1 argument, got 0", min)


def test_a_method_does_not_count_its_instance():
    # `self` fills the instance parameter, so it is in neither number, whether
    # the call bound it or wrote it.
    check(TypeError, "insert expected 2 arguments, got 1", [].insert, 1)
    check(TypeError, "insert expected 2 arguments, got 1", list.insert, [], 1)


def test_one_argument_is_singular():
    check(TypeError, "sorted expected 1 argument, got 0", sorted)
    check(TypeError, "divmod expected 2 arguments, got 1", divmod, 1)
    check(TypeError, "float expected at most 1 argument, got 2", float, 1, 2)
    check(TypeError, "int expected at most 2 arguments, got 3", int, 1, 2, 3)


def test_a_keyword_it_did_not_expect_is_quoted():
    check(
        TypeError,
        "split() got an unexpected keyword argument 'bogus'",
        "".split,
        bogus=1,
    )
    check(
        TypeError,
        "round() got an unexpected keyword argument 'bogus'",
        round,
        1,
        bogus=2,
    )


def test_a_parameter_a_call_may_name_is_named_back():
    check(
        TypeError,
        "memoryview() missing required argument 'object' (pos 1)",
        memoryview,
    )
    check(
        TypeError,
        "cast() missing required argument 'format' (pos 1)",
        memoryview(b"a").cast,
    )


def test_a_type_is_named_by_the_type_it_builds():
    # The name is the type the slot was written for, without the module and
    # without the subclass being constructed.
    class Deque(collections.deque):
        pass

    check(TypeError, "range expected at least 1 argument, got 0", range)
    check(TypeError, "slice expected at least 1 argument, got 0", slice)
    check(TypeError, "filter expected 2 arguments, got 0", filter)
    check(TypeError, "set expected at most 1 argument, got 2", set, 1, 2)
    check(TypeError, "tuple expected at most 1 argument, got 2", tuple, 1, 2)
    check(TypeError, "staticmethod expected 1 argument, got 0", staticmethod)
    check(TypeError, "classmethod expected 1 argument, got 0", classmethod)
    with assert_raises(TypeError):
        Deque(1, 2, 3)
    with assert_raises(TypeError):
        array.array()


def test_a_slot_that_counts_its_own_arguments_says_the_same_thing():
    # Sites that check the count themselves raise what binding would have.
    class Obj:
        pass

    check(TypeError, "__new__ expected at least 1 argument, got 0", weakref.ref)
    check(
        TypeError,
        "__new__ expected at most 2 arguments, got 3",
        weakref.ref,
        Obj(),
        1,
        2,
    )
    check(TypeError, "GenericAlias expected 2 arguments, got 0", type(list[int]))
    check(TypeError, "frozenset expected at most 1 argument, got 2", frozenset, 1, 2)
    check(TypeError, "attrgetter expected 1 argument, got 0", operator.attrgetter)
    check(TypeError, "itemgetter expected 1 argument, got 0", operator.itemgetter)
    check(
        TypeError, "islice expected at least 2 arguments, got 1", itertools.islice, []
    )
    # The count is read before the keywords are.
    check(TypeError, "min expected at least 1 argument, got 0", min, bogus=1)
    check(TypeError, "max expected at least 1 argument, got 0", max, bogus=1)


def test_a_keyword_check_of_its_own_says_the_same_thing():
    check(
        TypeError,
        "AttributeError() got an unexpected keyword argument 'bogus'",
        AttributeError,
        bogus=1,
    )
    check(
        TypeError,
        "NameError() got an unexpected keyword argument 'bogus'",
        NameError,
        bogus=1,
    )
    check(
        TypeError,
        "typevar() got an unexpected keyword argument 'bogus'",
        typing.TypeVar,
        "T",
        bogus=1,
    )
    # The dialect is parsed by a parser of its own, which has no name to give.
    check(
        TypeError,
        "this function got an unexpected keyword argument 'bogus'",
        csv.reader,
        [],
        bogus=1,
    )


test_the_message_names_the_function()
test_a_method_does_not_count_its_instance()
test_one_argument_is_singular()
test_a_keyword_it_did_not_expect_is_quoted()
test_a_parameter_a_call_may_name_is_named_back()
test_a_type_is_named_by_the_type_it_builds()
test_a_slot_that_counts_its_own_arguments_says_the_same_thing()
test_a_keyword_check_of_its_own_says_the_same_thing()
