import contextlib
import collections
from collections import defaultdict
from functools import lru_cache, wraps
import inspect
import itertools
import pickle
import re
import sys
import warnings
from unittest import TestCase, main, skipUnless, skip
# TODO: RUSTPYTHON
import unittest
from copy import copy, deepcopy

from typing import Any, NoReturn, Never, assert_never
from typing import overload, get_overloads, clear_overloads
from typing import TypeVar, TypeVarTuple, Unpack, AnyStr
from typing import T, KT, VT  # Not in __all__.
from typing import Union, Optional, Literal
from typing import Tuple, List, Dict, MutableMapping
from typing import Callable
from typing import Generic, ClassVar, Final, final, Protocol
from typing import assert_type, cast, runtime_checkable
from typing import get_type_hints
from typing import get_origin, get_args
from typing import is_typeddict
from typing import reveal_type
from typing import dataclass_transform
from typing import no_type_check, no_type_check_decorator
from typing import Type
from typing import NamedTuple, NotRequired, Required, TypedDict
from typing import IO, TextIO, BinaryIO
from typing import Pattern, Match
from typing import Annotated, ForwardRef
from typing import Self, LiteralString
from typing import TypeAlias
from typing import ParamSpec, Concatenate, ParamSpecArgs, ParamSpecKwargs
from typing import TypeGuard
import abc
import textwrap
import typing
import weakref
import types

from test.support import import_helper, captured_stderr, cpython_only
from test import mod_generics_cache
from test import _typed_dict_helper


py_typing = import_helper.import_fresh_module('typing', blocked=['_typing'])
c_typing = import_helper.import_fresh_module('typing', fresh=['_typing'])

CANNOT_SUBCLASS_TYPE = 'Cannot subclass special typing classes'


class BaseTestCase(TestCase):

    def assertIsSubclass(self, cls, class_or_tuple, msg=None):
        if not issubclass(cls, class_or_tuple):
            message = '%r is not a subclass of %r' % (cls, class_or_tuple)
            if msg is not None:
                message += ' : %s' % msg
            raise self.failureException(message)

    def assertNotIsSubclass(self, cls, class_or_tuple, msg=None):
        if issubclass(cls, class_or_tuple):
            message = '%r is a subclass of %r' % (cls, class_or_tuple)
            if msg is not None:
                message += ' : %s' % msg
            raise self.failureException(message)

    def clear_caches(self):
        for f in typing._cleanups:
            f()


def all_pickle_protocols(test_func):
    """Runs `test_func` with various values for `proto` argument."""

    @wraps(test_func)
    def wrapper(self):
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.subTest(pickle_proto=proto):
                test_func(self, proto=proto)

    return wrapper


class Employee:
    pass


class Manager(Employee):
    pass


class Founder(Employee):
    pass


class ManagingFounder(Manager, Founder):
    pass


class AnyTests(BaseTestCase):

    def test_any_instance_type_error(self):
        with self.assertRaises(TypeError):
            isinstance(42, Any)

    def test_repr(self):
        self.assertEqual(repr(Any), 'typing.Any')

        class Sub(Any): pass
        self.assertEqual(
            repr(Sub),
            "<class 'test.test_typing.AnyTests.test_repr.<locals>.Sub'>",
        )

    def test_errors(self):
        with self.assertRaises(TypeError):
            issubclass(42, Any)
        with self.assertRaises(TypeError):
            Any[int]  # Any is not a generic type.

    def test_can_subclass(self):
        class Mock(Any): pass
        self.assertTrue(issubclass(Mock, Any))
        self.assertIsInstance(Mock(), Mock)

        class Something: pass
        self.assertFalse(issubclass(Something, Any))
        self.assertNotIsInstance(Something(), Mock)

        class MockSomething(Something, Mock): pass
        self.assertTrue(issubclass(MockSomething, Any))
        ms = MockSomething()
        self.assertIsInstance(ms, MockSomething)
        self.assertIsInstance(ms, Something)
        self.assertIsInstance(ms, Mock)

    def test_cannot_instantiate(self):
        with self.assertRaises(TypeError):
            Any()
        with self.assertRaises(TypeError):
            type(Any)()

    def test_any_works_with_alias(self):
        # These expressions must simply not fail.
        typing.Match[Any]
        typing.Pattern[Any]
        typing.IO[Any]


class BottomTypeTestsMixin:
    bottom_type: ClassVar[Any]

    def test_equality(self):
        self.assertEqual(self.bottom_type, self.bottom_type)
        self.assertIs(self.bottom_type, self.bottom_type)
        self.assertNotEqual(self.bottom_type, None)

    def test_get_origin(self):
        self.assertIs(get_origin(self.bottom_type), None)

    def test_instance_type_error(self):
        with self.assertRaises(TypeError):
            isinstance(42, self.bottom_type)

    def test_subclass_type_error(self):
        with self.assertRaises(TypeError):
            issubclass(Employee, self.bottom_type)
        with self.assertRaises(TypeError):
            issubclass(NoReturn, self.bottom_type)

    def test_not_generic(self):
        with self.assertRaises(TypeError):
            self.bottom_type[int]

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class A(self.bottom_type):
                pass
        with self.assertRaises(TypeError):
            class A(type(self.bottom_type)):
                pass

    def test_cannot_instantiate(self):
        with self.assertRaises(TypeError):
            self.bottom_type()
        with self.assertRaises(TypeError):
            type(self.bottom_type)()


class NoReturnTests(BottomTypeTestsMixin, BaseTestCase):
    bottom_type = NoReturn

    def test_repr(self):
        self.assertEqual(repr(NoReturn), 'typing.NoReturn')

    def test_get_type_hints(self):
        def some(arg: NoReturn) -> NoReturn: ...
        def some_str(arg: 'NoReturn') -> 'typing.NoReturn': ...

        expected = {'arg': NoReturn, 'return': NoReturn}
        for target in [some, some_str]:
            with self.subTest(target=target):
                self.assertEqual(gth(target), expected)

    def test_not_equality(self):
        self.assertNotEqual(NoReturn, Never)
        self.assertNotEqual(Never, NoReturn)


class NeverTests(BottomTypeTestsMixin, BaseTestCase):
    bottom_type = Never

    def test_repr(self):
        self.assertEqual(repr(Never), 'typing.Never')

    def test_get_type_hints(self):
        def some(arg: Never) -> Never: ...
        def some_str(arg: 'Never') -> 'typing.Never': ...

        expected = {'arg': Never, 'return': Never}
        for target in [some, some_str]:
            with self.subTest(target=target):
                self.assertEqual(gth(target), expected)


class AssertNeverTests(BaseTestCase):
    def test_exception(self):
        with self.assertRaises(AssertionError):
            assert_never(None)

        value = "some value"
        with self.assertRaisesRegex(AssertionError, value):
            assert_never(value)

        # Make sure a huge value doesn't get printed in its entirety
        huge_value = "a" * 10000
        with self.assertRaises(AssertionError) as cm:
            assert_never(huge_value)
        self.assertLess(
            len(cm.exception.args[0]),
            typing._ASSERT_NEVER_REPR_MAX_LENGTH * 2,
        )


class SelfTests(BaseTestCase):
    def test_equality(self):
        self.assertEqual(Self, Self)
        self.assertIs(Self, Self)
        self.assertNotEqual(Self, None)

    def test_basics(self):
        class Foo:
            def bar(self) -> Self: ...
        class FooStr:
            def bar(self) -> 'Self': ...
        class FooStrTyping:
            def bar(self) -> 'typing.Self': ...

        for target in [Foo, FooStr, FooStrTyping]:
            with self.subTest(target=target):
                self.assertEqual(gth(target.bar), {'return': Self})
        self.assertIs(get_origin(Self), None)

    def test_repr(self):
        self.assertEqual(repr(Self), 'typing.Self')

    def test_cannot_subscript(self):
        with self.assertRaises(TypeError):
            Self[int]

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(Self)):
                pass
        with self.assertRaises(TypeError):
            class C(Self):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            Self()
        with self.assertRaises(TypeError):
            type(Self)()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, Self)
        with self.assertRaises(TypeError):
            issubclass(int, Self)

    def test_alias(self):
        # TypeAliases are not actually part of the spec
        alias_1 = Tuple[Self, Self]
        alias_2 = List[Self]
        alias_3 = ClassVar[Self]
        self.assertEqual(get_args(alias_1), (Self, Self))
        self.assertEqual(get_args(alias_2), (Self,))
        self.assertEqual(get_args(alias_3), (Self,))


class LiteralStringTests(BaseTestCase):
    def test_equality(self):
        self.assertEqual(LiteralString, LiteralString)
        self.assertIs(LiteralString, LiteralString)
        self.assertNotEqual(LiteralString, None)

    def test_basics(self):
        class Foo:
            def bar(self) -> LiteralString: ...
        class FooStr:
            def bar(self) -> 'LiteralString': ...
        class FooStrTyping:
            def bar(self) -> 'typing.LiteralString': ...

        for target in [Foo, FooStr, FooStrTyping]:
            with self.subTest(target=target):
                self.assertEqual(gth(target.bar), {'return': LiteralString})
        self.assertIs(get_origin(LiteralString), None)

    def test_repr(self):
        self.assertEqual(repr(LiteralString), 'typing.LiteralString')

    def test_cannot_subscript(self):
        with self.assertRaises(TypeError):
            LiteralString[int]

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(LiteralString)):
                pass
        with self.assertRaises(TypeError):
            class C(LiteralString):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            LiteralString()
        with self.assertRaises(TypeError):
            type(LiteralString)()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, LiteralString)
        with self.assertRaises(TypeError):
            issubclass(int, LiteralString)

    def test_alias(self):
        alias_1 = Tuple[LiteralString, LiteralString]
        alias_2 = List[LiteralString]
        alias_3 = ClassVar[LiteralString]
        self.assertEqual(get_args(alias_1), (LiteralString, LiteralString))
        self.assertEqual(get_args(alias_2), (LiteralString,))
        self.assertEqual(get_args(alias_3), (LiteralString,))

class TypeVarTests(BaseTestCase):
    def test_basic_plain(self):
        T = TypeVar('T')
        # T equals itself.
        self.assertEqual(T, T)
        # T is an instance of TypeVar
        self.assertIsInstance(T, TypeVar)

    def test_typevar_instance_type_error(self):
        T = TypeVar('T')
        with self.assertRaises(TypeError):
            isinstance(42, T)

    def test_typevar_subclass_type_error(self):
        T = TypeVar('T')
        with self.assertRaises(TypeError):
            issubclass(int, T)
        with self.assertRaises(TypeError):
            issubclass(T, int)

    def test_constrained_error(self):
        with self.assertRaises(TypeError):
            X = TypeVar('X', int)
            X

    def test_union_unique(self):
        X = TypeVar('X')
        Y = TypeVar('Y')
        self.assertNotEqual(X, Y)
        self.assertEqual(Union[X], X)
        self.assertNotEqual(Union[X], Union[X, Y])
        self.assertEqual(Union[X, X], X)
        self.assertNotEqual(Union[X, int], Union[X])
        self.assertNotEqual(Union[X, int], Union[int])
        self.assertEqual(Union[X, int].__args__, (X, int))
        self.assertEqual(Union[X, int].__parameters__, (X,))
        self.assertIs(Union[X, int].__origin__, Union)

    def test_or(self):
        X = TypeVar('X')
        # use a string because str doesn't implement
        # __or__/__ror__ itself
        self.assertEqual(X | "x", Union[X, "x"])
        self.assertEqual("x" | X, Union["x", X])
        # make sure the order is correct
        self.assertEqual(get_args(X | "x"), (X, ForwardRef("x")))
        self.assertEqual(get_args("x" | X), (ForwardRef("x"), X))

    def test_union_constrained(self):
        A = TypeVar('A', str, bytes)
        self.assertNotEqual(Union[A, str], Union[A])

    def test_repr(self):
        self.assertEqual(repr(T), '~T')
        self.assertEqual(repr(KT), '~KT')
        self.assertEqual(repr(VT), '~VT')
        self.assertEqual(repr(AnyStr), '~AnyStr')
        T_co = TypeVar('T_co', covariant=True)
        self.assertEqual(repr(T_co), '+T_co')
        T_contra = TypeVar('T_contra', contravariant=True)
        self.assertEqual(repr(T_contra), '-T_contra')

    def test_no_redefinition(self):
        self.assertNotEqual(TypeVar('T'), TypeVar('T'))
        self.assertNotEqual(TypeVar('T', int, str), TypeVar('T', int, str))

    def test_cannot_subclass_vars(self):
        with self.assertRaises(TypeError):
            class V(TypeVar('T')):
                pass

    def test_cannot_subclass_var_itself(self):
        with self.assertRaises(TypeError):
            class V(TypeVar):
                pass

    def test_cannot_instantiate_vars(self):
        with self.assertRaises(TypeError):
            TypeVar('A')()

    def test_bound_errors(self):
        with self.assertRaises(TypeError):
            TypeVar('X', bound=Union)
        with self.assertRaises(TypeError):
            TypeVar('X', str, float, bound=Employee)

    def test_missing__name__(self):
        # See bpo-39942
        code = ("import typing\n"
                "T = typing.TypeVar('T')\n"
                )
        exec(code, {})

    def test_no_bivariant(self):
        with self.assertRaises(ValueError):
            TypeVar('T', covariant=True, contravariant=True)

    def test_var_substitution(self):
        T = TypeVar('T')
        subst = T.__typing_subst__
        self.assertIs(subst(int), int)
        self.assertEqual(subst(list[int]), list[int])
        self.assertEqual(subst(List[int]), List[int])
        self.assertEqual(subst(List), List)
        self.assertIs(subst(Any), Any)
        self.assertIs(subst(None), type(None))
        self.assertIs(subst(T), T)
        self.assertEqual(subst(int|str), int|str)
        self.assertEqual(subst(Union[int, str]), Union[int, str])

    def test_bad_var_substitution(self):
        T = TypeVar('T')
        P = ParamSpec("P")
        bad_args = (
            (), (int, str), Union,
            Generic, Generic[T], Protocol, Protocol[T],
            Final, Final[int], ClassVar, ClassVar[int],
        )
        for arg in bad_args:
            with self.subTest(arg=arg):
                with self.assertRaises(TypeError):
                    T.__typing_subst__(arg)
                with self.assertRaises(TypeError):
                    List[T][arg]
                with self.assertRaises(TypeError):
                    list[T][arg]


def template_replace(templates: list[str], replacements: dict[str, list[str]]) -> list[tuple[str]]:
    """Renders templates with possible combinations of replacements.

    Example 1: Suppose that:
      templates = ["dog_breed are awesome", "dog_breed are cool"]
      replacements = ["dog_breed": ["Huskies", "Beagles"]]
    Then we would return:
      [
          ("Huskies are awesome", "Huskies are cool"),
          ("Beagles are awesome", "Beagles are cool")
      ]

    Example 2: Suppose that:
      templates = ["Huskies are word1 but also word2"]
      replacements = {"word1": ["playful", "cute"],
                      "word2": ["feisty", "tiring"]}
    Then we would return:
      [
          ("Huskies are playful but also feisty"),
          ("Huskies are playful but also tiring"),
          ("Huskies are cute but also feisty"),
          ("Huskies are cute but also tiring")
      ]

    Note that if any of the replacements do not occur in any template:
      templates = ["Huskies are word1", "Beagles!"]
      replacements = {"word1": ["playful", "cute"],
                      "word2": ["feisty", "tiring"]}
    Then we do not generate duplicates, returning:
      [
          ("Huskies are playful", "Beagles!"),
          ("Huskies are cute", "Beagles!")
      ]
    """
    # First, build a structure like:
    #   [
    #     [("word1", "playful"), ("word1", "cute")],
    #     [("word2", "feisty"), ("word2", "tiring")]
    #   ]
    replacement_combos = []
    for original, possible_replacements in replacements.items():
        original_replacement_tuples = []
        for replacement in possible_replacements:
            original_replacement_tuples.append((original, replacement))
        replacement_combos.append(original_replacement_tuples)

    # Second, generate rendered templates, including possible duplicates.
    rendered_templates = []
    for replacement_combo in itertools.product(*replacement_combos):
        # replacement_combo would be e.g.
        #   [("word1", "playful"), ("word2", "feisty")]
        templates_with_replacements = []
        for template in templates:
            for original, replacement in replacement_combo:
                template = template.replace(original, replacement)
            templates_with_replacements.append(template)
        rendered_templates.append(tuple(templates_with_replacements))

    # Finally, remove the duplicates (but keep the order).
    rendered_templates_no_duplicates = []
    for x in rendered_templates:
        # Inefficient, but should be fine for our purposes.
        if x not in rendered_templates_no_duplicates:
            rendered_templates_no_duplicates.append(x)

    return rendered_templates_no_duplicates


class TemplateReplacementTests(BaseTestCase):

    def test_two_templates_two_replacements_yields_correct_renders(self):
        actual = template_replace(
                templates=["Cats are word1", "Dogs are word2"],
                replacements={
                    "word1": ["small", "cute"],
                    "word2": ["big", "fluffy"],
                },
        )
        expected = [
            ("Cats are small", "Dogs are big"),
            ("Cats are small", "Dogs are fluffy"),
            ("Cats are cute", "Dogs are big"),
            ("Cats are cute", "Dogs are fluffy"),
        ]
        self.assertEqual(actual, expected)

    def test_no_duplicates_if_replacement_not_in_templates(self):
        actual = template_replace(
                templates=["Cats are word1", "Dogs!"],
                replacements={
                    "word1": ["small", "cute"],
                    "word2": ["big", "fluffy"],
                },
        )
        expected = [
            ("Cats are small", "Dogs!"),
            ("Cats are cute", "Dogs!"),
        ]
        self.assertEqual(actual, expected)


class GenericAliasSubstitutionTests(BaseTestCase):
    """Tests for type variable substitution in generic aliases.

    For variadic cases, these tests should be regarded as the source of truth,
    since we hadn't realised the full complexity of variadic substitution
    at the time of finalizing PEP 646. For full discussion, see
    https://github.com/python/cpython/issues/91162.
    """

    def test_one_parameter(self):
        T = TypeVar('T')
        Ts = TypeVarTuple('Ts')
        Ts2 = TypeVarTuple('Ts2')

        class C(Generic[T]): pass

        generics = ['C', 'list', 'List']
        tuple_types = ['tuple', 'Tuple']

        tests = [
            # Alias                               # Args                     # Expected result
            ('generic[T]',                        '[()]',                    'TypeError'),
            ('generic[T]',                        '[int]',                   'generic[int]'),
            ('generic[T]',                        '[int, str]',              'TypeError'),
            ('generic[T]',                        '[tuple_type[int, ...]]',  'generic[tuple_type[int, ...]]'),
            ('generic[T]',                        '[*tuple_type[int]]',      'generic[int]'),
            ('generic[T]',                        '[*tuple_type[()]]',       'TypeError'),
            ('generic[T]',                        '[*tuple_type[int, str]]', 'TypeError'),
            ('generic[T]',                        '[*tuple_type[int, ...]]', 'TypeError'),
            ('generic[T]',                        '[*Ts]',                   'TypeError'),
            ('generic[T]',                        '[T, *Ts]',                'TypeError'),
            ('generic[T]',                        '[*Ts, T]',                'TypeError'),
            # Raises TypeError because C is not variadic.
            # (If C _were_ variadic, it'd be fine.)
            ('C[T, *tuple_type[int, ...]]',       '[int]',                   'TypeError'),
            # Should definitely raise TypeError: list only takes one argument.
            ('list[T, *tuple_type[int, ...]]',    '[int]',                   'list[int, *tuple_type[int, ...]]'),
            ('List[T, *tuple_type[int, ...]]',    '[int]',                   'TypeError'),
            # Should raise, because more than one `TypeVarTuple` is not supported.
            ('generic[*Ts, *Ts2]',                '[int]',                   'TypeError'),
        ]

        for alias_template, args_template, expected_template in tests:
            rendered_templates = template_replace(
                    templates=[alias_template, args_template, expected_template],
                    replacements={'generic': generics, 'tuple_type': tuple_types}
            )
            for alias_str, args_str, expected_str in rendered_templates:
                with self.subTest(alias=alias_str, args=args_str, expected=expected_str):
                    if expected_str == 'TypeError':
                        with self.assertRaises(TypeError):
                            eval(alias_str + args_str)
                    else:
                        self.assertEqual(
                            eval(alias_str + args_str),
                            eval(expected_str)
                        )


    def test_two_parameters(self):
        T1 = TypeVar('T1')
        T2 = TypeVar('T2')
        Ts = TypeVarTuple('Ts')

        class C(Generic[T1, T2]): pass

        generics = ['C', 'dict', 'Dict']
        tuple_types = ['tuple', 'Tuple']

        tests = [
            # Alias                                    # Args                                               # Expected result
            ('generic[T1, T2]',                        '[()]',                                              'TypeError'),
            ('generic[T1, T2]',                        '[int]',                                             'TypeError'),
            ('generic[T1, T2]',                        '[int, str]',                                        'generic[int, str]'),
            ('generic[T1, T2]',                        '[int, str, bool]',                                  'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int]]',                                'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int, str]]',                           'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[int, str, bool]]',                     'TypeError'),

            ('generic[T1, T2]',                        '[int, *tuple_type[str]]',                           'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[int], str]',                           'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[int], *tuple_type[str]]',              'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[int, str], *tuple_type[()]]',          'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[()], *tuple_type[int, str]]',          'generic[int, str]'),
            ('generic[T1, T2]',                        '[*tuple_type[int], *tuple_type[()]]',               'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[()], *tuple_type[int]]',               'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int, str], *tuple_type[float]]',       'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int], *tuple_type[str, float]]',       'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int, str], *tuple_type[float, bool]]', 'TypeError'),

            ('generic[T1, T2]',                        '[tuple_type[int, ...]]',                            'TypeError'),
            ('generic[T1, T2]',                        '[tuple_type[int, ...], tuple_type[str, ...]]',      'generic[tuple_type[int, ...], tuple_type[str, ...]]'),
            ('generic[T1, T2]',                        '[*tuple_type[int, ...]]',                           'TypeError'),
            ('generic[T1, T2]',                        '[int, *tuple_type[str, ...]]',                      'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int, ...], str]',                      'TypeError'),
            ('generic[T1, T2]',                        '[*tuple_type[int, ...], *tuple_type[str, ...]]',    'TypeError'),
            ('generic[T1, T2]',                        '[*Ts]',                                             'TypeError'),
            ('generic[T1, T2]',                        '[T, *Ts]',                                          'TypeError'),
            ('generic[T1, T2]',                        '[*Ts, T]',                                          'TypeError'),
            # This one isn't technically valid - none of the things that
            # `generic` can be (defined in `generics` above) are variadic, so we
            # shouldn't really be able to do `generic[T1, *tuple_type[int, ...]]`.
            # So even if type checkers shouldn't allow it, we allow it at
            # runtime, in accordance with a general philosophy of "Keep the
            # runtime lenient so people can experiment with typing constructs".
            ('generic[T1, *tuple_type[int, ...]]',     '[str]',                                             'generic[str, *tuple_type[int, ...]]'),
        ]

        for alias_template, args_template, expected_template in tests:
            rendered_templates = template_replace(
                    templates=[alias_template, args_template, expected_template],
                    replacements={'generic': generics, 'tuple_type': tuple_types}
            )
            for alias_str, args_str, expected_str in rendered_templates:
                with self.subTest(alias=alias_str, args=args_str, expected=expected_str):
                    if expected_str == 'TypeError':
                        with self.assertRaises(TypeError):
                            eval(alias_str + args_str)
                    else:
                        self.assertEqual(
                            eval(alias_str + args_str),
                            eval(expected_str)
                        )

    def test_three_parameters(self):
        T1 = TypeVar('T1')
        T2 = TypeVar('T2')
        T3 = TypeVar('T3')

        class C(Generic[T1, T2, T3]): pass

        generics = ['C']
        tuple_types = ['tuple', 'Tuple']

        tests = [
            # Alias                                    # Args                                               # Expected result
            ('generic[T1, bool, T2]',                  '[int, str]',                                        'generic[int, bool, str]'),
            ('generic[T1, bool, T2]',                  '[*tuple_type[int, str]]',                           'generic[int, bool, str]'),
        ]

        for alias_template, args_template, expected_template in tests:
            rendered_templates = template_replace(
                templates=[alias_template, args_template, expected_template],
                replacements={'generic': generics, 'tuple_type': tuple_types}
            )
            for alias_str, args_str, expected_str in rendered_templates:
                with self.subTest(alias=alias_str, args=args_str, expected=expected_str):
                    if expected_str == 'TypeError':
                        with self.assertRaises(TypeError):
                            eval(alias_str + args_str)
                    else:
                        self.assertEqual(
                            eval(alias_str + args_str),
                            eval(expected_str)
                        )

    def test_variadic_parameters(self):
        T1 = TypeVar('T1')
        T2 = TypeVar('T2')
        Ts = TypeVarTuple('Ts')

        class C(Generic[*Ts]): pass

        generics = ['C', 'tuple', 'Tuple']
        tuple_types = ['tuple', 'Tuple']

        tests = [
            # Alias                                    # Args                                            # Expected result
            ('generic[*Ts]',                           '[()]',                                           'generic[()]'),
            ('generic[*Ts]',                           '[int]',                                          'generic[int]'),
            ('generic[*Ts]',                           '[int, str]',                                     'generic[int, str]'),
            ('generic[*Ts]',                           '[*tuple_type[int]]',                             'generic[int]'),
            ('generic[*Ts]',                           '[*tuple_type[*Ts]]',                             'generic[*Ts]'),
            ('generic[*Ts]',                           '[*tuple_type[int, str]]',                        'generic[int, str]'),
            ('generic[*Ts]',                           '[str, *tuple_type[int, ...], bool]',             'generic[str, *tuple_type[int, ...], bool]'),
            ('generic[*Ts]',                           '[tuple_type[int, ...]]',                         'generic[tuple_type[int, ...]]'),
            ('generic[*Ts]',                           '[tuple_type[int, ...], tuple_type[str, ...]]',   'generic[tuple_type[int, ...], tuple_type[str, ...]]'),
            ('generic[*Ts]',                           '[*tuple_type[int, ...]]',                        'generic[*tuple_type[int, ...]]'),
            ('generic[*Ts]',                           '[*tuple_type[int, ...], *tuple_type[str, ...]]', 'TypeError'),

            ('generic[*Ts]',                           '[*Ts]',                                          'generic[*Ts]'),
            ('generic[*Ts]',                           '[T, *Ts]',                                       'generic[T, *Ts]'),
            ('generic[*Ts]',                           '[*Ts, T]',                                       'generic[*Ts, T]'),
            ('generic[T, *Ts]',                        '[()]',                                           'TypeError'),
            ('generic[T, *Ts]',                        '[int]',                                          'generic[int]'),
            ('generic[T, *Ts]',                        '[int, str]',                                     'generic[int, str]'),
            ('generic[T, *Ts]',                        '[int, str, bool]',                               'generic[int, str, bool]'),
            ('generic[list[T], *Ts]',                  '[()]',                                           'TypeError'),
            ('generic[list[T], *Ts]',                  '[int]',                                          'generic[list[int]]'),
            ('generic[list[T], *Ts]',                  '[int, str]',                                     'generic[list[int], str]'),
            ('generic[list[T], *Ts]',                  '[int, str, bool]',                               'generic[list[int], str, bool]'),

            ('generic[*Ts, T]',                        '[()]',                                           'TypeError'),
            ('generic[*Ts, T]',                        '[int]',                                          'generic[int]'),
            ('generic[*Ts, T]',                        '[int, str]',                                     'generic[int, str]'),
            ('generic[*Ts, T]',                        '[int, str, bool]',                               'generic[int, str, bool]'),
            ('generic[*Ts, list[T]]',                  '[()]',                                           'TypeError'),
            ('generic[*Ts, list[T]]',                  '[int]',                                          'generic[list[int]]'),
            ('generic[*Ts, list[T]]',                  '[int, str]',                                     'generic[int, list[str]]'),
            ('generic[*Ts, list[T]]',                  '[int, str, bool]',                               'generic[int, str, list[bool]]'),

            ('generic[T1, T2, *Ts]',                   '[()]',                                           'TypeError'),
            ('generic[T1, T2, *Ts]',                   '[int]',                                          'TypeError'),
            ('generic[T1, T2, *Ts]',                   '[int, str]',                                     'generic[int, str]'),
            ('generic[T1, T2, *Ts]',                   '[int, str, bool]',                               'generic[int, str, bool]'),
            ('generic[T1, T2, *Ts]',                   '[int, str, bool, bytes]',                        'generic[int, str, bool, bytes]'),

            ('generic[*Ts, T1, T2]',                   '[()]',                                           'TypeError'),
            ('generic[*Ts, T1, T2]',                   '[int]',                                          'TypeError'),
            ('generic[*Ts, T1, T2]',                   '[int, str]',                                     'generic[int, str]'),
            ('generic[*Ts, T1, T2]',                   '[int, str, bool]',                               'generic[int, str, bool]'),
            ('generic[*Ts, T1, T2]',                   '[int, str, bool, bytes]',                        'generic[int, str, bool, bytes]'),

            ('generic[T1, *Ts, T2]',                   '[()]',                                           'TypeError'),
            ('generic[T1, *Ts, T2]',                   '[int]',                                          'TypeError'),
            ('generic[T1, *Ts, T2]',                   '[int, str]',                                     'generic[int, str]'),
            ('generic[T1, *Ts, T2]',                   '[int, str, bool]',                               'generic[int, str, bool]'),
            ('generic[T1, *Ts, T2]',                   '[int, str, bool, bytes]',                        'generic[int, str, bool, bytes]'),

            ('generic[T, *Ts]',                        '[*tuple_type[int, ...]]',                        'generic[int, *tuple_type[int, ...]]'),
            ('generic[T, *Ts]',                        '[str, *tuple_type[int, ...]]',                   'generic[str, *tuple_type[int, ...]]'),
            ('generic[T, *Ts]',                        '[*tuple_type[int, ...], str]',                   'generic[int, *tuple_type[int, ...], str]'),
            ('generic[*Ts, T]',                        '[*tuple_type[int, ...]]',                        'generic[*tuple_type[int, ...], int]'),
            ('generic[*Ts, T]',                        '[str, *tuple_type[int, ...]]',                   'generic[str, *tuple_type[int, ...], int]'),
            ('generic[*Ts, T]',                        '[*tuple_type[int, ...], str]',                   'generic[*tuple_type[int, ...], str]'),
            ('generic[T1, *Ts, T2]',                   '[*tuple_type[int, ...]]',                        'generic[int, *tuple_type[int, ...], int]'),
            ('generic[T, str, *Ts]',                   '[*tuple_type[int, ...]]',                        'generic[int, str, *tuple_type[int, ...]]'),
            ('generic[*Ts, str, T]',                   '[*tuple_type[int, ...]]',                        'generic[*tuple_type[int, ...], str, int]'),
            ('generic[list[T], *Ts]',                  '[*tuple_type[int, ...]]',                        'generic[list[int], *tuple_type[int, ...]]'),
            ('generic[*Ts, list[T]]',                  '[*tuple_type[int, ...]]',                        'generic[*tuple_type[int, ...], list[int]]'),

            ('generic[T, *tuple_type[int, ...]]',      '[str]',                                          'generic[str, *tuple_type[int, ...]]'),
            ('generic[T1, T2, *tuple_type[int, ...]]', '[str, bool]',                                    'generic[str, bool, *tuple_type[int, ...]]'),
            ('generic[T1, *tuple_type[int, ...], T2]', '[str, bool]',                                    'generic[str, *tuple_type[int, ...], bool]'),
            ('generic[T1, *tuple_type[int, ...], T2]', '[str, bool, float]',                             'TypeError'),

            ('generic[T1, *tuple_type[T2, ...]]',      '[int, str]',                                     'generic[int, *tuple_type[str, ...]]'),
            ('generic[*tuple_type[T1, ...], T2]',      '[int, str]',                                     'generic[*tuple_type[int, ...], str]'),
            ('generic[T1, *tuple_type[generic[*Ts], ...]]', '[int, str, bool]',                          'generic[int, *tuple_type[generic[str, bool], ...]]'),
            ('generic[*tuple_type[generic[*Ts], ...], T1]', '[int, str, bool]',                          'generic[*tuple_type[generic[int, str], ...], bool]'),
        ]

        for alias_template, args_template, expected_template in tests:
            rendered_templates = template_replace(
                    templates=[alias_template, args_template, expected_template],
                    replacements={'generic': generics, 'tuple_type': tuple_types}
            )
            for alias_str, args_str, expected_str in rendered_templates:
                with self.subTest(alias=alias_str, args=args_str, expected=expected_str):
                    if expected_str == 'TypeError':
                        with self.assertRaises(TypeError):
                            eval(alias_str + args_str)
                    else:
                        self.assertEqual(
                            eval(alias_str + args_str),
                            eval(expected_str)
                        )



class UnpackTests(BaseTestCase):

    def test_accepts_single_type(self):
        (*tuple[int],)
        Unpack[Tuple[int]]

    def test_rejects_multiple_types(self):
        with self.assertRaises(TypeError):
            Unpack[Tuple[int], Tuple[str]]
        # We can't do the equivalent for `*` here -
        # *(Tuple[int], Tuple[str]) is just plain tuple unpacking,
        # which is valid.

    def test_rejects_multiple_parameterization(self):
        with self.assertRaises(TypeError):
            (*tuple[int],)[0][tuple[int]]
        with self.assertRaises(TypeError):
            Unpack[Tuple[int]][Tuple[int]]

    def test_cannot_be_called(self):
        with self.assertRaises(TypeError):
            Unpack()


class TypeVarTupleTests(BaseTestCase):

    def assertEndsWith(self, string, tail):
        if not string.endswith(tail):
            self.fail(f"String {string!r} does not end with {tail!r}")

    def test_name(self):
        Ts = TypeVarTuple('Ts')
        self.assertEqual(Ts.__name__, 'Ts')
        Ts2 = TypeVarTuple('Ts2')
        self.assertEqual(Ts2.__name__, 'Ts2')

    def test_instance_is_equal_to_itself(self):
        Ts = TypeVarTuple('Ts')
        self.assertEqual(Ts, Ts)

    def test_different_instances_are_different(self):
        self.assertNotEqual(TypeVarTuple('Ts'), TypeVarTuple('Ts'))

    def test_instance_isinstance_of_typevartuple(self):
        Ts = TypeVarTuple('Ts')
        self.assertIsInstance(Ts, TypeVarTuple)

    def test_cannot_call_instance(self):
        Ts = TypeVarTuple('Ts')
        with self.assertRaises(TypeError):
            Ts()

    def test_unpacked_typevartuple_is_equal_to_itself(self):
        Ts = TypeVarTuple('Ts')
        self.assertEqual((*Ts,)[0], (*Ts,)[0])
        self.assertEqual(Unpack[Ts], Unpack[Ts])

    def test_parameterised_tuple_is_equal_to_itself(self):
        Ts = TypeVarTuple('Ts')
        self.assertEqual(tuple[*Ts], tuple[*Ts])
        self.assertEqual(Tuple[Unpack[Ts]], Tuple[Unpack[Ts]])

    def tests_tuple_arg_ordering_matters(self):
        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')
        self.assertNotEqual(
            tuple[*Ts1, *Ts2],
            tuple[*Ts2, *Ts1],
        )
        self.assertNotEqual(
            Tuple[Unpack[Ts1], Unpack[Ts2]],
            Tuple[Unpack[Ts2], Unpack[Ts1]],
        )

    def test_tuple_args_and_parameters_are_correct(self):
        Ts = TypeVarTuple('Ts')
        t1 = tuple[*Ts]
        self.assertEqual(t1.__args__, (*Ts,))
        self.assertEqual(t1.__parameters__, (Ts,))
        t2 = Tuple[Unpack[Ts]]
        self.assertEqual(t2.__args__, (Unpack[Ts],))
        self.assertEqual(t2.__parameters__, (Ts,))

    def test_var_substitution(self):
        Ts = TypeVarTuple('Ts')
        T = TypeVar('T')
        T2 = TypeVar('T2')
        class G1(Generic[*Ts]): pass
        class G2(Generic[Unpack[Ts]]): pass

        for A in G1, G2, Tuple, tuple:
            B = A[*Ts]
            self.assertEqual(B[()], A[()])
            self.assertEqual(B[float], A[float])
            self.assertEqual(B[float, str], A[float, str])

            C = A[Unpack[Ts]]
            self.assertEqual(C[()], A[()])
            self.assertEqual(C[float], A[float])
            self.assertEqual(C[float, str], A[float, str])

            D = list[A[*Ts]]
            self.assertEqual(D[()], list[A[()]])
            self.assertEqual(D[float], list[A[float]])
            self.assertEqual(D[float, str], list[A[float, str]])

            E = List[A[Unpack[Ts]]]
            self.assertEqual(E[()], List[A[()]])
            self.assertEqual(E[float], List[A[float]])
            self.assertEqual(E[float, str], List[A[float, str]])

            F = A[T, *Ts, T2]
            with self.assertRaises(TypeError):
                F[()]
            with self.assertRaises(TypeError):
                F[float]
            self.assertEqual(F[float, str], A[float, str])
            self.assertEqual(F[float, str, int], A[float, str, int])
            self.assertEqual(F[float, str, int, bytes], A[float, str, int, bytes])

            G = A[T, Unpack[Ts], T2]
            with self.assertRaises(TypeError):
                G[()]
            with self.assertRaises(TypeError):
                G[float]
            self.assertEqual(G[float, str], A[float, str])
            self.assertEqual(G[float, str, int], A[float, str, int])
            self.assertEqual(G[float, str, int, bytes], A[float, str, int, bytes])

            H = tuple[list[T], A[*Ts], list[T2]]
            with self.assertRaises(TypeError):
                H[()]
            with self.assertRaises(TypeError):
                H[float]
            if A != Tuple:
                self.assertEqual(H[float, str],
                                 tuple[list[float], A[()], list[str]])
            self.assertEqual(H[float, str, int],
                             tuple[list[float], A[str], list[int]])
            self.assertEqual(H[float, str, int, bytes],
                             tuple[list[float], A[str, int], list[bytes]])

            I = Tuple[List[T], A[Unpack[Ts]], List[T2]]
            with self.assertRaises(TypeError):
                I[()]
            with self.assertRaises(TypeError):
                I[float]
            if A != Tuple:
                self.assertEqual(I[float, str],
                                 Tuple[List[float], A[()], List[str]])
            self.assertEqual(I[float, str, int],
                             Tuple[List[float], A[str], List[int]])
            self.assertEqual(I[float, str, int, bytes],
                             Tuple[List[float], A[str, int], List[bytes]])

    def test_bad_var_substitution(self):
        Ts = TypeVarTuple('Ts')
        T = TypeVar('T')
        T2 = TypeVar('T2')
        class G1(Generic[*Ts]): pass
        class G2(Generic[Unpack[Ts]]): pass

        for A in G1, G2, Tuple, tuple:
            B = A[Ts]
            with self.assertRaises(TypeError):
                B[int, str]

            C = A[T, T2]
            with self.assertRaises(TypeError):
                C[*Ts]
            with self.assertRaises(TypeError):
                C[Unpack[Ts]]

            B = A[T, *Ts, str, T2]
            with self.assertRaises(TypeError):
                B[int, *Ts]
            with self.assertRaises(TypeError):
                B[int, *Ts, *Ts]

            C = A[T, Unpack[Ts], str, T2]
            with self.assertRaises(TypeError):
                C[int, Unpack[Ts]]
            with self.assertRaises(TypeError):
                C[int, Unpack[Ts], Unpack[Ts]]

    def test_repr_is_correct(self):
        Ts = TypeVarTuple('Ts')
        T = TypeVar('T')
        T2 = TypeVar('T2')

        class G1(Generic[*Ts]): pass
        class G2(Generic[Unpack[Ts]]): pass

        self.assertEqual(repr(Ts), 'Ts')

        self.assertEqual(repr((*Ts,)[0]), '*Ts')
        self.assertEqual(repr(Unpack[Ts]), '*Ts')

        self.assertEqual(repr(tuple[*Ts]), 'tuple[*Ts]')
        self.assertEqual(repr(Tuple[Unpack[Ts]]), 'typing.Tuple[*Ts]')

        self.assertEqual(repr(*tuple[*Ts]), '*tuple[*Ts]')
        self.assertEqual(repr(Unpack[Tuple[Unpack[Ts]]]), '*typing.Tuple[*Ts]')

    def test_variadic_class_repr_is_correct(self):
        Ts = TypeVarTuple('Ts')
        class A(Generic[*Ts]): pass
        class B(Generic[Unpack[Ts]]): pass

        self.assertEndsWith(repr(A[()]), 'A[()]')
        self.assertEndsWith(repr(B[()]), 'B[()]')
        self.assertEndsWith(repr(A[float]), 'A[float]')
        self.assertEndsWith(repr(B[float]), 'B[float]')
        self.assertEndsWith(repr(A[float, str]), 'A[float, str]')
        self.assertEndsWith(repr(B[float, str]), 'B[float, str]')

        self.assertEndsWith(repr(A[*tuple[int, ...]]),
                            'A[*tuple[int, ...]]')
        self.assertEndsWith(repr(B[Unpack[Tuple[int, ...]]]),
                            'B[*typing.Tuple[int, ...]]')

        self.assertEndsWith(repr(A[float, *tuple[int, ...]]),
                            'A[float, *tuple[int, ...]]')
        self.assertEndsWith(repr(A[float, Unpack[Tuple[int, ...]]]),
                            'A[float, *typing.Tuple[int, ...]]')

        self.assertEndsWith(repr(A[*tuple[int, ...], str]),
                            'A[*tuple[int, ...], str]')
        self.assertEndsWith(repr(B[Unpack[Tuple[int, ...]], str]),
                            'B[*typing.Tuple[int, ...], str]')

        self.assertEndsWith(repr(A[float, *tuple[int, ...], str]),
                            'A[float, *tuple[int, ...], str]')
        self.assertEndsWith(repr(B[float, Unpack[Tuple[int, ...]], str]),
                            'B[float, *typing.Tuple[int, ...], str]')

    def test_variadic_class_alias_repr_is_correct(self):
        Ts = TypeVarTuple('Ts')
        class A(Generic[Unpack[Ts]]): pass

        B = A[*Ts]
        self.assertEndsWith(repr(B), 'A[*Ts]')
        self.assertEndsWith(repr(B[()]), 'A[()]')
        self.assertEndsWith(repr(B[float]), 'A[float]')
        self.assertEndsWith(repr(B[float, str]), 'A[float, str]')

        C = A[Unpack[Ts]]
        self.assertEndsWith(repr(C), 'A[*Ts]')
        self.assertEndsWith(repr(C[()]), 'A[()]')
        self.assertEndsWith(repr(C[float]), 'A[float]')
        self.assertEndsWith(repr(C[float, str]), 'A[float, str]')

        D = A[*Ts, int]
        self.assertEndsWith(repr(D), 'A[*Ts, int]')
        self.assertEndsWith(repr(D[()]), 'A[int]')
        self.assertEndsWith(repr(D[float]), 'A[float, int]')
        self.assertEndsWith(repr(D[float, str]), 'A[float, str, int]')

        E = A[Unpack[Ts], int]
        self.assertEndsWith(repr(E), 'A[*Ts, int]')
        self.assertEndsWith(repr(E[()]), 'A[int]')
        self.assertEndsWith(repr(E[float]), 'A[float, int]')
        self.assertEndsWith(repr(E[float, str]), 'A[float, str, int]')

        F = A[int, *Ts]
        self.assertEndsWith(repr(F), 'A[int, *Ts]')
        self.assertEndsWith(repr(F[()]), 'A[int]')
        self.assertEndsWith(repr(F[float]), 'A[int, float]')
        self.assertEndsWith(repr(F[float, str]), 'A[int, float, str]')

        G = A[int, Unpack[Ts]]
        self.assertEndsWith(repr(G), 'A[int, *Ts]')
        self.assertEndsWith(repr(G[()]), 'A[int]')
        self.assertEndsWith(repr(G[float]), 'A[int, float]')
        self.assertEndsWith(repr(G[float, str]), 'A[int, float, str]')

        H = A[int, *Ts, str]
        self.assertEndsWith(repr(H), 'A[int, *Ts, str]')
        self.assertEndsWith(repr(H[()]), 'A[int, str]')
        self.assertEndsWith(repr(H[float]), 'A[int, float, str]')
        self.assertEndsWith(repr(H[float, str]), 'A[int, float, str, str]')

        I = A[int, Unpack[Ts], str]
        self.assertEndsWith(repr(I), 'A[int, *Ts, str]')
        self.assertEndsWith(repr(I[()]), 'A[int, str]')
        self.assertEndsWith(repr(I[float]), 'A[int, float, str]')
        self.assertEndsWith(repr(I[float, str]), 'A[int, float, str, str]')

        J = A[*Ts, *tuple[str, ...]]
        self.assertEndsWith(repr(J), 'A[*Ts, *tuple[str, ...]]')
        self.assertEndsWith(repr(J[()]), 'A[*tuple[str, ...]]')
        self.assertEndsWith(repr(J[float]), 'A[float, *tuple[str, ...]]')
        self.assertEndsWith(repr(J[float, str]), 'A[float, str, *tuple[str, ...]]')

        K = A[Unpack[Ts], Unpack[Tuple[str, ...]]]
        self.assertEndsWith(repr(K), 'A[*Ts, *typing.Tuple[str, ...]]')
        self.assertEndsWith(repr(K[()]), 'A[*typing.Tuple[str, ...]]')
        self.assertEndsWith(repr(K[float]), 'A[float, *typing.Tuple[str, ...]]')
        self.assertEndsWith(repr(K[float, str]), 'A[float, str, *typing.Tuple[str, ...]]')

    def test_cannot_subclass_class(self):
        with self.assertRaises(TypeError):
            class C(TypeVarTuple): pass

    def test_cannot_subclass_instance(self):
        Ts = TypeVarTuple('Ts')
        with self.assertRaises(TypeError):
            class C(Ts): pass
        with self.assertRaisesRegex(TypeError, CANNOT_SUBCLASS_TYPE):
            class C(type(Unpack)): pass
        with self.assertRaisesRegex(TypeError, CANNOT_SUBCLASS_TYPE):
            class C(type(*Ts)): pass
        with self.assertRaisesRegex(TypeError, CANNOT_SUBCLASS_TYPE):
            class C(type(Unpack[Ts])): pass
        with self.assertRaisesRegex(TypeError,
                                    r'Cannot subclass typing\.Unpack'):
            class C(Unpack): pass
        with self.assertRaisesRegex(TypeError, r'Cannot subclass \*Ts'):
            class C(*Ts): pass
        with self.assertRaisesRegex(TypeError, r'Cannot subclass \*Ts'):
            class C(Unpack[Ts]): pass

    def test_variadic_class_args_are_correct(self):
        T = TypeVar('T')
        Ts = TypeVarTuple('Ts')
        class A(Generic[*Ts]): pass
        class B(Generic[Unpack[Ts]]): pass

        C = A[()]
        D = B[()]
        self.assertEqual(C.__args__, ())
        self.assertEqual(D.__args__, ())

        E = A[int]
        F = B[int]
        self.assertEqual(E.__args__, (int,))
        self.assertEqual(F.__args__, (int,))

        G = A[int, str]
        H = B[int, str]
        self.assertEqual(G.__args__, (int, str))
        self.assertEqual(H.__args__, (int, str))

        I = A[T]
        J = B[T]
        self.assertEqual(I.__args__, (T,))
        self.assertEqual(J.__args__, (T,))

        K = A[*Ts]
        L = B[Unpack[Ts]]
        self.assertEqual(K.__args__, (*Ts,))
        self.assertEqual(L.__args__, (Unpack[Ts],))

        M = A[T, *Ts]
        N = B[T, Unpack[Ts]]
        self.assertEqual(M.__args__, (T, *Ts))
        self.assertEqual(N.__args__, (T, Unpack[Ts]))

        O = A[*Ts, T]
        P = B[Unpack[Ts], T]
        self.assertEqual(O.__args__, (*Ts, T))
        self.assertEqual(P.__args__, (Unpack[Ts], T))

    def test_variadic_class_origin_is_correct(self):
        Ts = TypeVarTuple('Ts')

        class C(Generic[*Ts]): pass
        self.assertIs(C[int].__origin__, C)
        self.assertIs(C[T].__origin__, C)
        self.assertIs(C[Unpack[Ts]].__origin__, C)

        class D(Generic[Unpack[Ts]]): pass
        self.assertIs(D[int].__origin__, D)
        self.assertIs(D[T].__origin__, D)
        self.assertIs(D[Unpack[Ts]].__origin__, D)

    def test_get_type_hints_on_unpack_args(self):
        Ts = TypeVarTuple('Ts')

        def func1(*args: *Ts): pass
        self.assertEqual(gth(func1), {'args': Unpack[Ts]})

        def func2(*args: *tuple[int, str]): pass
        self.assertEqual(gth(func2), {'args': Unpack[tuple[int, str]]})

        class CustomVariadic(Generic[*Ts]): pass

        def func3(*args: *CustomVariadic[int, str]): pass
        self.assertEqual(gth(func3), {'args': Unpack[CustomVariadic[int, str]]})

    def test_get_type_hints_on_unpack_args_string(self):
        Ts = TypeVarTuple('Ts')

        def func1(*args: '*Ts'): pass
        self.assertEqual(gth(func1, localns={'Ts': Ts}),
                        {'args': Unpack[Ts]})

        def func2(*args: '*tuple[int, str]'): pass
        self.assertEqual(gth(func2), {'args': Unpack[tuple[int, str]]})

        class CustomVariadic(Generic[*Ts]): pass

        def func3(*args: '*CustomVariadic[int, str]'): pass
        self.assertEqual(gth(func3, localns={'CustomVariadic': CustomVariadic}),
                         {'args': Unpack[CustomVariadic[int, str]]})

    def test_tuple_args_are_correct(self):
        Ts = TypeVarTuple('Ts')

        self.assertEqual(tuple[*Ts].__args__, (*Ts,))
        self.assertEqual(Tuple[Unpack[Ts]].__args__, (Unpack[Ts],))

        self.assertEqual(tuple[*Ts, int].__args__, (*Ts, int))
        self.assertEqual(Tuple[Unpack[Ts], int].__args__, (Unpack[Ts], int))

        self.assertEqual(tuple[int, *Ts].__args__, (int, *Ts))
        self.assertEqual(Tuple[int, Unpack[Ts]].__args__, (int, Unpack[Ts]))

        self.assertEqual(tuple[int, *Ts, str].__args__,
                         (int, *Ts, str))
        self.assertEqual(Tuple[int, Unpack[Ts], str].__args__,
                         (int, Unpack[Ts], str))

        self.assertEqual(tuple[*Ts, int].__args__, (*Ts, int))
        self.assertEqual(Tuple[Unpack[Ts]].__args__, (Unpack[Ts],))

    def test_callable_args_are_correct(self):
        Ts = TypeVarTuple('Ts')
        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')

        # TypeVarTuple in the arguments

        a = Callable[[*Ts], None]
        b = Callable[[Unpack[Ts]], None]
        self.assertEqual(a.__args__, (*Ts, type(None)))
        self.assertEqual(b.__args__, (Unpack[Ts], type(None)))

        c = Callable[[int, *Ts], None]
        d = Callable[[int, Unpack[Ts]], None]
        self.assertEqual(c.__args__, (int, *Ts, type(None)))
        self.assertEqual(d.__args__, (int, Unpack[Ts], type(None)))

        e = Callable[[*Ts, int], None]
        f = Callable[[Unpack[Ts], int], None]
        self.assertEqual(e.__args__, (*Ts, int, type(None)))
        self.assertEqual(f.__args__, (Unpack[Ts], int, type(None)))

        g = Callable[[str, *Ts, int], None]
        h = Callable[[str, Unpack[Ts], int], None]
        self.assertEqual(g.__args__, (str, *Ts, int, type(None)))
        self.assertEqual(h.__args__, (str, Unpack[Ts], int, type(None)))

        # TypeVarTuple as the return

        i = Callable[[None], *Ts]
        j = Callable[[None], Unpack[Ts]]
        self.assertEqual(i.__args__, (type(None), *Ts))
        self.assertEqual(i.__args__, (type(None), Unpack[Ts]))

        k = Callable[[None], tuple[int, *Ts]]
        l = Callable[[None], Tuple[int, Unpack[Ts]]]
        self.assertEqual(k.__args__, (type(None), tuple[int, *Ts]))
        self.assertEqual(l.__args__, (type(None), Tuple[int, Unpack[Ts]]))

        m = Callable[[None], tuple[*Ts, int]]
        n = Callable[[None], Tuple[Unpack[Ts], int]]
        self.assertEqual(m.__args__, (type(None), tuple[*Ts, int]))
        self.assertEqual(n.__args__, (type(None), Tuple[Unpack[Ts], int]))

        o = Callable[[None], tuple[str, *Ts, int]]
        p = Callable[[None], Tuple[str, Unpack[Ts], int]]
        self.assertEqual(o.__args__, (type(None), tuple[str, *Ts, int]))
        self.assertEqual(p.__args__, (type(None), Tuple[str, Unpack[Ts], int]))

        # TypeVarTuple in both

        q = Callable[[*Ts], *Ts]
        r = Callable[[Unpack[Ts]], Unpack[Ts]]
        self.assertEqual(q.__args__, (*Ts, *Ts))
        self.assertEqual(r.__args__, (Unpack[Ts], Unpack[Ts]))

        s = Callable[[*Ts1], *Ts2]
        u = Callable[[Unpack[Ts1]], Unpack[Ts2]]
        self.assertEqual(s.__args__, (*Ts1, *Ts2))
        self.assertEqual(u.__args__, (Unpack[Ts1], Unpack[Ts2]))

    def test_variadic_class_with_duplicate_typevartuples_fails(self):
        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')

        with self.assertRaises(TypeError):
            class C(Generic[*Ts1, *Ts1]): pass
        with self.assertRaises(TypeError):
            class C(Generic[Unpack[Ts1], Unpack[Ts1]]): pass

        with self.assertRaises(TypeError):
            class C(Generic[*Ts1, *Ts2, *Ts1]): pass
        with self.assertRaises(TypeError):
            class C(Generic[Unpack[Ts1], Unpack[Ts2], Unpack[Ts1]]): pass

    def test_type_concatenation_in_variadic_class_argument_list_succeeds(self):
        Ts = TypeVarTuple('Ts')
        class C(Generic[Unpack[Ts]]): pass

        C[int, *Ts]
        C[int, Unpack[Ts]]

        C[*Ts, int]
        C[Unpack[Ts], int]

        C[int, *Ts, str]
        C[int, Unpack[Ts], str]

        C[int, bool, *Ts, float, str]
        C[int, bool, Unpack[Ts], float, str]

    def test_type_concatenation_in_tuple_argument_list_succeeds(self):
        Ts = TypeVarTuple('Ts')

        tuple[int, *Ts]
        tuple[*Ts, int]
        tuple[int, *Ts, str]
        tuple[int, bool, *Ts, float, str]

        Tuple[int, Unpack[Ts]]
        Tuple[Unpack[Ts], int]
        Tuple[int, Unpack[Ts], str]
        Tuple[int, bool, Unpack[Ts], float, str]

    def test_variadic_class_definition_using_packed_typevartuple_fails(self):
        Ts = TypeVarTuple('Ts')
        with self.assertRaises(TypeError):
            class C(Generic[Ts]): pass

    def test_variadic_class_definition_using_concrete_types_fails(self):
        Ts = TypeVarTuple('Ts')
        with self.assertRaises(TypeError):
            class F(Generic[*Ts, int]): pass
        with self.assertRaises(TypeError):
            class E(Generic[Unpack[Ts], int]): pass

    def test_variadic_class_with_2_typevars_accepts_2_or_more_args(self):
        Ts = TypeVarTuple('Ts')
        T1 = TypeVar('T1')
        T2 = TypeVar('T2')

        class A(Generic[T1, T2, *Ts]): pass
        A[int, str]
        A[int, str, float]
        A[int, str, float, bool]

        class B(Generic[T1, T2, Unpack[Ts]]): pass
        B[int, str]
        B[int, str, float]
        B[int, str, float, bool]

        class C(Generic[T1, *Ts, T2]): pass
        C[int, str]
        C[int, str, float]
        C[int, str, float, bool]

        class D(Generic[T1, Unpack[Ts], T2]): pass
        D[int, str]
        D[int, str, float]
        D[int, str, float, bool]

        class E(Generic[*Ts, T1, T2]): pass
        E[int, str]
        E[int, str, float]
        E[int, str, float, bool]

        class F(Generic[Unpack[Ts], T1, T2]): pass
        F[int, str]
        F[int, str, float]
        F[int, str, float, bool]

    def test_variadic_args_annotations_are_correct(self):
        Ts = TypeVarTuple('Ts')

        def f(*args: Unpack[Ts]): pass
        def g(*args: *Ts): pass
        self.assertEqual(f.__annotations__, {'args': Unpack[Ts]})
        self.assertEqual(g.__annotations__, {'args': (*Ts,)[0]})

    def test_variadic_args_with_ellipsis_annotations_are_correct(self):
        Ts = TypeVarTuple('Ts')

        def a(*args: *tuple[int, ...]): pass
        self.assertEqual(a.__annotations__,
                         {'args': (*tuple[int, ...],)[0]})

        def b(*args: Unpack[Tuple[int, ...]]): pass
        self.assertEqual(b.__annotations__,
                         {'args': Unpack[Tuple[int, ...]]})

    def test_concatenation_in_variadic_args_annotations_are_correct(self):
        Ts = TypeVarTuple('Ts')

        # Unpacking using `*`, native `tuple` type

        def a(*args: *tuple[int, *Ts]): pass
        self.assertEqual(
            a.__annotations__,
            {'args': (*tuple[int, *Ts],)[0]},
        )

        def b(*args: *tuple[*Ts, int]): pass
        self.assertEqual(
            b.__annotations__,
            {'args': (*tuple[*Ts, int],)[0]},
        )

        def c(*args: *tuple[str, *Ts, int]): pass
        self.assertEqual(
            c.__annotations__,
            {'args': (*tuple[str, *Ts, int],)[0]},
        )

        def d(*args: *tuple[int, bool, *Ts, float, str]): pass
        self.assertEqual(
            d.__annotations__,
            {'args': (*tuple[int, bool, *Ts, float, str],)[0]},
        )

        # Unpacking using `Unpack`, `Tuple` type from typing.py

        def e(*args: Unpack[Tuple[int, Unpack[Ts]]]): pass
        self.assertEqual(
            e.__annotations__,
            {'args': Unpack[Tuple[int, Unpack[Ts]]]},
        )

        def f(*args: Unpack[Tuple[Unpack[Ts], int]]): pass
        self.assertEqual(
            f.__annotations__,
            {'args': Unpack[Tuple[Unpack[Ts], int]]},
        )

        def g(*args: Unpack[Tuple[str, Unpack[Ts], int]]): pass
        self.assertEqual(
            g.__annotations__,
            {'args': Unpack[Tuple[str, Unpack[Ts], int]]},
        )

        def h(*args: Unpack[Tuple[int, bool, Unpack[Ts], float, str]]): pass
        self.assertEqual(
            h.__annotations__,
            {'args': Unpack[Tuple[int, bool, Unpack[Ts], float, str]]},
        )

    def test_variadic_class_same_args_results_in_equalty(self):
        Ts = TypeVarTuple('Ts')
        class C(Generic[*Ts]): pass
        class D(Generic[Unpack[Ts]]): pass

        self.assertEqual(C[int], C[int])
        self.assertEqual(D[int], D[int])

        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')

        self.assertEqual(
            C[*Ts1],
            C[*Ts1],
        )
        self.assertEqual(
            D[Unpack[Ts1]],
            D[Unpack[Ts1]],
        )

        self.assertEqual(
            C[*Ts1, *Ts2],
            C[*Ts1, *Ts2],
        )
        self.assertEqual(
            D[Unpack[Ts1], Unpack[Ts2]],
            D[Unpack[Ts1], Unpack[Ts2]],
        )

        self.assertEqual(
            C[int, *Ts1, *Ts2],
            C[int, *Ts1, *Ts2],
        )
        self.assertEqual(
            D[int, Unpack[Ts1], Unpack[Ts2]],
            D[int, Unpack[Ts1], Unpack[Ts2]],
        )

    def test_variadic_class_arg_ordering_matters(self):
        Ts = TypeVarTuple('Ts')
        class C(Generic[*Ts]): pass
        class D(Generic[Unpack[Ts]]): pass

        self.assertNotEqual(
            C[int, str],
            C[str, int],
        )
        self.assertNotEqual(
            D[int, str],
            D[str, int],
        )

        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')

        self.assertNotEqual(
            C[*Ts1, *Ts2],
            C[*Ts2, *Ts1],
        )
        self.assertNotEqual(
            D[Unpack[Ts1], Unpack[Ts2]],
            D[Unpack[Ts2], Unpack[Ts1]],
        )

    def test_variadic_class_arg_typevartuple_identity_matters(self):
        Ts = TypeVarTuple('Ts')
        Ts1 = TypeVarTuple('Ts1')
        Ts2 = TypeVarTuple('Ts2')

        class C(Generic[*Ts]): pass
        class D(Generic[Unpack[Ts]]): pass

        self.assertNotEqual(C[*Ts1], C[*Ts2])
        self.assertNotEqual(D[Unpack[Ts1]], D[Unpack[Ts2]])


class TypeVarTuplePicklingTests(BaseTestCase):
    # These are slightly awkward tests to run, because TypeVarTuples are only
    # picklable if defined in the global scope. We therefore need to push
    # various things defined in these tests into the global scope with `global`
    # statements at the start of each test.

    @all_pickle_protocols
    def test_pickling_then_unpickling_results_in_same_identity(self, proto):
        global global_Ts1  # See explanation at start of class.
        global_Ts1 = TypeVarTuple('global_Ts1')
        global_Ts2 = pickle.loads(pickle.dumps(global_Ts1, proto))
        self.assertIs(global_Ts1, global_Ts2)

    @all_pickle_protocols
    def test_pickling_then_unpickling_unpacked_results_in_same_identity(self, proto):
        global global_Ts  # See explanation at start of class.
        global_Ts = TypeVarTuple('global_Ts')

        unpacked1 = (*global_Ts,)[0]
        unpacked2 = pickle.loads(pickle.dumps(unpacked1, proto))
        self.assertIs(unpacked1, unpacked2)

        unpacked3 = Unpack[global_Ts]
        unpacked4 = pickle.loads(pickle.dumps(unpacked3, proto))
        self.assertIs(unpacked3, unpacked4)

    @all_pickle_protocols
    def test_pickling_then_unpickling_tuple_with_typevartuple_equality(
            self, proto
    ):
        global global_T, global_Ts  # See explanation at start of class.
        global_T = TypeVar('global_T')
        global_Ts = TypeVarTuple('global_Ts')

        tuples = [
            tuple[*global_Ts],
            Tuple[Unpack[global_Ts]],

            tuple[T, *global_Ts],
            Tuple[T, Unpack[global_Ts]],

            tuple[int, *global_Ts],
            Tuple[int, Unpack[global_Ts]],
        ]
        for t in tuples:
            t2 = pickle.loads(pickle.dumps(t, proto))
            self.assertEqual(t, t2)


class UnionTests(BaseTestCase):

    def test_basics(self):
        u = Union[int, float]
        self.assertNotEqual(u, Union)

    def test_subclass_error(self):
        with self.assertRaises(TypeError):
            issubclass(int, Union)
        with self.assertRaises(TypeError):
            issubclass(Union, int)
        with self.assertRaises(TypeError):
            issubclass(Union[int, str], int)

    def test_union_any(self):
        u = Union[Any]
        self.assertEqual(u, Any)
        u1 = Union[int, Any]
        u2 = Union[Any, int]
        u3 = Union[Any, object]
        self.assertEqual(u1, u2)
        self.assertNotEqual(u1, Any)
        self.assertNotEqual(u2, Any)
        self.assertNotEqual(u3, Any)

    def test_union_object(self):
        u = Union[object]
        self.assertEqual(u, object)
        u1 = Union[int, object]
        u2 = Union[object, int]
        self.assertEqual(u1, u2)
        self.assertNotEqual(u1, object)
        self.assertNotEqual(u2, object)

    def test_unordered(self):
        u1 = Union[int, float]
        u2 = Union[float, int]
        self.assertEqual(u1, u2)

    def test_single_class_disappears(self):
        t = Union[Employee]
        self.assertIs(t, Employee)

    def test_base_class_kept(self):
        u = Union[Employee, Manager]
        self.assertNotEqual(u, Employee)
        self.assertIn(Employee, u.__args__)
        self.assertIn(Manager, u.__args__)

    def test_union_union(self):
        u = Union[int, float]
        v = Union[u, Employee]
        self.assertEqual(v, Union[int, float, Employee])

    def test_repr(self):
        self.assertEqual(repr(Union), 'typing.Union')
        u = Union[Employee, int]
        self.assertEqual(repr(u), 'typing.Union[%s.Employee, int]' % __name__)
        u = Union[int, Employee]
        self.assertEqual(repr(u), 'typing.Union[int, %s.Employee]' % __name__)
        T = TypeVar('T')
        u = Union[T, int][int]
        self.assertEqual(repr(u), repr(int))
        u = Union[List[int], int]
        self.assertEqual(repr(u), 'typing.Union[typing.List[int], int]')
        u = Union[list[int], dict[str, float]]
        self.assertEqual(repr(u), 'typing.Union[list[int], dict[str, float]]')
        u = Union[int | float]
        self.assertEqual(repr(u), 'typing.Union[int, float]')

        u = Union[None, str]
        self.assertEqual(repr(u), 'typing.Optional[str]')
        u = Union[str, None]
        self.assertEqual(repr(u), 'typing.Optional[str]')
        u = Union[None, str, int]
        self.assertEqual(repr(u), 'typing.Union[NoneType, str, int]')
        u = Optional[str]
        self.assertEqual(repr(u), 'typing.Optional[str]')

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(Union):
                pass
        with self.assertRaises(TypeError):
            class C(type(Union)):
                pass
        with self.assertRaises(TypeError):
            class C(Union[int, str]):
                pass

    def test_cannot_instantiate(self):
        with self.assertRaises(TypeError):
            Union()
        with self.assertRaises(TypeError):
            type(Union)()
        u = Union[int, float]
        with self.assertRaises(TypeError):
            u()
        with self.assertRaises(TypeError):
            type(u)()

    def test_union_generalization(self):
        self.assertFalse(Union[str, typing.Iterable[int]] == str)
        self.assertFalse(Union[str, typing.Iterable[int]] == typing.Iterable[int])
        self.assertIn(str, Union[str, typing.Iterable[int]].__args__)
        self.assertIn(typing.Iterable[int], Union[str, typing.Iterable[int]].__args__)

    def test_union_compare_other(self):
        self.assertNotEqual(Union, object)
        self.assertNotEqual(Union, Any)
        self.assertNotEqual(ClassVar, Union)
        self.assertNotEqual(Optional, Union)
        self.assertNotEqual([None], Optional)
        self.assertNotEqual(Optional, typing.Mapping)
        self.assertNotEqual(Optional[typing.MutableMapping], Union)

    def test_optional(self):
        o = Optional[int]
        u = Union[int, None]
        self.assertEqual(o, u)

    def test_empty(self):
        with self.assertRaises(TypeError):
            Union[()]

    def test_no_eval_union(self):
        u = Union[int, str]
        def f(x: u): ...
        self.assertIs(get_type_hints(f)['x'], u)

    def test_function_repr_union(self):
        def fun() -> int: ...
        self.assertEqual(repr(Union[fun, int]), 'typing.Union[fun, int]')

    def test_union_str_pattern(self):
        # Shouldn't crash; see http://bugs.python.org/issue25390
        A = Union[str, Pattern]
        A

    def test_etree(self):
        # See https://github.com/python/typing/issues/229
        # (Only relevant for Python 2.)
        from xml.etree.ElementTree import Element

        Union[Element, str]  # Shouldn't crash

        def Elem(*args):
            return Element(*args)

        Union[Elem, str]  # Nor should this


class TupleTests(BaseTestCase):

    def test_basics(self):
        with self.assertRaises(TypeError):
            issubclass(Tuple, Tuple[int, str])
        with self.assertRaises(TypeError):
            issubclass(tuple, Tuple[int, str])

        class TP(tuple): ...
        self.assertIsSubclass(tuple, Tuple)
        self.assertIsSubclass(TP, Tuple)

    def test_equality(self):
        self.assertEqual(Tuple[int], Tuple[int])
        self.assertEqual(Tuple[int, ...], Tuple[int, ...])
        self.assertNotEqual(Tuple[int], Tuple[int, int])
        self.assertNotEqual(Tuple[int], Tuple[int, ...])

    def test_tuple_subclass(self):
        class MyTuple(tuple):
            pass
        self.assertIsSubclass(MyTuple, Tuple)
        self.assertIsSubclass(Tuple, Tuple)
        self.assertIsSubclass(tuple, Tuple)

    def test_tuple_instance_type_error(self):
        with self.assertRaises(TypeError):
            isinstance((0, 0), Tuple[int, int])
        self.assertIsInstance((0, 0), Tuple)

    def test_repr(self):
        self.assertEqual(repr(Tuple), 'typing.Tuple')
        self.assertEqual(repr(Tuple[()]), 'typing.Tuple[()]')
        self.assertEqual(repr(Tuple[int, float]), 'typing.Tuple[int, float]')
        self.assertEqual(repr(Tuple[int, ...]), 'typing.Tuple[int, ...]')
        self.assertEqual(repr(Tuple[list[int]]), 'typing.Tuple[list[int]]')

    def test_errors(self):
        with self.assertRaises(TypeError):
            issubclass(42, Tuple)
        with self.assertRaises(TypeError):
            issubclass(42, Tuple[int])


class BaseCallableTests:

    def test_self_subclass(self):
        Callable = self.Callable
        with self.assertRaises(TypeError):
            issubclass(types.FunctionType, Callable[[int], int])
        self.assertIsSubclass(types.FunctionType, Callable)
        self.assertIsSubclass(Callable, Callable)

    def test_eq_hash(self):
        Callable = self.Callable
        C = Callable[[int], int]
        self.assertEqual(C, Callable[[int], int])
        self.assertEqual(len({C, Callable[[int], int]}), 1)
        self.assertNotEqual(C, Callable[[int], str])
        self.assertNotEqual(C, Callable[[str], int])
        self.assertNotEqual(C, Callable[[int, int], int])
        self.assertNotEqual(C, Callable[[], int])
        self.assertNotEqual(C, Callable[..., int])
        self.assertNotEqual(C, Callable)

    def test_cannot_instantiate(self):
        Callable = self.Callable
        with self.assertRaises(TypeError):
            Callable()
        with self.assertRaises(TypeError):
            type(Callable)()
        c = Callable[[int], str]
        with self.assertRaises(TypeError):
            c()
        with self.assertRaises(TypeError):
            type(c)()

    def test_callable_wrong_forms(self):
        Callable = self.Callable
        with self.assertRaises(TypeError):
            Callable[int]

    def test_callable_instance_works(self):
        Callable = self.Callable
        def f():
            pass
        self.assertIsInstance(f, Callable)
        self.assertNotIsInstance(None, Callable)

    def test_callable_instance_type_error(self):
        Callable = self.Callable
        def f():
            pass
        with self.assertRaises(TypeError):
            self.assertIsInstance(f, Callable[[], None])
        with self.assertRaises(TypeError):
            self.assertIsInstance(f, Callable[[], Any])
        with self.assertRaises(TypeError):
            self.assertNotIsInstance(None, Callable[[], None])
        with self.assertRaises(TypeError):
            self.assertNotIsInstance(None, Callable[[], Any])

    def test_repr(self):
        Callable = self.Callable
        fullname = f'{Callable.__module__}.Callable'
        ct0 = Callable[[], bool]
        self.assertEqual(repr(ct0), f'{fullname}[[], bool]')
        ct2 = Callable[[str, float], int]
        self.assertEqual(repr(ct2), f'{fullname}[[str, float], int]')
        ctv = Callable[..., str]
        self.assertEqual(repr(ctv), f'{fullname}[..., str]')
        ct3 = Callable[[str, float], list[int]]
        self.assertEqual(repr(ct3), f'{fullname}[[str, float], list[int]]')

    def test_callable_with_ellipsis(self):
        Callable = self.Callable
        def foo(a: Callable[..., T]):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': Callable[..., T]})

    def test_ellipsis_in_generic(self):
        Callable = self.Callable
        # Shouldn't crash; see https://github.com/python/typing/issues/259
        typing.List[Callable[..., str]]

    def test_or_and_ror(self):
        Callable = self.Callable
        self.assertEqual(Callable | Tuple, Union[Callable, Tuple])
        self.assertEqual(Tuple | Callable, Union[Tuple, Callable])

    def test_basic(self):
        Callable = self.Callable
        alias = Callable[[int, str], float]
        if Callable is collections.abc.Callable:
            self.assertIsInstance(alias, types.GenericAlias)
        self.assertIs(alias.__origin__, collections.abc.Callable)
        self.assertEqual(alias.__args__, (int, str, float))
        self.assertEqual(alias.__parameters__, ())

    def test_weakref(self):
        Callable = self.Callable
        alias = Callable[[int, str], float]
        self.assertEqual(weakref.ref(alias)(), alias)

    def test_pickle(self):
        Callable = self.Callable
        alias = Callable[[int, str], float]
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            s = pickle.dumps(alias, proto)
            loaded = pickle.loads(s)
            self.assertEqual(alias.__origin__, loaded.__origin__)
            self.assertEqual(alias.__args__, loaded.__args__)
            self.assertEqual(alias.__parameters__, loaded.__parameters__)

    def test_var_substitution(self):
        Callable = self.Callable
        fullname = f"{Callable.__module__}.Callable"
        C1 = Callable[[int, T], T]
        C2 = Callable[[KT, T], VT]
        C3 = Callable[..., T]
        self.assertEqual(C1[str], Callable[[int, str], str])
        self.assertEqual(C1[None], Callable[[int, type(None)], type(None)])
        self.assertEqual(C2[int, float, str], Callable[[int, float], str])
        self.assertEqual(C3[int], Callable[..., int])
        self.assertEqual(C3[NoReturn], Callable[..., NoReturn])

        # multi chaining
        C4 = C2[int, VT, str]
        self.assertEqual(repr(C4), f"{fullname}[[int, ~VT], str]")
        self.assertEqual(repr(C4[dict]), f"{fullname}[[int, dict], str]")
        self.assertEqual(C4[dict], Callable[[int, dict], str])

        # substitute a nested GenericAlias (both typing and the builtin
        # version)
        C5 = Callable[[typing.List[T], tuple[KT, T], VT], int]
        self.assertEqual(C5[int, str, float],
                         Callable[[typing.List[int], tuple[str, int], float], int])

    def test_type_erasure(self):
        Callable = self.Callable
        class C1(Callable):
            def __call__(self):
                return None
        a = C1[[int], T]
        self.assertIs(a().__class__, C1)
        self.assertEqual(a().__orig_class__, C1[[int], T])

    def test_paramspec(self):
        Callable = self.Callable
        fullname = f"{Callable.__module__}.Callable"
        P = ParamSpec('P')
        P2 = ParamSpec('P2')
        C1 = Callable[P, T]
        # substitution
        self.assertEqual(C1[[int], str], Callable[[int], str])
        self.assertEqual(C1[[int, str], str], Callable[[int, str], str])
        self.assertEqual(C1[[], str], Callable[[], str])
        self.assertEqual(C1[..., str], Callable[..., str])
        self.assertEqual(C1[P2, str], Callable[P2, str])
        self.assertEqual(C1[Concatenate[int, P2], str],
                         Callable[Concatenate[int, P2], str])
        self.assertEqual(repr(C1), f"{fullname}[~P, ~T]")
        self.assertEqual(repr(C1[[int, str], str]), f"{fullname}[[int, str], str]")
        with self.assertRaises(TypeError):
            C1[int, str]

        C2 = Callable[P, int]
        self.assertEqual(C2[[int]], Callable[[int], int])
        self.assertEqual(C2[[int, str]], Callable[[int, str], int])
        self.assertEqual(C2[[]], Callable[[], int])
        self.assertEqual(C2[...], Callable[..., int])
        self.assertEqual(C2[P2], Callable[P2, int])
        self.assertEqual(C2[Concatenate[int, P2]],
                         Callable[Concatenate[int, P2], int])
        # special case in PEP 612 where
        # X[int, str, float] == X[[int, str, float]]
        self.assertEqual(C2[int], Callable[[int], int])
        self.assertEqual(C2[int, str], Callable[[int, str], int])
        self.assertEqual(repr(C2), f"{fullname}[~P, int]")
        self.assertEqual(repr(C2[int, str]), f"{fullname}[[int, str], int]")

    def test_concatenate(self):
        Callable = self.Callable
        fullname = f"{Callable.__module__}.Callable"
        T = TypeVar('T')
        P = ParamSpec('P')
        P2 = ParamSpec('P2')
        C = Callable[Concatenate[int, P], T]
        self.assertEqual(repr(C),
                         f"{fullname}[typing.Concatenate[int, ~P], ~T]")
        self.assertEqual(C[P2, int], Callable[Concatenate[int, P2], int])
        self.assertEqual(C[[str, float], int], Callable[[int, str, float], int])
        self.assertEqual(C[[], int], Callable[[int], int])
        self.assertEqual(C[Concatenate[str, P2], int],
                         Callable[Concatenate[int, str, P2], int])
        self.assertEqual(C[..., int], Callable[Concatenate[int, ...], int])

        C = Callable[Concatenate[int, P], int]
        self.assertEqual(repr(C),
                         f"{fullname}[typing.Concatenate[int, ~P], int]")
        self.assertEqual(C[P2], Callable[Concatenate[int, P2], int])
        self.assertEqual(C[[str, float]], Callable[[int, str, float], int])
        self.assertEqual(C[str, float], Callable[[int, str, float], int])
        self.assertEqual(C[[]], Callable[[int], int])
        self.assertEqual(C[Concatenate[str, P2]],
                         Callable[Concatenate[int, str, P2], int])
        self.assertEqual(C[...], Callable[Concatenate[int, ...], int])

    def test_errors(self):
        Callable = self.Callable
        alias = Callable[[int, str], float]
        with self.assertRaisesRegex(TypeError, "is not a generic class"):
            alias[int]
        P = ParamSpec('P')
        C1 = Callable[P, T]
        with self.assertRaisesRegex(TypeError, "many arguments for"):
            C1[int, str, str]
        with self.assertRaisesRegex(TypeError, "few arguments for"):
            C1[int]

class TypingCallableTests(BaseCallableTests, BaseTestCase):
    Callable = typing.Callable

    def test_consistency(self):
        # bpo-42195
        # Testing collections.abc.Callable's consistency with typing.Callable
        c1 = typing.Callable[[int, str], dict]
        c2 = collections.abc.Callable[[int, str], dict]
        self.assertEqual(c1.__args__, c2.__args__)
        self.assertEqual(hash(c1.__args__), hash(c2.__args__))


class CollectionsCallableTests(BaseCallableTests, BaseTestCase):
    Callable = collections.abc.Callable
    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_errors(self): # TODO: RUSTPYTHON, remove when this passes
        super().test_errors() # TODO: RUSTPYTHON, remove when this passes

    # TODO: RUSTPYTHON, AssertionError: 'collections.abc.Callable[__main__.ParamSpec, typing.TypeVar]' != 'collections.abc.Callable[~P, ~T]'
    @unittest.expectedFailure
    def test_paramspec(self): # TODO: RUSTPYTHON, remove when this passes
        super().test_paramspec() # TODO: RUSTPYTHON, remove when this passes

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_concatenate(self):  # TODO: RUSTPYTHON, remove when this passes
        super().test_concatenate()  # TODO: RUSTPYTHON, remove when this passes

    # TODO: RUSTPYTHON might be fixed by updating typing to 3.12
    @unittest.expectedFailure
    def test_repr(self):  # TODO: RUSTPYTHON, remove when this passes
        super().test_repr()  # TODO: RUSTPYTHON, remove when this passes


class LiteralTests(BaseTestCase):
    def test_basics(self):
        # All of these are allowed.
        Literal[1]
        Literal[1, 2, 3]
        Literal["x", "y", "z"]
        Literal[None]
        Literal[True]
        Literal[1, "2", False]
        Literal[Literal[1, 2], Literal[4, 5]]
        Literal[b"foo", u"bar"]

    def test_illegal_parameters_do_not_raise_runtime_errors(self):
        # Type checkers should reject these types, but we do not
        # raise errors at runtime to maintain maximum flexibility.
        Literal[int]
        Literal[3j + 2, ..., ()]
        Literal[{"foo": 3, "bar": 4}]
        Literal[T]

    def test_literals_inside_other_types(self):
        List[Literal[1, 2, 3]]
        List[Literal[("foo", "bar", "baz")]]

    def test_repr(self):
        self.assertEqual(repr(Literal[1]), "typing.Literal[1]")
        self.assertEqual(repr(Literal[1, True, "foo"]), "typing.Literal[1, True, 'foo']")
        self.assertEqual(repr(Literal[int]), "typing.Literal[int]")
        self.assertEqual(repr(Literal), "typing.Literal")
        self.assertEqual(repr(Literal[None]), "typing.Literal[None]")
        self.assertEqual(repr(Literal[1, 2, 3, 3]), "typing.Literal[1, 2, 3]")

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            Literal()
        with self.assertRaises(TypeError):
            Literal[1]()
        with self.assertRaises(TypeError):
            type(Literal)()
        with self.assertRaises(TypeError):
            type(Literal[1])()

    def test_no_isinstance_or_issubclass(self):
        with self.assertRaises(TypeError):
            isinstance(1, Literal[1])
        with self.assertRaises(TypeError):
            isinstance(int, Literal[1])
        with self.assertRaises(TypeError):
            issubclass(1, Literal[1])
        with self.assertRaises(TypeError):
            issubclass(int, Literal[1])

    def test_no_subclassing(self):
        with self.assertRaises(TypeError):
            class Foo(Literal[1]): pass
        with self.assertRaises(TypeError):
            class Bar(Literal): pass

    def test_no_multiple_subscripts(self):
        with self.assertRaises(TypeError):
            Literal[1][1]

    def test_equal(self):
        self.assertNotEqual(Literal[0], Literal[False])
        self.assertNotEqual(Literal[True], Literal[1])
        self.assertNotEqual(Literal[1], Literal[2])
        self.assertNotEqual(Literal[1, True], Literal[1])
        self.assertNotEqual(Literal[1, True], Literal[1, 1])
        self.assertNotEqual(Literal[1, 2], Literal[True, 2])
        self.assertEqual(Literal[1], Literal[1])
        self.assertEqual(Literal[1, 2], Literal[2, 1])
        self.assertEqual(Literal[1, 2, 3], Literal[1, 2, 3, 3])

    def test_hash(self):
        self.assertEqual(hash(Literal[1]), hash(Literal[1]))
        self.assertEqual(hash(Literal[1, 2]), hash(Literal[2, 1]))
        self.assertEqual(hash(Literal[1, 2, 3]), hash(Literal[1, 2, 3, 3]))

    def test_args(self):
        self.assertEqual(Literal[1, 2, 3].__args__, (1, 2, 3))
        self.assertEqual(Literal[1, 2, 3, 3].__args__, (1, 2, 3))
        self.assertEqual(Literal[1, Literal[2], Literal[3, 4]].__args__, (1, 2, 3, 4))
        # Mutable arguments will not be deduplicated
        self.assertEqual(Literal[[], []].__args__, ([], []))

    def test_flatten(self):
        l1 = Literal[Literal[1], Literal[2], Literal[3]]
        l2 = Literal[Literal[1, 2], 3]
        l3 = Literal[Literal[1, 2, 3]]
        for l in l1, l2, l3:
            self.assertEqual(l, Literal[1, 2, 3])
            self.assertEqual(l.__args__, (1, 2, 3))


XK = TypeVar('XK', str, bytes)
XV = TypeVar('XV')


class SimpleMapping(Generic[XK, XV]):

    def __getitem__(self, key: XK) -> XV:
        ...

    def __setitem__(self, key: XK, value: XV):
        ...

    def get(self, key: XK, default: XV = None) -> XV:
        ...


class MySimpleMapping(SimpleMapping[XK, XV]):

    def __init__(self):
        self.store = {}

    def __getitem__(self, key: str):
        return self.store[key]

    def __setitem__(self, key: str, value):
        self.store[key] = value

    def get(self, key: str, default=None):
        try:
            return self.store[key]
        except KeyError:
            return default


class Coordinate(Protocol):
    x: int
    y: int

@runtime_checkable
class Point(Coordinate, Protocol):
    label: str

class MyPoint:
    x: int
    y: int
    label: str

class XAxis(Protocol):
    x: int

class YAxis(Protocol):
    y: int

@runtime_checkable
class Position(XAxis, YAxis, Protocol):
    pass

@runtime_checkable
class Proto(Protocol):
    attr: int
    def meth(self, arg: str) -> int:
        ...

class Concrete(Proto):
    pass

class Other:
    attr: int = 1
    def meth(self, arg: str) -> int:
        if arg == 'this':
            return 1
        return 0

class NT(NamedTuple):
    x: int
    y: int

@runtime_checkable
class HasCallProtocol(Protocol):
    __call__: typing.Callable


class ProtocolTests(BaseTestCase):
    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_basic_protocol(self):
        @runtime_checkable
        class P(Protocol):
            def meth(self):
                pass

        class C: pass

        class D:
            def meth(self):
                pass

        def f():
            pass

        self.assertIsSubclass(D, P)
        self.assertIsInstance(D(), P)
        self.assertNotIsSubclass(C, P)
        self.assertNotIsInstance(C(), P)
        self.assertNotIsSubclass(types.FunctionType, P)
        self.assertNotIsInstance(f, P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_everything_implements_empty_protocol(self):
        @runtime_checkable
        class Empty(Protocol):
            pass

        class C:
            pass

        def f():
            pass

        for thing in (object, type, tuple, C, types.FunctionType):
            self.assertIsSubclass(thing, Empty)
        for thing in (object(), 1, (), typing, f):
            self.assertIsInstance(thing, Empty)

    def test_function_implements_protocol(self):
        def f():
            pass

        self.assertIsInstance(f, HasCallProtocol)

    def test_no_inheritance_from_nominal(self):
        class C: pass

        class BP(Protocol): pass

        with self.assertRaises(TypeError):
            class P(C, Protocol):
                pass
        with self.assertRaises(TypeError):
            class P(Protocol, C):
                pass
        with self.assertRaises(TypeError):
            class P(BP, C, Protocol):
                pass

        class D(BP, C): pass

        class E(C, BP): pass

        self.assertNotIsInstance(D(), E)
        self.assertNotIsInstance(E(), D)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_no_instantiation(self):
        class P(Protocol): pass

        with self.assertRaises(TypeError):
            P()

        class C(P): pass

        self.assertIsInstance(C(), C)
        with self.assertRaises(TypeError):
            C(42)

        T = TypeVar('T')

        class PG(Protocol[T]): pass

        with self.assertRaises(TypeError):
            PG()
        with self.assertRaises(TypeError):
            PG[int]()
        with self.assertRaises(TypeError):
            PG[T]()

        class CG(PG[T]): pass

        self.assertIsInstance(CG[int](), CG)
        with self.assertRaises(TypeError):
            CG[int](42)

    def test_protocol_defining_init_does_not_get_overridden(self):
        # check that P.__init__ doesn't get clobbered
        # see https://bugs.python.org/issue44807

        class P(Protocol):
            x: int
            def __init__(self, x: int) -> None:
                self.x = x
        class C: pass

        c = C()
        P.__init__(c, 1)
        self.assertEqual(c.x, 1)

    def test_concrete_class_inheriting_init_from_protocol(self):
        class P(Protocol):
            x: int
            def __init__(self, x: int) -> None:
                self.x = x

        class C(P): pass

        c = C(1)
        self.assertIsInstance(c, C)
        self.assertEqual(c.x, 1)

    def test_cannot_instantiate_abstract(self):
        @runtime_checkable
        class P(Protocol):
            @abc.abstractmethod
            def ameth(self) -> int:
                raise NotImplementedError

        class B(P):
            pass

        class C(B):
            def ameth(self) -> int:
                return 26

        with self.assertRaises(TypeError):
            B()
        self.assertIsInstance(C(), P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_subprotocols_extending(self):
        class P1(Protocol):
            def meth1(self):
                pass

        @runtime_checkable
        class P2(P1, Protocol):
            def meth2(self):
                pass

        class C:
            def meth1(self):
                pass

            def meth2(self):
                pass

        class C1:
            def meth1(self):
                pass

        class C2:
            def meth2(self):
                pass

        self.assertNotIsInstance(C1(), P2)
        self.assertNotIsInstance(C2(), P2)
        self.assertNotIsSubclass(C1, P2)
        self.assertNotIsSubclass(C2, P2)
        self.assertIsInstance(C(), P2)
        self.assertIsSubclass(C, P2)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_subprotocols_merging(self):
        class P1(Protocol):
            def meth1(self):
                pass

        class P2(Protocol):
            def meth2(self):
                pass

        @runtime_checkable
        class P(P1, P2, Protocol):
            pass

        class C:
            def meth1(self):
                pass

            def meth2(self):
                pass

        class C1:
            def meth1(self):
                pass

        class C2:
            def meth2(self):
                pass

        self.assertNotIsInstance(C1(), P)
        self.assertNotIsInstance(C2(), P)
        self.assertNotIsSubclass(C1, P)
        self.assertNotIsSubclass(C2, P)
        self.assertIsInstance(C(), P)
        self.assertIsSubclass(C, P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_protocols_issubclass(self):
        T = TypeVar('T')

        @runtime_checkable
        class P(Protocol):
            def x(self): ...

        @runtime_checkable
        class PG(Protocol[T]):
            def x(self): ...

        class BadP(Protocol):
            def x(self): ...

        class BadPG(Protocol[T]):
            def x(self): ...

        class C:
            def x(self): ...

        self.assertIsSubclass(C, P)
        self.assertIsSubclass(C, PG)
        self.assertIsSubclass(BadP, PG)

        with self.assertRaises(TypeError):
            issubclass(C, PG[T])
        with self.assertRaises(TypeError):
            issubclass(C, PG[C])
        with self.assertRaises(TypeError):
            issubclass(C, BadP)
        with self.assertRaises(TypeError):
            issubclass(C, BadPG)
        with self.assertRaises(TypeError):
            issubclass(P, PG[T])
        with self.assertRaises(TypeError):
            issubclass(PG, PG[int])

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_protocols_issubclass_non_callable(self):
        class C:
            x = 1

        @runtime_checkable
        class PNonCall(Protocol):
            x = 1

        with self.assertRaises(TypeError):
            issubclass(C, PNonCall)
        self.assertIsInstance(C(), PNonCall)
        PNonCall.register(C)
        with self.assertRaises(TypeError):
            issubclass(C, PNonCall)
        self.assertIsInstance(C(), PNonCall)

        # check that non-protocol subclasses are not affected
        class D(PNonCall): ...

        self.assertNotIsSubclass(C, D)
        self.assertNotIsInstance(C(), D)
        D.register(C)
        self.assertIsSubclass(C, D)
        self.assertIsInstance(C(), D)
        with self.assertRaises(TypeError):
            issubclass(D, PNonCall)

    def test_protocols_isinstance(self):
        T = TypeVar('T')

        @runtime_checkable
        class P(Protocol):
            def meth(x): ...

        @runtime_checkable
        class PG(Protocol[T]):
            def meth(x): ...

        class BadP(Protocol):
            def meth(x): ...

        class BadPG(Protocol[T]):
            def meth(x): ...

        class C:
            def meth(x): ...

        self.assertIsInstance(C(), P)
        self.assertIsInstance(C(), PG)
        with self.assertRaises(TypeError):
            isinstance(C(), PG[T])
        with self.assertRaises(TypeError):
            isinstance(C(), PG[C])
        with self.assertRaises(TypeError):
            isinstance(C(), BadP)
        with self.assertRaises(TypeError):
            isinstance(C(), BadPG)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_protocols_isinstance_py36(self):
        class APoint:
            def __init__(self, x, y, label):
                self.x = x
                self.y = y
                self.label = label

        class BPoint:
            label = 'B'

            def __init__(self, x, y):
                self.x = x
                self.y = y

        class C:
            def __init__(self, attr):
                self.attr = attr

            def meth(self, arg):
                return 0

        class Bad: pass

        self.assertIsInstance(APoint(1, 2, 'A'), Point)
        self.assertIsInstance(BPoint(1, 2), Point)
        self.assertNotIsInstance(MyPoint(), Point)
        self.assertIsInstance(BPoint(1, 2), Position)
        self.assertIsInstance(Other(), Proto)
        self.assertIsInstance(Concrete(), Proto)
        self.assertIsInstance(C(42), Proto)
        self.assertNotIsInstance(Bad(), Proto)
        self.assertNotIsInstance(Bad(), Point)
        self.assertNotIsInstance(Bad(), Position)
        self.assertNotIsInstance(Bad(), Concrete)
        self.assertNotIsInstance(Other(), Concrete)
        self.assertIsInstance(NT(1, 2), Position)

    def test_protocols_isinstance_init(self):
        T = TypeVar('T')

        @runtime_checkable
        class P(Protocol):
            x = 1

        @runtime_checkable
        class PG(Protocol[T]):
            x = 1

        class C:
            def __init__(self, x):
                self.x = x

        self.assertIsInstance(C(1), P)
        self.assertIsInstance(C(1), PG)

    def test_protocol_checks_after_subscript(self):
        class P(Protocol[T]): pass
        class C(P[T]): pass
        class Other1: pass
        class Other2: pass
        CA = C[Any]

        self.assertNotIsInstance(Other1(), C)
        self.assertNotIsSubclass(Other2, C)

        class D1(C[Any]): pass
        class D2(C[Any]): pass
        CI = C[int]

        self.assertIsInstance(D1(), C)
        self.assertIsSubclass(D2, C)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_protocols_support_register(self):
        @runtime_checkable
        class P(Protocol):
            x = 1

        class PM(Protocol):
            def meth(self): pass

        class D(PM): pass

        class C: pass

        D.register(C)
        P.register(C)
        self.assertIsInstance(C(), P)
        self.assertIsInstance(C(), D)

    def test_none_on_non_callable_doesnt_block_implementation(self):
        @runtime_checkable
        class P(Protocol):
            x = 1

        class A:
            x = 1

        class B(A):
            x = None

        class C:
            def __init__(self):
                self.x = None

        self.assertIsInstance(B(), P)
        self.assertIsInstance(C(), P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_none_on_callable_blocks_implementation(self):
        @runtime_checkable
        class P(Protocol):
            def x(self): ...

        class A:
            def x(self): ...

        class B(A):
            x = None

        class C:
            def __init__(self):
                self.x = None

        self.assertNotIsInstance(B(), P)
        self.assertNotIsInstance(C(), P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_non_protocol_subclasses(self):
        class P(Protocol):
            x = 1

        @runtime_checkable
        class PR(Protocol):
            def meth(self): pass

        class NonP(P):
            x = 1

        class NonPR(PR): pass

        class C:
            x = 1

        class D:
            def meth(self): pass

        self.assertNotIsInstance(C(), NonP)
        self.assertNotIsInstance(D(), NonPR)
        self.assertNotIsSubclass(C, NonP)
        self.assertNotIsSubclass(D, NonPR)
        self.assertIsInstance(NonPR(), PR)
        self.assertIsSubclass(NonPR, PR)

    def test_custom_subclasshook(self):
        class P(Protocol):
            x = 1

        class OKClass: pass

        class BadClass:
            x = 1

        class C(P):
            @classmethod
            def __subclasshook__(cls, other):
                return other.__name__.startswith("OK")

        self.assertIsInstance(OKClass(), C)
        self.assertNotIsInstance(BadClass(), C)
        self.assertIsSubclass(OKClass, C)
        self.assertNotIsSubclass(BadClass, C)

    def test_issubclass_fails_correctly(self):
        @runtime_checkable
        class P(Protocol):
            x = 1

        class C: pass

        with self.assertRaises(TypeError):
            issubclass(C(), P)

    def test_defining_generic_protocols(self):
        T = TypeVar('T')
        S = TypeVar('S')

        @runtime_checkable
        class PR(Protocol[T, S]):
            def meth(self): pass

        class P(PR[int, T], Protocol[T]):
            y = 1

        with self.assertRaises(TypeError):
            PR[int]
        with self.assertRaises(TypeError):
            P[int, str]

        class C(PR[int, T]): pass

        self.assertIsInstance(C[str](), C)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_defining_generic_protocols_old_style(self):
        T = TypeVar('T')
        S = TypeVar('S')

        @runtime_checkable
        class PR(Protocol, Generic[T, S]):
            def meth(self): pass

        class P(PR[int, str], Protocol):
            y = 1

        with self.assertRaises(TypeError):
            issubclass(PR[int, str], PR)
        self.assertIsSubclass(P, PR)
        with self.assertRaises(TypeError):
            PR[int]

        class P1(Protocol, Generic[T]):
            def bar(self, x: T) -> str: ...

        class P2(Generic[T], Protocol):
            def bar(self, x: T) -> str: ...

        @runtime_checkable
        class PSub(P1[str], Protocol):
            x = 1

        class Test:
            x = 1

            def bar(self, x: str) -> str:
                return x

        self.assertIsInstance(Test(), PSub)

    def test_init_called(self):
        T = TypeVar('T')

        class P(Protocol[T]): pass

        class C(P[T]):
            def __init__(self):
                self.test = 'OK'

        self.assertEqual(C[int]().test, 'OK')

        class B:
            def __init__(self):
                self.test = 'OK'

        class D1(B, P[T]):
            pass

        self.assertEqual(D1[int]().test, 'OK')

        class D2(P[T], B):
            pass

        self.assertEqual(D2[int]().test, 'OK')

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_new_called(self):
        T = TypeVar('T')

        class P(Protocol[T]): pass

        class C(P[T]):
            def __new__(cls, *args):
                self = super().__new__(cls, *args)
                self.test = 'OK'
                return self

        self.assertEqual(C[int]().test, 'OK')
        with self.assertRaises(TypeError):
            C[int](42)
        with self.assertRaises(TypeError):
            C[int](a=42)

    # TODO: RUSTPYTHON the last line breaks any tests that use unittest.mock
    # See https://github.com/RustPython/RustPython/issues/5190#issuecomment-2010535802
    # It's possible that updating typing to 3.12 will resolve this
    @unittest.skip("TODO: RUSTPYTHON this test breaks other tests that use unittest.mock")
    def test_protocols_bad_subscripts(self):
        T = TypeVar('T')
        S = TypeVar('S')
        with self.assertRaises(TypeError):
            class P(Protocol[T, T]): pass
        with self.assertRaises(TypeError):
            class P(Protocol[int]): pass
        with self.assertRaises(TypeError):
            class P(Protocol[T], Protocol[S]): pass
        with self.assertRaises(TypeError):
            class P(typing.Mapping[T, S], Protocol[T]): pass

    def test_generic_protocols_repr(self):
        T = TypeVar('T')
        S = TypeVar('S')

        class P(Protocol[T, S]): pass

        self.assertTrue(repr(P[T, S]).endswith('P[~T, ~S]'))
        self.assertTrue(repr(P[int, str]).endswith('P[int, str]'))

    def test_generic_protocols_eq(self):
        T = TypeVar('T')
        S = TypeVar('S')

        class P(Protocol[T, S]): pass

        self.assertEqual(P, P)
        self.assertEqual(P[int, T], P[int, T])
        self.assertEqual(P[T, T][Tuple[T, S]][int, str],
                         P[Tuple[int, str], Tuple[int, str]])

    def test_generic_protocols_special_from_generic(self):
        T = TypeVar('T')

        class P(Protocol[T]): pass

        self.assertEqual(P.__parameters__, (T,))
        self.assertEqual(P[int].__parameters__, ())
        self.assertEqual(P[int].__args__, (int,))
        self.assertIs(P[int].__origin__, P)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_generic_protocols_special_from_protocol(self):
        @runtime_checkable
        class PR(Protocol):
            x = 1

        class P(Protocol):
            def meth(self):
                pass

        T = TypeVar('T')

        class PG(Protocol[T]):
            x = 1

            def meth(self):
                pass

        self.assertTrue(P._is_protocol)
        self.assertTrue(PR._is_protocol)
        self.assertTrue(PG._is_protocol)
        self.assertFalse(P._is_runtime_protocol)
        self.assertTrue(PR._is_runtime_protocol)
        self.assertTrue(PG[int]._is_protocol)
        self.assertEqual(typing._get_protocol_attrs(P), {'meth'})
        self.assertEqual(typing._get_protocol_attrs(PR), {'x'})
        self.assertEqual(frozenset(typing._get_protocol_attrs(PG)),
                         frozenset({'x', 'meth'}))

    def test_no_runtime_deco_on_nominal(self):
        with self.assertRaises(TypeError):
            @runtime_checkable
            class C: pass

        class Proto(Protocol):
            x = 1

        with self.assertRaises(TypeError):
            @runtime_checkable
            class Concrete(Proto):
                pass

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_none_treated_correctly(self):
        @runtime_checkable
        class P(Protocol):
            x = None  # type: int

        class B(object): pass

        self.assertNotIsInstance(B(), P)

        class C:
            x = 1

        class D:
            x = None

        self.assertIsInstance(C(), P)
        self.assertIsInstance(D(), P)

        class CI:
            def __init__(self):
                self.x = 1

        class DI:
            def __init__(self):
                self.x = None

        self.assertIsInstance(C(), P)
        self.assertIsInstance(D(), P)

    def test_protocols_in_unions(self):
        class P(Protocol):
            x = None  # type: int

        Alias = typing.Union[typing.Iterable, P]
        Alias2 = typing.Union[P, typing.Iterable]
        self.assertEqual(Alias, Alias2)

    def test_protocols_pickleable(self):
        global P, CP  # pickle wants to reference the class by name
        T = TypeVar('T')

        @runtime_checkable
        class P(Protocol[T]):
            x = 1

        class CP(P[int]):
            pass

        c = CP()
        c.foo = 42
        c.bar = 'abc'
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            z = pickle.dumps(c, proto)
            x = pickle.loads(z)
            self.assertEqual(x.foo, 42)
            self.assertEqual(x.bar, 'abc')
            self.assertEqual(x.x, 1)
            self.assertEqual(x.__dict__, {'foo': 42, 'bar': 'abc'})
            s = pickle.dumps(P, proto)
            D = pickle.loads(s)

            class E:
                x = 1

            self.assertIsInstance(E(), D)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_int(self):
        self.assertIsSubclass(int, typing.SupportsInt)
        self.assertNotIsSubclass(str, typing.SupportsInt)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_float(self):
        self.assertIsSubclass(float, typing.SupportsFloat)
        self.assertNotIsSubclass(str, typing.SupportsFloat)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_complex(self):

        class C:
            def __complex__(self):
                return 0j

        self.assertIsSubclass(complex, typing.SupportsComplex)
        self.assertIsSubclass(C, typing.SupportsComplex)
        self.assertNotIsSubclass(str, typing.SupportsComplex)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_bytes(self):

        class B:
            def __bytes__(self):
                return b''

        self.assertIsSubclass(bytes, typing.SupportsBytes)
        self.assertIsSubclass(B, typing.SupportsBytes)
        self.assertNotIsSubclass(str, typing.SupportsBytes)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_abs(self):
        self.assertIsSubclass(float, typing.SupportsAbs)
        self.assertIsSubclass(int, typing.SupportsAbs)
        self.assertNotIsSubclass(str, typing.SupportsAbs)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_round(self):
        issubclass(float, typing.SupportsRound)
        self.assertIsSubclass(float, typing.SupportsRound)
        self.assertIsSubclass(int, typing.SupportsRound)
        self.assertNotIsSubclass(str, typing.SupportsRound)

    def test_reversible(self):
        self.assertIsSubclass(list, typing.Reversible)
        self.assertNotIsSubclass(int, typing.Reversible)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_supports_index(self):
        self.assertIsSubclass(int, typing.SupportsIndex)
        self.assertNotIsSubclass(str, typing.SupportsIndex)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_bundled_protocol_instance_works(self):
        self.assertIsInstance(0, typing.SupportsAbs)
        class C1(typing.SupportsInt):
            def __int__(self) -> int:
                return 42
        class C2(C1):
            pass
        c = C2()
        self.assertIsInstance(c, C1)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_collections_protocols_allowed(self):
        @runtime_checkable
        class Custom(collections.abc.Iterable, Protocol):
            def close(self): ...

        class A: pass
        class B:
            def __iter__(self):
                return []
            def close(self):
                return 0

        self.assertIsSubclass(B, Custom)
        self.assertNotIsSubclass(A, Custom)

    def test_builtin_protocol_allowlist(self):
        with self.assertRaises(TypeError):
            class CustomProtocol(TestCase, Protocol):
                pass

        class CustomContextManager(typing.ContextManager, Protocol):
            pass

    def test_non_runtime_protocol_isinstance_check(self):
        class P(Protocol):
            x: int

        with self.assertRaisesRegex(TypeError, "@runtime_checkable"):
            isinstance(1, P)

    def test_super_call_init(self):
        class P(Protocol):
            x: int

        class Foo(P):
            def __init__(self):
                super().__init__()

        Foo()  # Previously triggered RecursionError


class GenericTests(BaseTestCase):

    def test_basics(self):
        X = SimpleMapping[str, Any]
        self.assertEqual(X.__parameters__, ())
        with self.assertRaises(TypeError):
            X[str]
        with self.assertRaises(TypeError):
            X[str, str]
        Y = SimpleMapping[XK, str]
        self.assertEqual(Y.__parameters__, (XK,))
        Y[str]
        with self.assertRaises(TypeError):
            Y[str, str]
        SM1 = SimpleMapping[str, int]
        with self.assertRaises(TypeError):
            issubclass(SM1, SimpleMapping)
        self.assertIsInstance(SM1(), SimpleMapping)
        T = TypeVar("T")
        self.assertEqual(List[list[T] | float].__parameters__, (T,))

    def test_generic_errors(self):
        T = TypeVar('T')
        S = TypeVar('S')
        with self.assertRaises(TypeError):
            Generic[T][T]
        with self.assertRaises(TypeError):
            Generic[T][S]
        with self.assertRaises(TypeError):
            class C(Generic[T], Generic[T]): ...
        with self.assertRaises(TypeError):
            isinstance([], List[int])
        with self.assertRaises(TypeError):
            issubclass(list, List[int])
        with self.assertRaises(TypeError):
            class NewGeneric(Generic): ...
        with self.assertRaises(TypeError):
            class MyGeneric(Generic[T], Generic[S]): ...
        with self.assertRaises(TypeError):
            class MyGeneric(List[T], Generic[S]): ...
        with self.assertRaises(TypeError):
            Generic[()]
        class C(Generic[T]): pass
        with self.assertRaises(TypeError):
            C[()]

    def test_init(self):
        T = TypeVar('T')
        S = TypeVar('S')
        with self.assertRaises(TypeError):
            Generic[T, T]
        with self.assertRaises(TypeError):
            Generic[T, S, T]

    def test_init_subclass(self):
        class X(typing.Generic[T]):
            def __init_subclass__(cls, **kwargs):
                super().__init_subclass__(**kwargs)
                cls.attr = 42
        class Y(X):
            pass
        self.assertEqual(Y.attr, 42)
        with self.assertRaises(AttributeError):
            X.attr
        X.attr = 1
        Y.attr = 2
        class Z(Y):
            pass
        class W(X[int]):
            pass
        self.assertEqual(Y.attr, 2)
        self.assertEqual(Z.attr, 42)
        self.assertEqual(W.attr, 42)

    def test_repr(self):
        self.assertEqual(repr(SimpleMapping),
                         f"<class '{__name__}.SimpleMapping'>")
        self.assertEqual(repr(MySimpleMapping),
                         f"<class '{__name__}.MySimpleMapping'>")

    def test_chain_repr(self):
        T = TypeVar('T')
        S = TypeVar('S')

        class C(Generic[T]):
            pass

        X = C[Tuple[S, T]]
        self.assertEqual(X, C[Tuple[S, T]])
        self.assertNotEqual(X, C[Tuple[T, S]])

        Y = X[T, int]
        self.assertEqual(Y, X[T, int])
        self.assertNotEqual(Y, X[S, int])
        self.assertNotEqual(Y, X[T, str])

        Z = Y[str]
        self.assertEqual(Z, Y[str])
        self.assertNotEqual(Z, Y[int])
        self.assertNotEqual(Z, Y[T])

        self.assertTrue(str(Z).endswith(
            '.C[typing.Tuple[str, int]]'))

    def test_new_repr(self):
        T = TypeVar('T')
        U = TypeVar('U', covariant=True)
        S = TypeVar('S')

        self.assertEqual(repr(List), 'typing.List')
        self.assertEqual(repr(List[T]), 'typing.List[~T]')
        self.assertEqual(repr(List[U]), 'typing.List[+U]')
        self.assertEqual(repr(List[S][T][int]), 'typing.List[int]')
        self.assertEqual(repr(List[int]), 'typing.List[int]')

    def test_new_repr_complex(self):
        T = TypeVar('T')
        TS = TypeVar('TS')

        self.assertEqual(repr(typing.Mapping[T, TS][TS, T]), 'typing.Mapping[~TS, ~T]')
        self.assertEqual(repr(List[Tuple[T, TS]][int, T]),
                         'typing.List[typing.Tuple[int, ~T]]')
        self.assertEqual(
            repr(List[Tuple[T, T]][List[int]]),
            'typing.List[typing.Tuple[typing.List[int], typing.List[int]]]'
        )

    def test_new_repr_bare(self):
        T = TypeVar('T')
        self.assertEqual(repr(Generic[T]), 'typing.Generic[~T]')
        self.assertEqual(repr(typing.Protocol[T]), 'typing.Protocol[~T]')
        class C(typing.Dict[Any, Any]): ...
        # this line should just work
        repr(C.__mro__)

    def test_dict(self):
        T = TypeVar('T')

        class B(Generic[T]):
            pass

        b = B()
        b.foo = 42
        self.assertEqual(b.__dict__, {'foo': 42})

        class C(B[int]):
            pass

        c = C()
        c.bar = 'abc'
        self.assertEqual(c.__dict__, {'bar': 'abc'})

    def test_subscripted_generics_as_proxies(self):
        T = TypeVar('T')
        class C(Generic[T]):
            x = 'def'
        self.assertEqual(C[int].x, 'def')
        self.assertEqual(C[C[int]].x, 'def')
        C[C[int]].x = 'changed'
        self.assertEqual(C.x, 'changed')
        self.assertEqual(C[str].x, 'changed')
        C[List[str]].z = 'new'
        self.assertEqual(C.z, 'new')
        self.assertEqual(C[Tuple[int]].z, 'new')

        self.assertEqual(C().x, 'changed')
        self.assertEqual(C[Tuple[str]]().z, 'new')

        class D(C[T]):
            pass
        self.assertEqual(D[int].x, 'changed')
        self.assertEqual(D.z, 'new')
        D.z = 'from derived z'
        D[int].x = 'from derived x'
        self.assertEqual(C.x, 'changed')
        self.assertEqual(C[int].z, 'new')
        self.assertEqual(D.x, 'from derived x')
        self.assertEqual(D[str].z, 'from derived z')

    def test_abc_registry_kept(self):
        T = TypeVar('T')
        class C(collections.abc.Mapping, Generic[T]): ...
        C.register(int)
        self.assertIsInstance(1, C)
        C[int]
        self.assertIsInstance(1, C)
        C._abc_registry_clear()
        C._abc_caches_clear()  # To keep refleak hunting mode clean

    def test_false_subclasses(self):
        class MyMapping(MutableMapping[str, str]): pass
        self.assertNotIsInstance({}, MyMapping)
        self.assertNotIsSubclass(dict, MyMapping)

    def test_abc_bases(self):
        class MM(MutableMapping[str, str]):
            def __getitem__(self, k):
                return None
            def __setitem__(self, k, v):
                pass
            def __delitem__(self, k):
                pass
            def __iter__(self):
                return iter(())
            def __len__(self):
                return 0
        # this should just work
        MM().update()
        self.assertIsInstance(MM(), collections.abc.MutableMapping)
        self.assertIsInstance(MM(), MutableMapping)
        self.assertNotIsInstance(MM(), List)
        self.assertNotIsInstance({}, MM)

    def test_multiple_bases(self):
        class MM1(MutableMapping[str, str], collections.abc.MutableMapping):
            pass
        class MM2(collections.abc.MutableMapping, MutableMapping[str, str]):
            pass
        self.assertEqual(MM2.__bases__, (collections.abc.MutableMapping, Generic))

    def test_orig_bases(self):
        T = TypeVar('T')
        class C(typing.Dict[str, T]): ...
        self.assertEqual(C.__orig_bases__, (typing.Dict[str, T],))

    def test_naive_runtime_checks(self):
        def naive_dict_check(obj, tp):
            # Check if a dictionary conforms to Dict type
            if len(tp.__parameters__) > 0:
                raise NotImplementedError
            if tp.__args__:
                KT, VT = tp.__args__
                return all(
                    isinstance(k, KT) and isinstance(v, VT)
                    for k, v in obj.items()
                )
        self.assertTrue(naive_dict_check({'x': 1}, typing.Dict[str, int]))
        self.assertFalse(naive_dict_check({1: 'x'}, typing.Dict[str, int]))
        with self.assertRaises(NotImplementedError):
            naive_dict_check({1: 'x'}, typing.Dict[str, T])

        def naive_generic_check(obj, tp):
            # Check if an instance conforms to the generic class
            if not hasattr(obj, '__orig_class__'):
                raise NotImplementedError
            return obj.__orig_class__ == tp
        class Node(Generic[T]): ...
        self.assertTrue(naive_generic_check(Node[int](), Node[int]))
        self.assertFalse(naive_generic_check(Node[str](), Node[int]))
        self.assertFalse(naive_generic_check(Node[str](), List))
        with self.assertRaises(NotImplementedError):
            naive_generic_check([1, 2, 3], Node[int])

        def naive_list_base_check(obj, tp):
            # Check if list conforms to a List subclass
            return all(isinstance(x, tp.__orig_bases__[0].__args__[0])
                       for x in obj)
        class C(List[int]): ...
        self.assertTrue(naive_list_base_check([1, 2, 3], C))
        self.assertFalse(naive_list_base_check(['a', 'b'], C))

    def test_multi_subscr_base(self):
        T = TypeVar('T')
        U = TypeVar('U')
        V = TypeVar('V')
        class C(List[T][U][V]): ...
        class D(C, List[T][U][V]): ...
        self.assertEqual(C.__parameters__, (V,))
        self.assertEqual(D.__parameters__, (V,))
        self.assertEqual(C[int].__parameters__, ())
        self.assertEqual(D[int].__parameters__, ())
        self.assertEqual(C[int].__args__, (int,))
        self.assertEqual(D[int].__args__, (int,))
        self.assertEqual(C.__bases__, (list, Generic))
        self.assertEqual(D.__bases__, (C, list, Generic))
        self.assertEqual(C.__orig_bases__, (List[T][U][V],))
        self.assertEqual(D.__orig_bases__, (C, List[T][U][V]))

    def test_subscript_meta(self):
        T = TypeVar('T')
        class Meta(type): ...
        self.assertEqual(Type[Meta], Type[Meta])
        self.assertEqual(Union[T, int][Meta], Union[Meta, int])
        self.assertEqual(Callable[..., Meta].__args__, (Ellipsis, Meta))

    def test_generic_hashes(self):
        class A(Generic[T]):
            ...

        class B(Generic[T]):
            class A(Generic[T]):
                ...

        self.assertEqual(A, A)
        self.assertEqual(mod_generics_cache.A[str], mod_generics_cache.A[str])
        self.assertEqual(B.A, B.A)
        self.assertEqual(mod_generics_cache.B.A[B.A[str]],
                         mod_generics_cache.B.A[B.A[str]])

        self.assertNotEqual(A, B.A)
        self.assertNotEqual(A, mod_generics_cache.A)
        self.assertNotEqual(A, mod_generics_cache.B.A)
        self.assertNotEqual(B.A, mod_generics_cache.A)
        self.assertNotEqual(B.A, mod_generics_cache.B.A)

        self.assertNotEqual(A[str], B.A[str])
        self.assertNotEqual(A[List[Any]], B.A[List[Any]])
        self.assertNotEqual(A[str], mod_generics_cache.A[str])
        self.assertNotEqual(A[str], mod_generics_cache.B.A[str])
        self.assertNotEqual(B.A[int], mod_generics_cache.A[int])
        self.assertNotEqual(B.A[List[Any]], mod_generics_cache.B.A[List[Any]])

        self.assertNotEqual(Tuple[A[str]], Tuple[B.A[str]])
        self.assertNotEqual(Tuple[A[List[Any]]], Tuple[B.A[List[Any]]])
        self.assertNotEqual(Union[str, A[str]], Union[str, mod_generics_cache.A[str]])
        self.assertNotEqual(Union[A[str], A[str]],
                            Union[A[str], mod_generics_cache.A[str]])
        self.assertNotEqual(typing.FrozenSet[A[str]],
                            typing.FrozenSet[mod_generics_cache.B.A[str]])

        self.assertTrue(repr(Tuple[A[str]]).endswith('<locals>.A[str]]'))
        self.assertTrue(repr(Tuple[B.A[str]]).endswith('<locals>.B.A[str]]'))
        self.assertTrue(repr(Tuple[mod_generics_cache.A[str]])
                        .endswith('mod_generics_cache.A[str]]'))
        self.assertTrue(repr(Tuple[mod_generics_cache.B.A[str]])
                        .endswith('mod_generics_cache.B.A[str]]'))

    def test_extended_generic_rules_eq(self):
        T = TypeVar('T')
        U = TypeVar('U')
        self.assertEqual(Tuple[T, T][int], Tuple[int, int])
        self.assertEqual(typing.Iterable[Tuple[T, T]][T], typing.Iterable[Tuple[T, T]])
        with self.assertRaises(TypeError):
            Tuple[T, int][()]

        self.assertEqual(Union[T, int][int], int)
        self.assertEqual(Union[T, U][int, Union[int, str]], Union[int, str])
        class Base: ...
        class Derived(Base): ...
        self.assertEqual(Union[T, Base][Union[Base, Derived]], Union[Base, Derived])
        self.assertEqual(Callable[[T], T][KT], Callable[[KT], KT])
        self.assertEqual(Callable[..., List[T]][int], Callable[..., List[int]])

    def test_extended_generic_rules_repr(self):
        T = TypeVar('T')
        self.assertEqual(repr(Union[Tuple, Callable]).replace('typing.', ''),
                         'Union[Tuple, Callable]')
        self.assertEqual(repr(Union[Tuple, Tuple[int]]).replace('typing.', ''),
                         'Union[Tuple, Tuple[int]]')
        self.assertEqual(repr(Callable[..., Optional[T]][int]).replace('typing.', ''),
                         'Callable[..., Optional[int]]')
        self.assertEqual(repr(Callable[[], List[T]][int]).replace('typing.', ''),
                         'Callable[[], List[int]]')

    def test_generic_forward_ref(self):
        def foobar(x: List[List['CC']]): ...
        def foobar2(x: list[list[ForwardRef('CC')]]): ...
        def foobar3(x: list[ForwardRef('CC | int')] | int): ...
        class CC: ...
        self.assertEqual(
            get_type_hints(foobar, globals(), locals()),
            {'x': List[List[CC]]}
        )
        self.assertEqual(
            get_type_hints(foobar2, globals(), locals()),
            {'x': list[list[CC]]}
        )
        self.assertEqual(
            get_type_hints(foobar3, globals(), locals()),
            {'x': list[CC | int] | int}
        )

        T = TypeVar('T')
        AT = Tuple[T, ...]
        def barfoo(x: AT): ...
        self.assertIs(get_type_hints(barfoo, globals(), locals())['x'], AT)
        CT = Callable[..., List[T]]
        def barfoo2(x: CT): ...
        self.assertIs(get_type_hints(barfoo2, globals(), locals())['x'], CT)

    def test_generic_pep585_forward_ref(self):
        # See https://bugs.python.org/issue41370

        class C1:
            a: list['C1']
        self.assertEqual(
            get_type_hints(C1, globals(), locals()),
            {'a': list[C1]}
        )

        class C2:
            a: dict['C1', list[List[list['C2']]]]
        self.assertEqual(
            get_type_hints(C2, globals(), locals()),
            {'a': dict[C1, list[List[list[C2]]]]}
        )

        # Test stringified annotations
        scope = {}
        exec(textwrap.dedent('''
        from __future__ import annotations
        class C3:
            a: List[list["C2"]]
        '''), scope)
        C3 = scope['C3']
        self.assertEqual(C3.__annotations__['a'], "List[list['C2']]")
        self.assertEqual(
            get_type_hints(C3, globals(), locals()),
            {'a': List[list[C2]]}
        )

        # Test recursive types
        X = list["X"]
        def f(x: X): ...
        self.assertEqual(
            get_type_hints(f, globals(), locals()),
            {'x': list[list[ForwardRef('X')]]}
        )

    def test_extended_generic_rules_subclassing(self):
        class T1(Tuple[T, KT]): ...
        class T2(Tuple[T, ...]): ...
        class C1(typing.Container[T]):
            def __contains__(self, item):
                return False

        self.assertEqual(T1.__parameters__, (T, KT))
        self.assertEqual(T1[int, str].__args__, (int, str))
        self.assertEqual(T1[int, T].__origin__, T1)

        self.assertEqual(T2.__parameters__, (T,))
        # These don't work because of tuple.__class_item__
        ## with self.assertRaises(TypeError):
        ##     T1[int]
        ## with self.assertRaises(TypeError):
        ##     T2[int, str]

        self.assertEqual(repr(C1[int]).split('.')[-1], 'C1[int]')
        self.assertEqual(C1.__parameters__, (T,))
        self.assertIsInstance(C1(), collections.abc.Container)
        self.assertIsSubclass(C1, collections.abc.Container)
        self.assertIsInstance(T1(), tuple)
        self.assertIsSubclass(T2, tuple)
        with self.assertRaises(TypeError):
            issubclass(Tuple[int, ...], typing.Sequence)
        with self.assertRaises(TypeError):
            issubclass(Tuple[int, ...], typing.Iterable)

    def test_fail_with_bare_union(self):
        with self.assertRaises(TypeError):
            List[Union]
        with self.assertRaises(TypeError):
            Tuple[Optional]
        with self.assertRaises(TypeError):
            ClassVar[ClassVar[int]]
        with self.assertRaises(TypeError):
            List[ClassVar[int]]

    def test_fail_with_bare_generic(self):
        T = TypeVar('T')
        with self.assertRaises(TypeError):
            List[Generic]
        with self.assertRaises(TypeError):
            Tuple[Generic[T]]
        with self.assertRaises(TypeError):
            List[typing.Protocol]

    def test_type_erasure_special(self):
        T = TypeVar('T')
        # this is the only test that checks type caching
        self.clear_caches()
        class MyTup(Tuple[T, T]): ...
        self.assertIs(MyTup[int]().__class__, MyTup)
        self.assertEqual(MyTup[int]().__orig_class__, MyTup[int])
        class MyDict(typing.Dict[T, T]): ...
        self.assertIs(MyDict[int]().__class__, MyDict)
        self.assertEqual(MyDict[int]().__orig_class__, MyDict[int])
        class MyDef(typing.DefaultDict[str, T]): ...
        self.assertIs(MyDef[int]().__class__, MyDef)
        self.assertEqual(MyDef[int]().__orig_class__, MyDef[int])
        class MyChain(typing.ChainMap[str, T]): ...
        self.assertIs(MyChain[int]().__class__, MyChain)
        self.assertEqual(MyChain[int]().__orig_class__, MyChain[int])

    def test_all_repr_eq_any(self):
        objs = (getattr(typing, el) for el in typing.__all__)
        for obj in objs:
            self.assertNotEqual(repr(obj), '')
            self.assertEqual(obj, obj)
            if (getattr(obj, '__parameters__', None)
                    and not isinstance(obj, typing.TypeVar)
                    and isinstance(obj.__parameters__, tuple)
                    and len(obj.__parameters__) == 1):
                self.assertEqual(obj[Any].__args__, (Any,))
            if isinstance(obj, type):
                for base in obj.__mro__:
                    self.assertNotEqual(repr(base), '')
                    self.assertEqual(base, base)

    def test_pickle(self):
        global C  # pickle wants to reference the class by name
        T = TypeVar('T')

        class B(Generic[T]):
            pass

        class C(B[int]):
            pass

        c = C()
        c.foo = 42
        c.bar = 'abc'
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            z = pickle.dumps(c, proto)
            x = pickle.loads(z)
            self.assertEqual(x.foo, 42)
            self.assertEqual(x.bar, 'abc')
            self.assertEqual(x.__dict__, {'foo': 42, 'bar': 'abc'})
        samples = [Any, Union, Tuple, Callable, ClassVar,
                   Union[int, str], ClassVar[List], Tuple[int, ...], Tuple[()],
                   Callable[[str], bytes],
                   typing.DefaultDict, typing.FrozenSet[int]]
        for s in samples:
            for proto in range(pickle.HIGHEST_PROTOCOL + 1):
                z = pickle.dumps(s, proto)
                x = pickle.loads(z)
                self.assertEqual(s, x)
        more_samples = [List, typing.Iterable, typing.Type, List[int],
                        typing.Type[typing.Mapping], typing.AbstractSet[Tuple[int, str]]]
        for s in more_samples:
            for proto in range(pickle.HIGHEST_PROTOCOL + 1):
                z = pickle.dumps(s, proto)
                x = pickle.loads(z)
                self.assertEqual(s, x)

    def test_copy_and_deepcopy(self):
        T = TypeVar('T')
        class Node(Generic[T]): ...
        things = [Union[T, int], Tuple[T, int], Tuple[()],
                  Callable[..., T], Callable[[int], int],
                  Tuple[Any, Any], Node[T], Node[int], Node[Any], typing.Iterable[T],
                  typing.Iterable[Any], typing.Iterable[int], typing.Dict[int, str],
                  typing.Dict[T, Any], ClassVar[int], ClassVar[List[T]], Tuple['T', 'T'],
                  Union['T', int], List['T'], typing.Mapping['T', int]]
        for t in things + [Any]:
            self.assertEqual(t, copy(t))
            self.assertEqual(t, deepcopy(t))

    def test_immutability_by_copy_and_pickle(self):
        # Special forms like Union, Any, etc., generic aliases to containers like List,
        # Mapping, etc., and type variabcles are considered immutable by copy and pickle.
        global TP, TPB, TPV, PP  # for pickle
        TP = TypeVar('TP')
        TPB = TypeVar('TPB', bound=int)
        TPV = TypeVar('TPV', bytes, str)
        PP = ParamSpec('PP')
        for X in [TP, TPB, TPV, PP,
                  List, typing.Mapping, ClassVar, typing.Iterable,
                  Union, Any, Tuple, Callable]:
            with self.subTest(thing=X):
                self.assertIs(copy(X), X)
                self.assertIs(deepcopy(X), X)
                for proto in range(pickle.HIGHEST_PROTOCOL + 1):
                    self.assertIs(pickle.loads(pickle.dumps(X, proto)), X)
        del TP, TPB, TPV, PP

        # Check that local type variables are copyable.
        TL = TypeVar('TL')
        TLB = TypeVar('TLB', bound=int)
        TLV = TypeVar('TLV', bytes, str)
        PL = ParamSpec('PL')
        for X in [TL, TLB, TLV, PL]:
            with self.subTest(thing=X):
                self.assertIs(copy(X), X)
                self.assertIs(deepcopy(X), X)

    def test_copy_generic_instances(self):
        T = TypeVar('T')
        class C(Generic[T]):
            def __init__(self, attr: T) -> None:
                self.attr = attr

        c = C(42)
        self.assertEqual(copy(c).attr, 42)
        self.assertEqual(deepcopy(c).attr, 42)
        self.assertIsNot(copy(c), c)
        self.assertIsNot(deepcopy(c), c)
        c.attr = 1
        self.assertEqual(copy(c).attr, 1)
        self.assertEqual(deepcopy(c).attr, 1)
        ci = C[int](42)
        self.assertEqual(copy(ci).attr, 42)
        self.assertEqual(deepcopy(ci).attr, 42)
        self.assertIsNot(copy(ci), ci)
        self.assertIsNot(deepcopy(ci), ci)
        ci.attr = 1
        self.assertEqual(copy(ci).attr, 1)
        self.assertEqual(deepcopy(ci).attr, 1)
        self.assertEqual(ci.__orig_class__, C[int])

    def test_weakref_all(self):
        T = TypeVar('T')
        things = [Any, Union[T, int], Callable[..., T], Tuple[Any, Any],
                  Optional[List[int]], typing.Mapping[int, str],
                  typing.Match[bytes], typing.Iterable['whatever']]
        for t in things:
            self.assertEqual(weakref.ref(t)(), t)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_parameterized_slots(self):
        T = TypeVar('T')
        class C(Generic[T]):
            __slots__ = ('potato',)

        c = C()
        c_int = C[int]()

        c.potato = 0
        c_int.potato = 0
        with self.assertRaises(AttributeError):
            c.tomato = 0
        with self.assertRaises(AttributeError):
            c_int.tomato = 0

        def foo(x: C['C']): ...
        self.assertEqual(get_type_hints(foo, globals(), locals())['x'], C[C])
        self.assertEqual(copy(C[int]), deepcopy(C[int]))

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_parameterized_slots_dict(self):
        T = TypeVar('T')
        class D(Generic[T]):
            __slots__ = {'banana': 42}

        d = D()
        d_int = D[int]()

        d.banana = 'yes'
        d_int.banana = 'yes'
        with self.assertRaises(AttributeError):
            d.foobar = 'no'
        with self.assertRaises(AttributeError):
            d_int.foobar = 'no'

    def test_errors(self):
        with self.assertRaises(TypeError):
            B = SimpleMapping[XK, Any]

            class C(Generic[B]):
                pass

    def test_repr_2(self):
        class C(Generic[T]):
            pass

        self.assertEqual(C.__module__, __name__)
        self.assertEqual(C.__qualname__,
                         'GenericTests.test_repr_2.<locals>.C')
        X = C[int]
        self.assertEqual(X.__module__, __name__)
        self.assertEqual(repr(X).split('.')[-1], 'C[int]')

        class Y(C[int]):
            pass

        self.assertEqual(Y.__module__, __name__)
        self.assertEqual(Y.__qualname__,
                         'GenericTests.test_repr_2.<locals>.Y')

    def test_eq_1(self):
        self.assertEqual(Generic, Generic)
        self.assertEqual(Generic[T], Generic[T])
        self.assertNotEqual(Generic[KT], Generic[VT])

    def test_eq_2(self):

        class A(Generic[T]):
            pass

        class B(Generic[T]):
            pass

        self.assertEqual(A, A)
        self.assertNotEqual(A, B)
        self.assertEqual(A[T], A[T])
        self.assertNotEqual(A[T], B[T])

    def test_multiple_inheritance(self):

        class A(Generic[T, VT]):
            pass

        class B(Generic[KT, T]):
            pass

        class C(A[T, VT], Generic[VT, T, KT], B[KT, T]):
            pass

        self.assertEqual(C.__parameters__, (VT, T, KT))

    def test_multiple_inheritance_special(self):
        S = TypeVar('S')
        class B(Generic[S]): ...
        class C(List[int], B): ...
        self.assertEqual(C.__mro__, (C, list, B, Generic, object))

    def test_init_subclass_super_called(self):
        class FinalException(Exception):
            pass

        class Final:
            def __init_subclass__(cls, **kwargs) -> None:
                for base in cls.__bases__:
                    if base is not Final and issubclass(base, Final):
                        raise FinalException(base)
                super().__init_subclass__(**kwargs)
        class Test(Generic[T], Final):
            pass
        with self.assertRaises(FinalException):
            class Subclass(Test):
                pass
        with self.assertRaises(FinalException):
            class Subclass(Test[int]):
                pass

    def test_nested(self):

        G = Generic

        class Visitor(G[T]):

            a = None

            def set(self, a: T):
                self.a = a

            def get(self):
                return self.a

            def visit(self) -> T:
                return self.a

        V = Visitor[typing.List[int]]

        class IntListVisitor(V):

            def append(self, x: int):
                self.a.append(x)

        a = IntListVisitor()
        a.set([])
        a.append(1)
        a.append(42)
        self.assertEqual(a.get(), [1, 42])

    def test_type_erasure(self):
        T = TypeVar('T')

        class Node(Generic[T]):
            def __init__(self, label: T,
                         left: 'Node[T]' = None,
                         right: 'Node[T]' = None):
                self.label = label  # type: T
                self.left = left  # type: Optional[Node[T]]
                self.right = right  # type: Optional[Node[T]]

        def foo(x: T):
            a = Node(x)
            b = Node[T](x)
            c = Node[Any](x)
            self.assertIs(type(a), Node)
            self.assertIs(type(b), Node)
            self.assertIs(type(c), Node)
            self.assertEqual(a.label, x)
            self.assertEqual(b.label, x)
            self.assertEqual(c.label, x)

        foo(42)

    def test_implicit_any(self):
        T = TypeVar('T')

        class C(Generic[T]):
            pass

        class D(C):
            pass

        self.assertEqual(D.__parameters__, ())

        with self.assertRaises(TypeError):
            D[int]
        with self.assertRaises(TypeError):
            D[Any]
        with self.assertRaises(TypeError):
            D[T]

    def test_new_with_args(self):

        class A(Generic[T]):
            pass

        class B:
            def __new__(cls, arg):
                # call object
                obj = super().__new__(cls)
                obj.arg = arg
                return obj

        # mro: C, A, Generic, B, object
        class C(A, B):
            pass

        c = C('foo')
        self.assertEqual(c.arg, 'foo')

    def test_new_with_args2(self):

        class A:
            def __init__(self, arg):
                self.from_a = arg
                # call object
                super().__init__()

        # mro: C, Generic, A, object
        class C(Generic[T], A):
            def __init__(self, arg):
                self.from_c = arg
                # call Generic
                super().__init__(arg)

        c = C('foo')
        self.assertEqual(c.from_a, 'foo')
        self.assertEqual(c.from_c, 'foo')

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_new_no_args(self):

        class A(Generic[T]):
            pass

        with self.assertRaises(TypeError):
            A('foo')

        class B:
            def __new__(cls):
                # call object
                obj = super().__new__(cls)
                obj.from_b = 'b'
                return obj

        # mro: C, A, Generic, B, object
        class C(A, B):
            def __init__(self, arg):
                self.arg = arg

            def __new__(cls, arg):
                # call A
                obj = super().__new__(cls)
                obj.from_c = 'c'
                return obj

        c = C('foo')
        self.assertEqual(c.arg, 'foo')
        self.assertEqual(c.from_b, 'b')
        self.assertEqual(c.from_c, 'c')

    def test_subclass_special_form(self):
        for obj in (
            ClassVar[int],
            Final[int],
            Union[int, float],
            Optional[int],
            Literal[1, 2],
            Concatenate[int, ParamSpec("P")],
            TypeGuard[int],
        ):
            with self.subTest(msg=obj):
                with self.assertRaisesRegex(
                        TypeError, f'^{re.escape(f"Cannot subclass {obj!r}")}$'
                ):
                    class Foo(obj):
                        pass

    def test_complex_subclasses(self):
        T_co = TypeVar("T_co", covariant=True)

        class Base(Generic[T_co]):
            ...

        T = TypeVar("T")

        # see gh-94607: this fails in that bug
        class Sub(Base, Generic[T]):
            ...

    def test_parameter_detection(self):
        self.assertEqual(List[T].__parameters__, (T,))
        self.assertEqual(List[List[T]].__parameters__, (T,))
        class A:
            __parameters__ = (T,)
        # Bare classes should be skipped
        for a in (List, list):
            for b in (A, int, TypeVar, TypeVarTuple, ParamSpec, types.GenericAlias, types.UnionType):
                with self.subTest(generic=a, sub=b):
                    with self.assertRaisesRegex(TypeError, '.* is not a generic class'):
                        a[b][str]
        # Duck-typing anything that looks like it has __parameters__.
        # These tests are optional and failure is okay.
        self.assertEqual(List[A()].__parameters__, (T,))
        # C version of GenericAlias
        self.assertEqual(list[A()].__parameters__, (T,))

    def test_non_generic_subscript(self):
        T = TypeVar('T')
        class G(Generic[T]):
            pass
        class A:
            __parameters__ = (T,)

        for s in (int, G, A, List, list,
                  TypeVar, TypeVarTuple, ParamSpec,
                  types.GenericAlias, types.UnionType):

            for t in Tuple, tuple:
                with self.subTest(tuple=t, sub=s):
                    self.assertEqual(t[s, T][int], t[s, int])
                    self.assertEqual(t[T, s][int], t[int, s])
                    a = t[s]
                    with self.assertRaises(TypeError):
                        a[int]

            for c in Callable, collections.abc.Callable:
                with self.subTest(callable=c, sub=s):
                    self.assertEqual(c[[s], T][int], c[[s], int])
                    self.assertEqual(c[[T], s][int], c[[int], s])
                    a = c[[s], s]
                    with self.assertRaises(TypeError):
                        a[int]


class ClassVarTests(BaseTestCase):

    def test_basics(self):
        with self.assertRaises(TypeError):
            ClassVar[int, str]
        with self.assertRaises(TypeError):
            ClassVar[int][str]

    def test_repr(self):
        self.assertEqual(repr(ClassVar), 'typing.ClassVar')
        cv = ClassVar[int]
        self.assertEqual(repr(cv), 'typing.ClassVar[int]')
        cv = ClassVar[Employee]
        self.assertEqual(repr(cv), 'typing.ClassVar[%s.Employee]' % __name__)

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(ClassVar)):
                pass
        with self.assertRaises(TypeError):
            class C(type(ClassVar[int])):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            ClassVar()
        with self.assertRaises(TypeError):
            type(ClassVar)()
        with self.assertRaises(TypeError):
            type(ClassVar[Optional[int]])()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, ClassVar[int])
        with self.assertRaises(TypeError):
            issubclass(int, ClassVar)

class FinalTests(BaseTestCase):

    def test_basics(self):
        Final[int]  # OK
        with self.assertRaises(TypeError):
            Final[int, str]
        with self.assertRaises(TypeError):
            Final[int][str]
        with self.assertRaises(TypeError):
            Optional[Final[int]]

    def test_repr(self):
        self.assertEqual(repr(Final), 'typing.Final')
        cv = Final[int]
        self.assertEqual(repr(cv), 'typing.Final[int]')
        cv = Final[Employee]
        self.assertEqual(repr(cv), 'typing.Final[%s.Employee]' % __name__)
        cv = Final[tuple[int]]
        self.assertEqual(repr(cv), 'typing.Final[tuple[int]]')

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(Final)):
                pass
        with self.assertRaises(TypeError):
            class C(type(Final[int])):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            Final()
        with self.assertRaises(TypeError):
            type(Final)()
        with self.assertRaises(TypeError):
            type(Final[Optional[int]])()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, Final[int])
        with self.assertRaises(TypeError):
            issubclass(int, Final)


class FinalDecoratorTests(BaseTestCase):
    def test_final_unmodified(self):
        def func(x): ...
        self.assertIs(func, final(func))

    def test_dunder_final(self):
        @final
        def func(): ...
        @final
        class Cls: ...
        self.assertIs(True, func.__final__)
        self.assertIs(True, Cls.__final__)

        class Wrapper:
            __slots__ = ("func",)
            def __init__(self, func):
                self.func = func
            def __call__(self, *args, **kwargs):
                return self.func(*args, **kwargs)

        # Check that no error is thrown if the attribute
        # is not writable.
        @final
        @Wrapper
        def wrapped(): ...
        self.assertIsInstance(wrapped, Wrapper)
        self.assertIs(False, hasattr(wrapped, "__final__"))

        class Meta(type):
            @property
            def __final__(self): return "can't set me"
        @final
        class WithMeta(metaclass=Meta): ...
        self.assertEqual(WithMeta.__final__, "can't set me")

        # Builtin classes throw TypeError if you try to set an
        # attribute.
        final(int)
        self.assertIs(False, hasattr(int, "__final__"))

        # Make sure it works with common builtin decorators
        class Methods:
            @final
            @classmethod
            def clsmethod(cls): ...

            @final
            @staticmethod
            def stmethod(): ...

            # The other order doesn't work because property objects
            # don't allow attribute assignment.
            @property
            @final
            def prop(self): ...

            @final
            @lru_cache()
            def cached(self): ...

        # Use getattr_static because the descriptor returns the
        # underlying function, which doesn't have __final__.
        self.assertIs(
            True,
            inspect.getattr_static(Methods, "clsmethod").__final__
        )
        self.assertIs(
            True,
            inspect.getattr_static(Methods, "stmethod").__final__
        )
        self.assertIs(True, Methods.prop.fget.__final__)
        self.assertIs(True, Methods.cached.__final__)


class CastTests(BaseTestCase):

    def test_basics(self):
        self.assertEqual(cast(int, 42), 42)
        self.assertEqual(cast(float, 42), 42)
        self.assertIs(type(cast(float, 42)), int)
        self.assertEqual(cast(Any, 42), 42)
        self.assertEqual(cast(list, 42), 42)
        self.assertEqual(cast(Union[str, float], 42), 42)
        self.assertEqual(cast(AnyStr, 42), 42)
        self.assertEqual(cast(None, 42), 42)

    def test_errors(self):
        # Bogus calls are not expected to fail.
        cast(42, 42)
        cast('hello', 42)


class AssertTypeTests(BaseTestCase):

    def test_basics(self):
        arg = 42
        self.assertIs(assert_type(arg, int), arg)
        self.assertIs(assert_type(arg, str | float), arg)
        self.assertIs(assert_type(arg, AnyStr), arg)
        self.assertIs(assert_type(arg, None), arg)

    def test_errors(self):
        # Bogus calls are not expected to fail.
        arg = 42
        self.assertIs(assert_type(arg, 42), arg)
        self.assertIs(assert_type(arg, 'hello'), arg)


# We need this to make sure that `@no_type_check` respects `__module__` attr:
from test import ann_module8

@no_type_check
class NoTypeCheck_Outer:
    Inner = ann_module8.NoTypeCheck_Outer.Inner

@no_type_check
class NoTypeCheck_WithFunction:
    NoTypeCheck_function = ann_module8.NoTypeCheck_function


class ForwardRefTests(BaseTestCase):

    def test_basics(self):

        class Node(Generic[T]):

            def __init__(self, label: T):
                self.label = label
                self.left = self.right = None

            def add_both(self,
                         left: 'Optional[Node[T]]',
                         right: 'Node[T]' = None,
                         stuff: int = None,
                         blah=None):
                self.left = left
                self.right = right

            def add_left(self, node: Optional['Node[T]']):
                self.add_both(node, None)

            def add_right(self, node: 'Node[T]' = None):
                self.add_both(None, node)

        t = Node[int]
        both_hints = get_type_hints(t.add_both, globals(), locals())
        self.assertEqual(both_hints['left'], Optional[Node[T]])
        self.assertEqual(both_hints['right'], Node[T])
        self.assertEqual(both_hints['stuff'], int)
        self.assertNotIn('blah', both_hints)

        left_hints = get_type_hints(t.add_left, globals(), locals())
        self.assertEqual(left_hints['node'], Optional[Node[T]])

        right_hints = get_type_hints(t.add_right, globals(), locals())
        self.assertEqual(right_hints['node'], Node[T])

    def test_forwardref_instance_type_error(self):
        fr = typing.ForwardRef('int')
        with self.assertRaises(TypeError):
            isinstance(42, fr)

    def test_forwardref_subclass_type_error(self):
        fr = typing.ForwardRef('int')
        with self.assertRaises(TypeError):
            issubclass(int, fr)

    def test_forwardref_only_str_arg(self):
        with self.assertRaises(TypeError):
            typing.ForwardRef(1)  # only `str` type is allowed

    def test_forward_equality(self):
        fr = typing.ForwardRef('int')
        self.assertEqual(fr, typing.ForwardRef('int'))
        self.assertNotEqual(List['int'], List[int])
        self.assertNotEqual(fr, typing.ForwardRef('int', module=__name__))
        frm = typing.ForwardRef('int', module=__name__)
        self.assertEqual(frm, typing.ForwardRef('int', module=__name__))
        self.assertNotEqual(frm, typing.ForwardRef('int', module='__other_name__'))

    def test_forward_equality_gth(self):
        c1 = typing.ForwardRef('C')
        c1_gth = typing.ForwardRef('C')
        c2 = typing.ForwardRef('C')
        c2_gth = typing.ForwardRef('C')

        class C:
            pass
        def foo(a: c1_gth, b: c2_gth):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()), {'a': C, 'b': C})
        self.assertEqual(c1, c2)
        self.assertEqual(c1, c1_gth)
        self.assertEqual(c1_gth, c2_gth)
        self.assertEqual(List[c1], List[c1_gth])
        self.assertNotEqual(List[c1], List[C])
        self.assertNotEqual(List[c1_gth], List[C])
        self.assertEqual(Union[c1, c1_gth], Union[c1])
        self.assertEqual(Union[c1, c1_gth, int], Union[c1, int])

    def test_forward_equality_hash(self):
        c1 = typing.ForwardRef('int')
        c1_gth = typing.ForwardRef('int')
        c2 = typing.ForwardRef('int')
        c2_gth = typing.ForwardRef('int')

        def foo(a: c1_gth, b: c2_gth):
            pass
        get_type_hints(foo, globals(), locals())

        self.assertEqual(hash(c1), hash(c2))
        self.assertEqual(hash(c1_gth), hash(c2_gth))
        self.assertEqual(hash(c1), hash(c1_gth))

        c3 = typing.ForwardRef('int', module=__name__)
        c4 = typing.ForwardRef('int', module='__other_name__')

        self.assertNotEqual(hash(c3), hash(c1))
        self.assertNotEqual(hash(c3), hash(c1_gth))
        self.assertNotEqual(hash(c3), hash(c4))
        self.assertEqual(hash(c3), hash(typing.ForwardRef('int', module=__name__)))

    def test_forward_equality_namespace(self):
        class A:
            pass
        def namespace1():
            a = typing.ForwardRef('A')
            def fun(x: a):
                pass
            get_type_hints(fun, globals(), locals())
            return a

        def namespace2():
            a = typing.ForwardRef('A')

            class A:
                pass
            def fun(x: a):
                pass

            get_type_hints(fun, globals(), locals())
            return a

        self.assertEqual(namespace1(), namespace1())
        self.assertNotEqual(namespace1(), namespace2())

    def test_forward_repr(self):
        self.assertEqual(repr(List['int']), "typing.List[ForwardRef('int')]")
        self.assertEqual(repr(List[ForwardRef('int', module='mod')]),
                         "typing.List[ForwardRef('int', module='mod')]")

    def test_union_forward(self):

        def foo(a: Union['T']):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': Union[T]})

        def foo(a: tuple[ForwardRef('T')] | int):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': tuple[T] | int})

    def test_tuple_forward(self):

        def foo(a: Tuple['T']):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': Tuple[T]})

        def foo(a: tuple[ForwardRef('T')]):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': tuple[T]})

    def test_double_forward(self):
        def foo(a: 'List[\'int\']'):
            pass
        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': List[int]})

    def test_forward_recursion_actually(self):
        def namespace1():
            a = typing.ForwardRef('A')
            A = a
            def fun(x: a): pass

            ret = get_type_hints(fun, globals(), locals())
            return a

        def namespace2():
            a = typing.ForwardRef('A')
            A = a
            def fun(x: a): pass

            ret = get_type_hints(fun, globals(), locals())
            return a

        def cmp(o1, o2):
            return o1 == o2

        r1 = namespace1()
        r2 = namespace2()
        self.assertIsNot(r1, r2)
        self.assertRaises(RecursionError, cmp, r1, r2)

    def test_union_forward_recursion(self):
        ValueList = List['Value']
        Value = Union[str, ValueList]

        class C:
            foo: List[Value]
        class D:
            foo: Union[Value, ValueList]
        class E:
            foo: Union[List[Value], ValueList]
        class F:
            foo: Union[Value, List[Value], ValueList]

        self.assertEqual(get_type_hints(C, globals(), locals()), get_type_hints(C, globals(), locals()))
        self.assertEqual(get_type_hints(C, globals(), locals()),
                         {'foo': List[Union[str, List[Union[str, List['Value']]]]]})
        self.assertEqual(get_type_hints(D, globals(), locals()),
                         {'foo': Union[str, List[Union[str, List['Value']]]]})
        self.assertEqual(get_type_hints(E, globals(), locals()),
                         {'foo': Union[
                             List[Union[str, List[Union[str, List['Value']]]]],
                             List[Union[str, List['Value']]]
                         ]
                          })
        self.assertEqual(get_type_hints(F, globals(), locals()),
                         {'foo': Union[
                             str,
                             List[Union[str, List['Value']]],
                             List[Union[str, List[Union[str, List['Value']]]]]
                         ]
                          })

    def test_callable_forward(self):

        def foo(a: Callable[['T'], 'T']):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': Callable[[T], T]})

    def test_callable_with_ellipsis_forward(self):

        def foo(a: 'Callable[..., T]'):
            pass

        self.assertEqual(get_type_hints(foo, globals(), locals()),
                         {'a': Callable[..., T]})

    def test_special_forms_forward(self):

        class C:
            a: Annotated['ClassVar[int]', (3, 5)] = 4
            b: Annotated['Final[int]', "const"] = 4
            x: 'ClassVar' = 4
            y: 'Final' = 4

        class CF:
            b: List['Final[int]'] = 4

        self.assertEqual(get_type_hints(C, globals())['a'], ClassVar[int])
        self.assertEqual(get_type_hints(C, globals())['b'], Final[int])
        self.assertEqual(get_type_hints(C, globals())['x'], ClassVar)
        self.assertEqual(get_type_hints(C, globals())['y'], Final)
        with self.assertRaises(TypeError):
            get_type_hints(CF, globals()),

    def test_syntax_error(self):

        with self.assertRaises(SyntaxError):
            Generic['/T']

    def test_delayed_syntax_error(self):

        def foo(a: 'Node[T'):
            pass

        with self.assertRaises(SyntaxError):
            get_type_hints(foo)

    def test_name_error(self):

        def foo(a: 'Noode[T]'):
            pass

        with self.assertRaises(NameError):
            get_type_hints(foo, locals())

    def test_no_type_check(self):

        @no_type_check
        def foo(a: 'whatevers') -> {}:
            pass

        th = get_type_hints(foo)
        self.assertEqual(th, {})

    def test_no_type_check_class(self):

        @no_type_check
        class C:
            def foo(a: 'whatevers') -> {}:
                pass

        cth = get_type_hints(C.foo)
        self.assertEqual(cth, {})
        ith = get_type_hints(C().foo)
        self.assertEqual(ith, {})

    def test_no_type_check_no_bases(self):
        class C:
            def meth(self, x: int): ...
        @no_type_check
        class D(C):
            c = C

        # verify that @no_type_check never affects bases
        self.assertEqual(get_type_hints(C.meth), {'x': int})

        # and never child classes:
        class Child(D):
            def foo(self, x: int): ...

        self.assertEqual(get_type_hints(Child.foo), {'x': int})

    def test_no_type_check_nested_types(self):
        # See https://bugs.python.org/issue46571
        class Other:
            o: int
        class B:  # Has the same `__name__`` as `A.B` and different `__qualname__`
            o: int
        @no_type_check
        class A:
            a: int
            class B:
                b: int
                class C:
                    c: int
            class D:
                d: int

            Other = Other

        for klass in [A, A.B, A.B.C, A.D]:
            with self.subTest(klass=klass):
                self.assertTrue(klass.__no_type_check__)
                self.assertEqual(get_type_hints(klass), {})

        for not_modified in [Other, B]:
            with self.subTest(not_modified=not_modified):
                with self.assertRaises(AttributeError):
                    not_modified.__no_type_check__
                self.assertNotEqual(get_type_hints(not_modified), {})

    def test_no_type_check_class_and_static_methods(self):
        @no_type_check
        class Some:
            @staticmethod
            def st(x: int) -> int: ...
            @classmethod
            def cl(cls, y: int) -> int: ...

        self.assertTrue(Some.st.__no_type_check__)
        self.assertEqual(get_type_hints(Some.st), {})
        self.assertTrue(Some.cl.__no_type_check__)
        self.assertEqual(get_type_hints(Some.cl), {})

    def test_no_type_check_other_module(self):
        self.assertTrue(NoTypeCheck_Outer.__no_type_check__)
        with self.assertRaises(AttributeError):
            ann_module8.NoTypeCheck_Outer.__no_type_check__
        with self.assertRaises(AttributeError):
            ann_module8.NoTypeCheck_Outer.Inner.__no_type_check__

        self.assertTrue(NoTypeCheck_WithFunction.__no_type_check__)
        with self.assertRaises(AttributeError):
            ann_module8.NoTypeCheck_function.__no_type_check__

    def test_no_type_check_foreign_functions(self):
        # We should not modify this function:
        def some(*args: int) -> int:
            ...

        @no_type_check
        class A:
            some_alias = some
            some_class = classmethod(some)
            some_static = staticmethod(some)

        with self.assertRaises(AttributeError):
            some.__no_type_check__
        self.assertEqual(get_type_hints(some), {'args': int, 'return': int})

    def test_no_type_check_lambda(self):
        @no_type_check
        class A:
            # Corner case: `lambda` is both an assignment and a function:
            bar: Callable[[int], int] = lambda arg: arg

        self.assertTrue(A.bar.__no_type_check__)
        self.assertEqual(get_type_hints(A.bar), {})

    def test_no_type_check_TypeError(self):
        # This simply should not fail with
        # `TypeError: can't set attributes of built-in/extension type 'dict'`
        no_type_check(dict)

    def test_no_type_check_forward_ref_as_string(self):
        class C:
            foo: typing.ClassVar[int] = 7
        class D:
            foo: ClassVar[int] = 7
        class E:
            foo: 'typing.ClassVar[int]' = 7
        class F:
            foo: 'ClassVar[int]' = 7

        expected_result = {'foo': typing.ClassVar[int]}
        for clazz in [C, D, E, F]:
            self.assertEqual(get_type_hints(clazz), expected_result)

    def test_nested_classvar_fails_forward_ref_check(self):
        class E:
            foo: 'typing.ClassVar[typing.ClassVar[int]]' = 7
        class F:
            foo: ClassVar['ClassVar[int]'] = 7

        for clazz in [E, F]:
            with self.assertRaises(TypeError):
                get_type_hints(clazz)

    def test_meta_no_type_check(self):

        @no_type_check_decorator
        def magic_decorator(func):
            return func

        self.assertEqual(magic_decorator.__name__, 'magic_decorator')

        @magic_decorator
        def foo(a: 'whatevers') -> {}:
            pass

        @magic_decorator
        class C:
            def foo(a: 'whatevers') -> {}:
                pass

        self.assertEqual(foo.__name__, 'foo')
        th = get_type_hints(foo)
        self.assertEqual(th, {})
        cth = get_type_hints(C.foo)
        self.assertEqual(cth, {})
        ith = get_type_hints(C().foo)
        self.assertEqual(ith, {})

    def test_default_globals(self):
        code = ("class C:\n"
                "    def foo(self, a: 'C') -> 'D': pass\n"
                "class D:\n"
                "    def bar(self, b: 'D') -> C: pass\n"
                )
        ns = {}
        exec(code, ns)
        hints = get_type_hints(ns['C'].foo)
        self.assertEqual(hints, {'a': ns['C'], 'return': ns['D']})

    def test_final_forward_ref(self):
        self.assertEqual(gth(Loop, globals())['attr'], Final[Loop])
        self.assertNotEqual(gth(Loop, globals())['attr'], Final[int])
        self.assertNotEqual(gth(Loop, globals())['attr'], Final)

    def test_or(self):
        X = ForwardRef('X')
        # __or__/__ror__ itself
        self.assertEqual(X | "x", Union[X, "x"])
        self.assertEqual("x" | X, Union["x", X])


@lru_cache()
def cached_func(x, y):
    return 3 * x + y


class MethodHolder:
    @classmethod
    def clsmethod(cls): ...
    @staticmethod
    def stmethod(): ...
    def method(self): ...


class OverloadTests(BaseTestCase):

    def test_overload_fails(self):
        with self.assertRaises(NotImplementedError):

            @overload
            def blah():
                pass

            blah()

    def test_overload_succeeds(self):
        @overload
        def blah():
            pass

        def blah():
            pass

        blah()

    @cpython_only  # gh-98713
    def test_overload_on_compiled_functions(self):
        with patch("typing._overload_registry",
                   defaultdict(lambda: defaultdict(dict))):
            # The registry starts out empty:
            self.assertEqual(typing._overload_registry, {})

            # This should just not fail:
            overload(sum)
            overload(print)

            # No overloads are recorded (but, it still has a side-effect):
            self.assertEqual(typing.get_overloads(sum), [])
            self.assertEqual(typing.get_overloads(print), [])

    def set_up_overloads(self):
        def blah():
            pass

        overload1 = blah
        overload(blah)

        def blah():
            pass

        overload2 = blah
        overload(blah)

        def blah():
            pass

        return blah, [overload1, overload2]

    # Make sure we don't clear the global overload registry
    @patch("typing._overload_registry",
        defaultdict(lambda: defaultdict(dict)))
    def test_overload_registry(self):
        # The registry starts out empty
        self.assertEqual(typing._overload_registry, {})

        impl, overloads = self.set_up_overloads()
        self.assertNotEqual(typing._overload_registry, {})
        self.assertEqual(list(get_overloads(impl)), overloads)

        def some_other_func(): pass
        overload(some_other_func)
        other_overload = some_other_func
        def some_other_func(): pass
        self.assertEqual(list(get_overloads(some_other_func)), [other_overload])
        # Unrelated function still has no overloads:
        def not_overloaded(): pass
        self.assertEqual(list(get_overloads(not_overloaded)), [])

        # Make sure that after we clear all overloads, the registry is
        # completely empty.
        clear_overloads()
        self.assertEqual(typing._overload_registry, {})
        self.assertEqual(get_overloads(impl), [])

        # Querying a function with no overloads shouldn't change the registry.
        def the_only_one(): pass
        self.assertEqual(get_overloads(the_only_one), [])
        self.assertEqual(typing._overload_registry, {})

    def test_overload_registry_repeated(self):
        for _ in range(2):
            impl, overloads = self.set_up_overloads()

            self.assertEqual(list(get_overloads(impl)), overloads)


# Definitions needed for features introduced in Python 3.6

from test import ann_module, ann_module2, ann_module3, ann_module5, ann_module6
import asyncio

T_a = TypeVar('T_a')

class AwaitableWrapper(typing.Awaitable[T_a]):

    def __init__(self, value):
        self.value = value

    def __await__(self) -> typing.Iterator[T_a]:
        yield
        return self.value

class AsyncIteratorWrapper(typing.AsyncIterator[T_a]):

    def __init__(self, value: typing.Iterable[T_a]):
        self.value = value

    def __aiter__(self) -> typing.AsyncIterator[T_a]:
        return self

    async def __anext__(self) -> T_a:
        data = await self.value
        if data:
            return data
        else:
            raise StopAsyncIteration

class ACM:
    async def __aenter__(self) -> int:
        return 42
    async def __aexit__(self, etype, eval, tb):
        return None

class A:
    y: float
class B(A):
    x: ClassVar[Optional['B']] = None
    y: int
    b: int
class CSub(B):
    z: ClassVar['CSub'] = B()
class G(Generic[T]):
    lst: ClassVar[List[T]] = []

class Loop:
    attr: Final['Loop']

class NoneAndForward:
    parent: 'NoneAndForward'
    meaning: None

class CoolEmployee(NamedTuple):
    name: str
    cool: int

class CoolEmployeeWithDefault(NamedTuple):
    name: str
    cool: int = 0

class XMeth(NamedTuple):
    x: int
    def double(self):
        return 2 * self.x

class XRepr(NamedTuple):
    x: int
    y: int = 1
    def __str__(self):
        return f'{self.x} -> {self.y}'
    def __add__(self, other):
        return 0

Label = TypedDict('Label', [('label', str)])

class Point2D(TypedDict):
    x: int
    y: int

class Point2DGeneric(Generic[T], TypedDict):
    a: T
    b: T

class Bar(_typed_dict_helper.Foo, total=False):
    b: int

class BarGeneric(_typed_dict_helper.FooGeneric[T], total=False):
    b: int

class LabelPoint2D(Point2D, Label): ...

class Options(TypedDict, total=False):
    log_level: int
    log_path: str

class TotalMovie(TypedDict):
    title: str
    year: NotRequired[int]

class NontotalMovie(TypedDict, total=False):
    title: Required[str]
    year: int

class ParentNontotalMovie(TypedDict, total=False):
    title: Required[str]

class ChildTotalMovie(ParentNontotalMovie):
    year: NotRequired[int]

class ParentDeeplyAnnotatedMovie(TypedDict):
    title: Annotated[Annotated[Required[str], "foobar"], "another level"]

class ChildDeeplyAnnotatedMovie(ParentDeeplyAnnotatedMovie):
    year: NotRequired[Annotated[int, 2000]]

class AnnotatedMovie(TypedDict):
    title: Annotated[Required[str], "foobar"]
    year: NotRequired[Annotated[int, 2000]]

class DeeplyAnnotatedMovie(TypedDict):
    title: Annotated[Annotated[Required[str], "foobar"], "another level"]
    year: NotRequired[Annotated[int, 2000]]

class WeirdlyQuotedMovie(TypedDict):
    title: Annotated['Annotated[Required[str], "foobar"]', "another level"]
    year: NotRequired['Annotated[int, 2000]']

class HasForeignBaseClass(mod_generics_cache.A):
    some_xrepr: 'XRepr'
    other_a: 'mod_generics_cache.A'

async def g_with(am: typing.AsyncContextManager[int]):
    x: int
    async with am as x:
        return x

try:
    g_with(ACM()).send(None)
except StopIteration as e:
    assert e.args[0] == 42

gth = get_type_hints

class ForRefExample:
    @ann_module.dec
    def func(self: 'ForRefExample'):
        pass

    @ann_module.dec
    @ann_module.dec
    def nested(self: 'ForRefExample'):
        pass


class GetTypeHintTests(BaseTestCase):
    def test_get_type_hints_from_various_objects(self):
        # For invalid objects should fail with TypeError (not AttributeError etc).
        with self.assertRaises(TypeError):
            gth(123)
        with self.assertRaises(TypeError):
            gth('abc')
        with self.assertRaises(TypeError):
            gth(None)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_get_type_hints_modules(self):
        ann_module_type_hints = {1: 2, 'f': Tuple[int, int], 'x': int, 'y': str, 'u': int | float}
        self.assertEqual(gth(ann_module), ann_module_type_hints)
        self.assertEqual(gth(ann_module2), {})
        self.assertEqual(gth(ann_module3), {})

    @skip("known bug")
    def test_get_type_hints_modules_forwardref(self):
        # FIXME: This currently exposes a bug in typing. Cached forward references
        # don't account for the case where there are multiple types of the same
        # name coming from different modules in the same program.
        mgc_hints = {'default_a': Optional[mod_generics_cache.A],
                     'default_b': Optional[mod_generics_cache.B]}
        self.assertEqual(gth(mod_generics_cache), mgc_hints)

    def test_get_type_hints_classes(self):
        self.assertEqual(gth(ann_module.C),  # gth will find the right globalns
                         {'y': Optional[ann_module.C]})
        self.assertIsInstance(gth(ann_module.j_class), dict)
        self.assertEqual(gth(ann_module.M), {'123': 123, 'o': type})
        self.assertEqual(gth(ann_module.D),
                         {'j': str, 'k': str, 'y': Optional[ann_module.C]})
        self.assertEqual(gth(ann_module.Y), {'z': int})
        self.assertEqual(gth(ann_module.h_class),
                         {'y': Optional[ann_module.C]})
        self.assertEqual(gth(ann_module.S), {'x': str, 'y': str})
        self.assertEqual(gth(ann_module.foo), {'x': int})
        self.assertEqual(gth(NoneAndForward),
                         {'parent': NoneAndForward, 'meaning': type(None)})
        self.assertEqual(gth(HasForeignBaseClass),
                         {'some_xrepr': XRepr, 'other_a': mod_generics_cache.A,
                          'some_b': mod_generics_cache.B})
        self.assertEqual(gth(XRepr.__new__),
                         {'x': int, 'y': int})
        self.assertEqual(gth(mod_generics_cache.B),
                         {'my_inner_a1': mod_generics_cache.B.A,
                          'my_inner_a2': mod_generics_cache.B.A,
                          'my_outer_a': mod_generics_cache.A})

    def test_get_type_hints_classes_no_implicit_optional(self):
        class WithNoneDefault:
            field: int = None  # most type-checkers won't be happy with it

        self.assertEqual(gth(WithNoneDefault), {'field': int})

    def test_respect_no_type_check(self):
        @no_type_check
        class NoTpCheck:
            class Inn:
                def __init__(self, x: 'not a type'): ...
        self.assertTrue(NoTpCheck.__no_type_check__)
        self.assertTrue(NoTpCheck.Inn.__init__.__no_type_check__)
        self.assertEqual(gth(ann_module2.NTC.meth), {})
        class ABase(Generic[T]):
            def meth(x: int): ...
        @no_type_check
        class Der(ABase): ...
        self.assertEqual(gth(ABase.meth), {'x': int})

    def test_get_type_hints_for_builtins(self):
        # Should not fail for built-in classes and functions.
        self.assertEqual(gth(int), {})
        self.assertEqual(gth(type), {})
        self.assertEqual(gth(dir), {})
        self.assertEqual(gth(len), {})
        self.assertEqual(gth(object.__str__), {})
        self.assertEqual(gth(object().__str__), {})
        self.assertEqual(gth(str.join), {})

    def test_previous_behavior(self):
        def testf(x, y): ...
        testf.__annotations__['x'] = 'int'
        self.assertEqual(gth(testf), {'x': int})
        def testg(x: None): ...
        self.assertEqual(gth(testg), {'x': type(None)})

    def test_get_type_hints_for_object_with_annotations(self):
        class A: ...
        class B: ...
        b = B()
        b.__annotations__ = {'x': 'A'}
        self.assertEqual(gth(b, locals()), {'x': A})

    def test_get_type_hints_ClassVar(self):
        self.assertEqual(gth(ann_module2.CV, ann_module2.__dict__),
                         {'var': typing.ClassVar[ann_module2.CV]})
        self.assertEqual(gth(B, globals()),
                         {'y': int, 'x': ClassVar[Optional[B]], 'b': int})
        self.assertEqual(gth(CSub, globals()),
                         {'z': ClassVar[CSub], 'y': int, 'b': int,
                          'x': ClassVar[Optional[B]]})
        self.assertEqual(gth(G), {'lst': ClassVar[List[T]]})

    def test_get_type_hints_wrapped_decoratored_func(self):
        expects = {'self': ForRefExample}
        self.assertEqual(gth(ForRefExample.func), expects)
        self.assertEqual(gth(ForRefExample.nested), expects)

    def test_get_type_hints_annotated(self):
        def foobar(x: List['X']): ...
        X = Annotated[int, (1, 10)]
        self.assertEqual(
            get_type_hints(foobar, globals(), locals()),
            {'x': List[int]}
        )
        self.assertEqual(
            get_type_hints(foobar, globals(), locals(), include_extras=True),
            {'x': List[Annotated[int, (1, 10)]]}
        )

        def foobar(x: list[ForwardRef('X')]): ...
        X = Annotated[int, (1, 10)]
        self.assertEqual(
            get_type_hints(foobar, globals(), locals()),
            {'x': list[int]}
        )
        self.assertEqual(
            get_type_hints(foobar, globals(), locals(), include_extras=True),
            {'x': list[Annotated[int, (1, 10)]]}
        )

        BA = Tuple[Annotated[T, (1, 0)], ...]
        def barfoo(x: BA): ...
        self.assertEqual(get_type_hints(barfoo, globals(), locals())['x'], Tuple[T, ...])
        self.assertEqual(
            get_type_hints(barfoo, globals(), locals(), include_extras=True)['x'],
            BA
        )

        BA = tuple[Annotated[T, (1, 0)], ...]
        def barfoo(x: BA): ...
        self.assertEqual(get_type_hints(barfoo, globals(), locals())['x'], tuple[T, ...])
        self.assertEqual(
            get_type_hints(barfoo, globals(), locals(), include_extras=True)['x'],
            BA
        )

        def barfoo2(x: typing.Callable[..., Annotated[List[T], "const"]],
                    y: typing.Union[int, Annotated[T, "mutable"]]): ...
        self.assertEqual(
            get_type_hints(barfoo2, globals(), locals()),
            {'x': typing.Callable[..., List[T]], 'y': typing.Union[int, T]}
        )

        BA2 = typing.Callable[..., List[T]]
        def barfoo3(x: BA2): ...
        self.assertIs(
            get_type_hints(barfoo3, globals(), locals(), include_extras=True)["x"],
            BA2
        )
        BA3 = typing.Annotated[int | float, "const"]
        def barfoo4(x: BA3): ...
        self.assertEqual(
            get_type_hints(barfoo4, globals(), locals()),
            {"x": int | float}
        )
        self.assertEqual(
            get_type_hints(barfoo4, globals(), locals(), include_extras=True),
            {"x": typing.Annotated[int | float, "const"]}
        )

    def test_get_type_hints_annotated_in_union(self):  # bpo-46603
        def with_union(x: int | list[Annotated[str, 'meta']]): ...

        self.assertEqual(get_type_hints(with_union), {'x': int | list[str]})
        self.assertEqual(
            get_type_hints(with_union, include_extras=True),
            {'x': int | list[Annotated[str, 'meta']]},
        )

    def test_get_type_hints_annotated_refs(self):

        Const = Annotated[T, "Const"]

        class MySet(Generic[T]):

            def __ior__(self, other: "Const[MySet[T]]") -> "MySet[T]":
                ...

            def __iand__(self, other: Const["MySet[T]"]) -> "MySet[T]":
                ...

        self.assertEqual(
            get_type_hints(MySet.__iand__, globals(), locals()),
            {'other': MySet[T], 'return': MySet[T]}
        )

        self.assertEqual(
            get_type_hints(MySet.__iand__, globals(), locals(), include_extras=True),
            {'other': Const[MySet[T]], 'return': MySet[T]}
        )

        self.assertEqual(
            get_type_hints(MySet.__ior__, globals(), locals()),
            {'other': MySet[T], 'return': MySet[T]}
        )

    def test_get_type_hints_annotated_with_none_default(self):
        # See: https://bugs.python.org/issue46195
        def annotated_with_none_default(x: Annotated[int, 'data'] = None): ...
        self.assertEqual(
            get_type_hints(annotated_with_none_default),
            {'x': int},
        )
        self.assertEqual(
            get_type_hints(annotated_with_none_default, include_extras=True),
            {'x': Annotated[int, 'data']},
        )

    def test_get_type_hints_classes_str_annotations(self):
        class Foo:
            y = str
            x: 'y'
        # This previously raised an error under PEP 563.
        self.assertEqual(get_type_hints(Foo), {'x': str})

    def test_get_type_hints_bad_module(self):
        # bpo-41515
        class BadModule:
            pass
        BadModule.__module__ = 'bad' # Something not in sys.modules
        self.assertNotIn('bad', sys.modules)
        self.assertEqual(get_type_hints(BadModule), {})

    def test_get_type_hints_annotated_bad_module(self):
        # See https://bugs.python.org/issue44468
        class BadBase:
            foo: tuple
        class BadType(BadBase):
            bar: list
        BadType.__module__ = BadBase.__module__ = 'bad'
        self.assertNotIn('bad', sys.modules)
        self.assertEqual(get_type_hints(BadType), {'foo': tuple, 'bar': list})

    def test_forward_ref_and_final(self):
        # https://bugs.python.org/issue45166
        hints = get_type_hints(ann_module5)
        self.assertEqual(hints, {'name': Final[str]})

        hints = get_type_hints(ann_module5.MyClass)
        self.assertEqual(hints, {'value': Final})

    def test_top_level_class_var(self):
        # https://bugs.python.org/issue45166
        with self.assertRaisesRegex(
            TypeError,
            r'typing.ClassVar\[int\] is not valid as type argument',
        ):
            get_type_hints(ann_module6)

    def test_get_type_hints_typeddict(self):
        self.assertEqual(get_type_hints(TotalMovie), {'title': str, 'year': int})
        self.assertEqual(get_type_hints(TotalMovie, include_extras=True), {
            'title': str,
            'year': NotRequired[int],
        })

        self.assertEqual(get_type_hints(AnnotatedMovie), {'title': str, 'year': int})
        self.assertEqual(get_type_hints(AnnotatedMovie, include_extras=True), {
            'title': Annotated[Required[str], "foobar"],
            'year': NotRequired[Annotated[int, 2000]],
        })

        self.assertEqual(get_type_hints(DeeplyAnnotatedMovie), {'title': str, 'year': int})
        self.assertEqual(get_type_hints(DeeplyAnnotatedMovie, include_extras=True), {
            'title': Annotated[Required[str], "foobar", "another level"],
            'year': NotRequired[Annotated[int, 2000]],
        })

        self.assertEqual(get_type_hints(WeirdlyQuotedMovie), {'title': str, 'year': int})
        self.assertEqual(get_type_hints(WeirdlyQuotedMovie, include_extras=True), {
            'title': Annotated[Required[str], "foobar", "another level"],
            'year': NotRequired[Annotated[int, 2000]],
        })

        self.assertEqual(get_type_hints(_typed_dict_helper.VeryAnnotated), {'a': int})
        self.assertEqual(get_type_hints(_typed_dict_helper.VeryAnnotated, include_extras=True), {
            'a': Annotated[Required[int], "a", "b", "c"]
        })

        self.assertEqual(get_type_hints(ChildTotalMovie), {"title": str, "year": int})
        self.assertEqual(get_type_hints(ChildTotalMovie, include_extras=True), {
            "title": Required[str], "year": NotRequired[int]
        })

        self.assertEqual(get_type_hints(ChildDeeplyAnnotatedMovie), {"title": str, "year": int})
        self.assertEqual(get_type_hints(ChildDeeplyAnnotatedMovie, include_extras=True), {
            "title": Annotated[Required[str], "foobar", "another level"],
            "year": NotRequired[Annotated[int, 2000]]
        })

    def test_get_type_hints_collections_abc_callable(self):
        # https://github.com/python/cpython/issues/91621
        P = ParamSpec('P')
        def f(x: collections.abc.Callable[[int], int]): ...
        def g(x: collections.abc.Callable[..., int]): ...
        def h(x: collections.abc.Callable[P, int]): ...

        self.assertEqual(get_type_hints(f), {'x': collections.abc.Callable[[int], int]})
        self.assertEqual(get_type_hints(g), {'x': collections.abc.Callable[..., int]})
        self.assertEqual(get_type_hints(h), {'x': collections.abc.Callable[P, int]})


class GetUtilitiesTestCase(TestCase):
    def test_get_origin(self):
        T = TypeVar('T')
        Ts = TypeVarTuple('Ts')
        P = ParamSpec('P')
        class C(Generic[T]): pass
        self.assertIs(get_origin(C[int]), C)
        self.assertIs(get_origin(C[T]), C)
        self.assertIs(get_origin(int), None)
        self.assertIs(get_origin(ClassVar[int]), ClassVar)
        self.assertIs(get_origin(Union[int, str]), Union)
        self.assertIs(get_origin(Literal[42, 43]), Literal)
        self.assertIs(get_origin(Final[List[int]]), Final)
        self.assertIs(get_origin(Generic), Generic)
        self.assertIs(get_origin(Generic[T]), Generic)
        self.assertIs(get_origin(List[Tuple[T, T]][int]), list)
        self.assertIs(get_origin(Annotated[T, 'thing']), Annotated)
        self.assertIs(get_origin(List), list)
        self.assertIs(get_origin(Tuple), tuple)
        self.assertIs(get_origin(Callable), collections.abc.Callable)
        self.assertIs(get_origin(list[int]), list)
        self.assertIs(get_origin(list), None)
        self.assertIs(get_origin(list | str), types.UnionType)
        self.assertIs(get_origin(P.args), P)
        self.assertIs(get_origin(P.kwargs), P)
        self.assertIs(get_origin(Required[int]), Required)
        self.assertIs(get_origin(NotRequired[int]), NotRequired)
        self.assertIs(get_origin((*Ts,)[0]), Unpack)
        self.assertIs(get_origin(Unpack[Ts]), Unpack)
        self.assertIs(get_origin((*tuple[*Ts],)[0]), tuple)
        self.assertIs(get_origin(Unpack[Tuple[Unpack[Ts]]]), Unpack)

    def test_get_args(self):
        T = TypeVar('T')
        class C(Generic[T]): pass
        self.assertEqual(get_args(C[int]), (int,))
        self.assertEqual(get_args(C[T]), (T,))
        self.assertEqual(get_args(int), ())
        self.assertEqual(get_args(ClassVar[int]), (int,))
        self.assertEqual(get_args(Union[int, str]), (int, str))
        self.assertEqual(get_args(Literal[42, 43]), (42, 43))
        self.assertEqual(get_args(Final[List[int]]), (List[int],))
        self.assertEqual(get_args(Union[int, Tuple[T, int]][str]),
                         (int, Tuple[str, int]))
        self.assertEqual(get_args(typing.Dict[int, Tuple[T, T]][Optional[int]]),
                         (int, Tuple[Optional[int], Optional[int]]))
        self.assertEqual(get_args(Callable[[], T][int]), ([], int))
        self.assertEqual(get_args(Callable[..., int]), (..., int))
        self.assertEqual(get_args(Union[int, Callable[[Tuple[T, ...]], str]]),
                         (int, Callable[[Tuple[T, ...]], str]))
        self.assertEqual(get_args(Tuple[int, ...]), (int, ...))
        self.assertEqual(get_args(Tuple[()]), ())
        self.assertEqual(get_args(Annotated[T, 'one', 2, ['three']]), (T, 'one', 2, ['three']))
        self.assertEqual(get_args(List), ())
        self.assertEqual(get_args(Tuple), ())
        self.assertEqual(get_args(Callable), ())
        self.assertEqual(get_args(list[int]), (int,))
        self.assertEqual(get_args(list), ())
        self.assertEqual(get_args(collections.abc.Callable[[int], str]), ([int], str))
        self.assertEqual(get_args(collections.abc.Callable[..., str]), (..., str))
        self.assertEqual(get_args(collections.abc.Callable[[], str]), ([], str))
        self.assertEqual(get_args(collections.abc.Callable[[int], str]),
                         get_args(Callable[[int], str]))
        P = ParamSpec('P')
        self.assertEqual(get_args(Callable[P, int]), (P, int))
        self.assertEqual(get_args(Callable[Concatenate[int, P], int]),
                         (Concatenate[int, P], int))
        self.assertEqual(get_args(list | str), (list, str))
        self.assertEqual(get_args(Required[int]), (int,))
        self.assertEqual(get_args(NotRequired[int]), (int,))
        self.assertEqual(get_args(TypeAlias), ())
        self.assertEqual(get_args(TypeGuard[int]), (int,))
        Ts = TypeVarTuple('Ts')
        self.assertEqual(get_args(Ts), ())
        self.assertEqual(get_args((*Ts,)[0]), (Ts,))
        self.assertEqual(get_args(Unpack[Ts]), (Ts,))
        self.assertEqual(get_args(tuple[*Ts]), (*Ts,))
        self.assertEqual(get_args(tuple[Unpack[Ts]]), (Unpack[Ts],))
        self.assertEqual(get_args((*tuple[*Ts],)[0]), (*Ts,))
        self.assertEqual(get_args(Unpack[tuple[Unpack[Ts]]]), (tuple[Unpack[Ts]],))


class CollectionsAbcTests(BaseTestCase):

    def test_hashable(self):
        self.assertIsInstance(42, typing.Hashable)
        self.assertNotIsInstance([], typing.Hashable)

    def test_iterable(self):
        self.assertIsInstance([], typing.Iterable)
        # Due to ABC caching, the second time takes a separate code
        # path and could fail.  So call this a few times.
        self.assertIsInstance([], typing.Iterable)
        self.assertIsInstance([], typing.Iterable)
        self.assertNotIsInstance(42, typing.Iterable)
        # Just in case, also test issubclass() a few times.
        self.assertIsSubclass(list, typing.Iterable)
        self.assertIsSubclass(list, typing.Iterable)

    def test_iterator(self):
        it = iter([])
        self.assertIsInstance(it, typing.Iterator)
        self.assertNotIsInstance(42, typing.Iterator)

    def test_awaitable(self):
        ns = {}
        exec(
            "async def foo() -> typing.Awaitable[int]:\n"
            "    return await AwaitableWrapper(42)\n",
            globals(), ns)
        foo = ns['foo']
        g = foo()
        self.assertIsInstance(g, typing.Awaitable)
        self.assertNotIsInstance(foo, typing.Awaitable)
        g.send(None)  # Run foo() till completion, to avoid warning.

    def test_coroutine(self):
        ns = {}
        exec(
            "async def foo():\n"
            "    return\n",
            globals(), ns)
        foo = ns['foo']
        g = foo()
        self.assertIsInstance(g, typing.Coroutine)
        with self.assertRaises(TypeError):
            isinstance(g, typing.Coroutine[int])
        self.assertNotIsInstance(foo, typing.Coroutine)
        try:
            g.send(None)
        except StopIteration:
            pass

    def test_async_iterable(self):
        base_it = range(10)  # type: Iterator[int]
        it = AsyncIteratorWrapper(base_it)
        self.assertIsInstance(it, typing.AsyncIterable)
        self.assertIsInstance(it, typing.AsyncIterable)
        self.assertNotIsInstance(42, typing.AsyncIterable)

    def test_async_iterator(self):
        base_it = range(10)  # type: Iterator[int]
        it = AsyncIteratorWrapper(base_it)
        self.assertIsInstance(it, typing.AsyncIterator)
        self.assertNotIsInstance(42, typing.AsyncIterator)

    def test_sized(self):
        self.assertIsInstance([], typing.Sized)
        self.assertNotIsInstance(42, typing.Sized)

    def test_container(self):
        self.assertIsInstance([], typing.Container)
        self.assertNotIsInstance(42, typing.Container)

    def test_collection(self):
        self.assertIsInstance(tuple(), typing.Collection)
        self.assertIsInstance(frozenset(), typing.Collection)
        self.assertIsSubclass(dict, typing.Collection)
        self.assertNotIsInstance(42, typing.Collection)

    def test_abstractset(self):
        self.assertIsInstance(set(), typing.AbstractSet)
        self.assertNotIsInstance(42, typing.AbstractSet)

    def test_mutableset(self):
        self.assertIsInstance(set(), typing.MutableSet)
        self.assertNotIsInstance(frozenset(), typing.MutableSet)

    def test_mapping(self):
        self.assertIsInstance({}, typing.Mapping)
        self.assertNotIsInstance(42, typing.Mapping)

    def test_mutablemapping(self):
        self.assertIsInstance({}, typing.MutableMapping)
        self.assertNotIsInstance(42, typing.MutableMapping)

    def test_sequence(self):
        self.assertIsInstance([], typing.Sequence)
        self.assertNotIsInstance(42, typing.Sequence)

    def test_mutablesequence(self):
        self.assertIsInstance([], typing.MutableSequence)
        self.assertNotIsInstance((), typing.MutableSequence)

    def test_bytestring(self):
        self.assertIsInstance(b'', typing.ByteString)
        self.assertIsInstance(bytearray(b''), typing.ByteString)

    def test_list(self):
        self.assertIsSubclass(list, typing.List)

    def test_deque(self):
        self.assertIsSubclass(collections.deque, typing.Deque)
        class MyDeque(typing.Deque[int]): ...
        self.assertIsInstance(MyDeque(), collections.deque)

    def test_counter(self):
        self.assertIsSubclass(collections.Counter, typing.Counter)

    def test_set(self):
        self.assertIsSubclass(set, typing.Set)
        self.assertNotIsSubclass(frozenset, typing.Set)

    def test_frozenset(self):
        self.assertIsSubclass(frozenset, typing.FrozenSet)
        self.assertNotIsSubclass(set, typing.FrozenSet)

    def test_dict(self):
        self.assertIsSubclass(dict, typing.Dict)

    def test_dict_subscribe(self):
        K = TypeVar('K')
        V = TypeVar('V')
        self.assertEqual(Dict[K, V][str, int], Dict[str, int])
        self.assertEqual(Dict[K, int][str], Dict[str, int])
        self.assertEqual(Dict[str, V][int], Dict[str, int])
        self.assertEqual(Dict[K, List[V]][str, int], Dict[str, List[int]])
        self.assertEqual(Dict[K, List[int]][str], Dict[str, List[int]])
        self.assertEqual(Dict[K, list[V]][str, int], Dict[str, list[int]])
        self.assertEqual(Dict[K, list[int]][str], Dict[str, list[int]])

    def test_no_list_instantiation(self):
        with self.assertRaises(TypeError):
            typing.List()
        with self.assertRaises(TypeError):
            typing.List[T]()
        with self.assertRaises(TypeError):
            typing.List[int]()

    def test_list_subclass(self):

        class MyList(typing.List[int]):
            pass

        a = MyList()
        self.assertIsInstance(a, MyList)
        self.assertIsInstance(a, typing.Sequence)

        self.assertIsSubclass(MyList, list)
        self.assertNotIsSubclass(list, MyList)

    def test_no_dict_instantiation(self):
        with self.assertRaises(TypeError):
            typing.Dict()
        with self.assertRaises(TypeError):
            typing.Dict[KT, VT]()
        with self.assertRaises(TypeError):
            typing.Dict[str, int]()

    def test_dict_subclass(self):

        class MyDict(typing.Dict[str, int]):
            pass

        d = MyDict()
        self.assertIsInstance(d, MyDict)
        self.assertIsInstance(d, typing.MutableMapping)

        self.assertIsSubclass(MyDict, dict)
        self.assertNotIsSubclass(dict, MyDict)

    def test_defaultdict_instantiation(self):
        self.assertIs(type(typing.DefaultDict()), collections.defaultdict)
        self.assertIs(type(typing.DefaultDict[KT, VT]()), collections.defaultdict)
        self.assertIs(type(typing.DefaultDict[str, int]()), collections.defaultdict)

    def test_defaultdict_subclass(self):

        class MyDefDict(typing.DefaultDict[str, int]):
            pass

        dd = MyDefDict()
        self.assertIsInstance(dd, MyDefDict)

        self.assertIsSubclass(MyDefDict, collections.defaultdict)
        self.assertNotIsSubclass(collections.defaultdict, MyDefDict)

    def test_ordereddict_instantiation(self):
        self.assertIs(type(typing.OrderedDict()), collections.OrderedDict)
        self.assertIs(type(typing.OrderedDict[KT, VT]()), collections.OrderedDict)
        self.assertIs(type(typing.OrderedDict[str, int]()), collections.OrderedDict)

    def test_ordereddict_subclass(self):

        class MyOrdDict(typing.OrderedDict[str, int]):
            pass

        od = MyOrdDict()
        self.assertIsInstance(od, MyOrdDict)

        self.assertIsSubclass(MyOrdDict, collections.OrderedDict)
        self.assertNotIsSubclass(collections.OrderedDict, MyOrdDict)

    def test_chainmap_instantiation(self):
        self.assertIs(type(typing.ChainMap()), collections.ChainMap)
        self.assertIs(type(typing.ChainMap[KT, VT]()), collections.ChainMap)
        self.assertIs(type(typing.ChainMap[str, int]()), collections.ChainMap)
        class CM(typing.ChainMap[KT, VT]): ...
        self.assertIs(type(CM[int, str]()), CM)

    def test_chainmap_subclass(self):

        class MyChainMap(typing.ChainMap[str, int]):
            pass

        cm = MyChainMap()
        self.assertIsInstance(cm, MyChainMap)

        self.assertIsSubclass(MyChainMap, collections.ChainMap)
        self.assertNotIsSubclass(collections.ChainMap, MyChainMap)

    def test_deque_instantiation(self):
        self.assertIs(type(typing.Deque()), collections.deque)
        self.assertIs(type(typing.Deque[T]()), collections.deque)
        self.assertIs(type(typing.Deque[int]()), collections.deque)
        class D(typing.Deque[T]): ...
        self.assertIs(type(D[int]()), D)

    def test_counter_instantiation(self):
        self.assertIs(type(typing.Counter()), collections.Counter)
        self.assertIs(type(typing.Counter[T]()), collections.Counter)
        self.assertIs(type(typing.Counter[int]()), collections.Counter)
        class C(typing.Counter[T]): ...
        self.assertIs(type(C[int]()), C)

    def test_counter_subclass_instantiation(self):

        class MyCounter(typing.Counter[int]):
            pass

        d = MyCounter()
        self.assertIsInstance(d, MyCounter)
        self.assertIsInstance(d, typing.Counter)
        self.assertIsInstance(d, collections.Counter)

    def test_no_set_instantiation(self):
        with self.assertRaises(TypeError):
            typing.Set()
        with self.assertRaises(TypeError):
            typing.Set[T]()
        with self.assertRaises(TypeError):
            typing.Set[int]()

    def test_set_subclass_instantiation(self):

        class MySet(typing.Set[int]):
            pass

        d = MySet()
        self.assertIsInstance(d, MySet)

    def test_no_frozenset_instantiation(self):
        with self.assertRaises(TypeError):
            typing.FrozenSet()
        with self.assertRaises(TypeError):
            typing.FrozenSet[T]()
        with self.assertRaises(TypeError):
            typing.FrozenSet[int]()

    def test_frozenset_subclass_instantiation(self):

        class MyFrozenSet(typing.FrozenSet[int]):
            pass

        d = MyFrozenSet()
        self.assertIsInstance(d, MyFrozenSet)

    def test_no_tuple_instantiation(self):
        with self.assertRaises(TypeError):
            Tuple()
        with self.assertRaises(TypeError):
            Tuple[T]()
        with self.assertRaises(TypeError):
            Tuple[int]()

    def test_generator(self):
        def foo():
            yield 42
        g = foo()
        self.assertIsSubclass(type(g), typing.Generator)

    def test_no_generator_instantiation(self):
        with self.assertRaises(TypeError):
            typing.Generator()
        with self.assertRaises(TypeError):
            typing.Generator[T, T, T]()
        with self.assertRaises(TypeError):
            typing.Generator[int, int, int]()

    def test_async_generator(self):
        ns = {}
        exec("async def f():\n"
             "    yield 42\n", globals(), ns)
        g = ns['f']()
        self.assertIsSubclass(type(g), typing.AsyncGenerator)

    def test_no_async_generator_instantiation(self):
        with self.assertRaises(TypeError):
            typing.AsyncGenerator()
        with self.assertRaises(TypeError):
            typing.AsyncGenerator[T, T]()
        with self.assertRaises(TypeError):
            typing.AsyncGenerator[int, int]()

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_subclassing(self):

        class MMA(typing.MutableMapping):
            pass

        with self.assertRaises(TypeError):  # It's abstract
            MMA()

        class MMC(MMA):
            def __getitem__(self, k):
                return None
            def __setitem__(self, k, v):
                pass
            def __delitem__(self, k):
                pass
            def __iter__(self):
                return iter(())
            def __len__(self):
                return 0

        self.assertEqual(len(MMC()), 0)
        assert callable(MMC.update)
        self.assertIsInstance(MMC(), typing.Mapping)

        class MMB(typing.MutableMapping[KT, VT]):
            def __getitem__(self, k):
                return None
            def __setitem__(self, k, v):
                pass
            def __delitem__(self, k):
                pass
            def __iter__(self):
                return iter(())
            def __len__(self):
                return 0

        self.assertEqual(len(MMB()), 0)
        self.assertEqual(len(MMB[str, str]()), 0)
        self.assertEqual(len(MMB[KT, VT]()), 0)

        self.assertNotIsSubclass(dict, MMA)
        self.assertNotIsSubclass(dict, MMB)

        self.assertIsSubclass(MMA, typing.Mapping)
        self.assertIsSubclass(MMB, typing.Mapping)
        self.assertIsSubclass(MMC, typing.Mapping)

        self.assertIsInstance(MMB[KT, VT](), typing.Mapping)
        self.assertIsInstance(MMB[KT, VT](), collections.abc.Mapping)

        self.assertIsSubclass(MMA, collections.abc.Mapping)
        self.assertIsSubclass(MMB, collections.abc.Mapping)
        self.assertIsSubclass(MMC, collections.abc.Mapping)

        with self.assertRaises(TypeError):
            issubclass(MMB[str, str], typing.Mapping)
        self.assertIsSubclass(MMC, MMA)

        class I(typing.Iterable): ...
        self.assertNotIsSubclass(list, I)

        class G(typing.Generator[int, int, int]): ...
        def g(): yield 0
        self.assertIsSubclass(G, typing.Generator)
        self.assertIsSubclass(G, typing.Iterable)
        self.assertIsSubclass(G, collections.abc.Generator)
        self.assertIsSubclass(G, collections.abc.Iterable)
        self.assertNotIsSubclass(type(g), G)

    def test_subclassing_async_generator(self):
        class G(typing.AsyncGenerator[int, int]):
            def asend(self, value):
                pass
            def athrow(self, typ, val=None, tb=None):
                pass

        ns = {}
        exec('async def g(): yield 0', globals(), ns)
        g = ns['g']
        self.assertIsSubclass(G, typing.AsyncGenerator)
        self.assertIsSubclass(G, typing.AsyncIterable)
        self.assertIsSubclass(G, collections.abc.AsyncGenerator)
        self.assertIsSubclass(G, collections.abc.AsyncIterable)
        self.assertNotIsSubclass(type(g), G)

        instance = G()
        self.assertIsInstance(instance, typing.AsyncGenerator)
        self.assertIsInstance(instance, typing.AsyncIterable)
        self.assertIsInstance(instance, collections.abc.AsyncGenerator)
        self.assertIsInstance(instance, collections.abc.AsyncIterable)
        self.assertNotIsInstance(type(g), G)
        self.assertNotIsInstance(g, G)

    def test_subclassing_subclasshook(self):

        class Base(typing.Iterable):
            @classmethod
            def __subclasshook__(cls, other):
                if other.__name__ == 'Foo':
                    return True
                else:
                    return False

        class C(Base): ...
        class Foo: ...
        class Bar: ...
        self.assertIsSubclass(Foo, Base)
        self.assertIsSubclass(Foo, C)
        self.assertNotIsSubclass(Bar, C)

    def test_subclassing_register(self):

        class A(typing.Container): ...
        class B(A): ...

        class C: ...
        A.register(C)
        self.assertIsSubclass(C, A)
        self.assertNotIsSubclass(C, B)

        class D: ...
        B.register(D)
        self.assertIsSubclass(D, A)
        self.assertIsSubclass(D, B)

        class M(): ...
        collections.abc.MutableMapping.register(M)
        self.assertIsSubclass(M, typing.Mapping)

    def test_collections_as_base(self):

        class M(collections.abc.Mapping): ...
        self.assertIsSubclass(M, typing.Mapping)
        self.assertIsSubclass(M, typing.Iterable)

        class S(collections.abc.MutableSequence): ...
        self.assertIsSubclass(S, typing.MutableSequence)
        self.assertIsSubclass(S, typing.Iterable)

        class I(collections.abc.Iterable): ...
        self.assertIsSubclass(I, typing.Iterable)

        class A(collections.abc.Mapping, metaclass=abc.ABCMeta): ...
        class B: ...
        A.register(B)
        self.assertIsSubclass(B, typing.Mapping)

    def test_or_and_ror(self):
        self.assertEqual(typing.Sized | typing.Awaitable, Union[typing.Sized, typing.Awaitable])
        self.assertEqual(typing.Coroutine | typing.Hashable, Union[typing.Coroutine, typing.Hashable])


class OtherABCTests(BaseTestCase):

    def test_contextmanager(self):
        @contextlib.contextmanager
        def manager():
            yield 42

        cm = manager()
        self.assertIsInstance(cm, typing.ContextManager)
        self.assertNotIsInstance(42, typing.ContextManager)

    def test_async_contextmanager(self):
        class NotACM:
            pass
        self.assertIsInstance(ACM(), typing.AsyncContextManager)
        self.assertNotIsInstance(NotACM(), typing.AsyncContextManager)
        @contextlib.contextmanager
        def manager():
            yield 42

        cm = manager()
        self.assertNotIsInstance(cm, typing.AsyncContextManager)
        self.assertEqual(typing.AsyncContextManager[int].__args__, (int,))
        with self.assertRaises(TypeError):
            isinstance(42, typing.AsyncContextManager[int])
        with self.assertRaises(TypeError):
            typing.AsyncContextManager[int, str]


class TypeTests(BaseTestCase):

    def test_type_basic(self):

        class User: pass
        class BasicUser(User): pass
        class ProUser(User): pass

        def new_user(user_class: Type[User]) -> User:
            return user_class()

        new_user(BasicUser)

    def test_type_typevar(self):

        class User: pass
        class BasicUser(User): pass
        class ProUser(User): pass

        U = TypeVar('U', bound=User)

        def new_user(user_class: Type[U]) -> U:
            return user_class()

        new_user(BasicUser)

    def test_type_optional(self):
        A = Optional[Type[BaseException]]

        def foo(a: A) -> Optional[BaseException]:
            if a is None:
                return None
            else:
                return a()

        assert isinstance(foo(KeyboardInterrupt), KeyboardInterrupt)
        assert foo(None) is None


class TestModules(TestCase):
    func_names = ['_idfunc']

    def test_py_functions(self):
        for fname in self.func_names:
            self.assertEqual(getattr(py_typing, fname).__module__, 'typing')

    @skipUnless(c_typing, 'requires _typing')
    def test_c_functions(self):
        for fname in self.func_names:
            self.assertEqual(getattr(c_typing, fname).__module__, '_typing')


class NewTypeTests:
    def cleanup(self):
        for f in self.module._cleanups:
            f()

    @classmethod
    def setUpClass(cls):
        sys.modules['typing'] = cls.module
        global UserId
        UserId = cls.module.NewType('UserId', int)
        cls.UserName = cls.module.NewType(cls.__qualname__ + '.UserName', str)

    @classmethod
    def tearDownClass(cls):
        global UserId
        del UserId
        del cls.UserName
        sys.modules['typing'] = typing

    def tearDown(self):
        self.cleanup()

    def test_basic(self):
        self.assertIsInstance(UserId(5), int)
        self.assertIsInstance(self.UserName('Joe'), str)
        self.assertEqual(UserId(5) + 1, 6)

    def test_errors(self):
        with self.assertRaises(TypeError):
            issubclass(UserId, int)
        with self.assertRaises(TypeError):
            class D(UserId):
                pass

    def test_or(self):
        for cls in (int, self.UserName):
            with self.subTest(cls=cls):
                self.assertEqual(UserId | cls, self.module.Union[UserId, cls])
                self.assertEqual(cls | UserId, self.module.Union[cls, UserId])

                self.assertEqual(self.module.get_args(UserId | cls), (UserId, cls))
                self.assertEqual(self.module.get_args(cls | UserId), (cls, UserId))

    def test_special_attrs(self):
        self.assertEqual(UserId.__name__, 'UserId')
        self.assertEqual(UserId.__qualname__, 'UserId')
        self.assertEqual(UserId.__module__, __name__)
        self.assertEqual(UserId.__supertype__, int)

        UserName = self.UserName
        self.assertEqual(UserName.__name__, 'UserName')
        self.assertEqual(UserName.__qualname__,
                         self.__class__.__qualname__ + '.UserName')
        self.assertEqual(UserName.__module__, __name__)
        self.assertEqual(UserName.__supertype__, str)

    def test_repr(self):
        self.assertEqual(repr(UserId), f'{__name__}.UserId')
        self.assertEqual(repr(self.UserName),
                         f'{__name__}.{self.__class__.__qualname__}.UserName')

    def test_pickle(self):
        UserAge = self.module.NewType('UserAge', float)
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.subTest(proto=proto):
                pickled = pickle.dumps(UserId, proto)
                loaded = pickle.loads(pickled)
                self.assertIs(loaded, UserId)

                pickled = pickle.dumps(self.UserName, proto)
                loaded = pickle.loads(pickled)
                self.assertIs(loaded, self.UserName)

                with self.assertRaises(pickle.PicklingError):
                    pickle.dumps(UserAge, proto)

    def test_missing__name__(self):
        code = ("import typing\n"
                "NT = typing.NewType('NT', int)\n"
                )
        exec(code, {})

    def test_error_message_when_subclassing(self):
        with self.assertRaisesRegex(
            TypeError,
            re.escape(
                "Cannot subclass an instance of NewType. Perhaps you were looking for: "
                "`ProUserId = NewType('ProUserId', UserId)`"
            )
        ):
            class ProUserId(UserId):
                ...


class NewTypePythonTests(NewTypeTests, BaseTestCase):
    module = py_typing


@skipUnless(c_typing, 'requires _typing')
class NewTypeCTests(NewTypeTests, BaseTestCase):
    module = c_typing


class NamedTupleTests(BaseTestCase):
    class NestedEmployee(NamedTuple):
        name: str
        cool: int

    def test_basics(self):
        Emp = NamedTuple('Emp', [('name', str), ('id', int)])
        self.assertIsSubclass(Emp, tuple)
        joe = Emp('Joe', 42)
        jim = Emp(name='Jim', id=1)
        self.assertIsInstance(joe, Emp)
        self.assertIsInstance(joe, tuple)
        self.assertEqual(joe.name, 'Joe')
        self.assertEqual(joe.id, 42)
        self.assertEqual(jim.name, 'Jim')
        self.assertEqual(jim.id, 1)
        self.assertEqual(Emp.__name__, 'Emp')
        self.assertEqual(Emp._fields, ('name', 'id'))
        self.assertEqual(Emp.__annotations__,
                         collections.OrderedDict([('name', str), ('id', int)]))

    def test_annotation_usage(self):
        tim = CoolEmployee('Tim', 9000)
        self.assertIsInstance(tim, CoolEmployee)
        self.assertIsInstance(tim, tuple)
        self.assertEqual(tim.name, 'Tim')
        self.assertEqual(tim.cool, 9000)
        self.assertEqual(CoolEmployee.__name__, 'CoolEmployee')
        self.assertEqual(CoolEmployee._fields, ('name', 'cool'))
        self.assertEqual(CoolEmployee.__annotations__,
                         collections.OrderedDict(name=str, cool=int))

    def test_annotation_usage_with_default(self):
        jelle = CoolEmployeeWithDefault('Jelle')
        self.assertIsInstance(jelle, CoolEmployeeWithDefault)
        self.assertIsInstance(jelle, tuple)
        self.assertEqual(jelle.name, 'Jelle')
        self.assertEqual(jelle.cool, 0)
        cooler_employee = CoolEmployeeWithDefault('Sjoerd', 1)
        self.assertEqual(cooler_employee.cool, 1)

        self.assertEqual(CoolEmployeeWithDefault.__name__, 'CoolEmployeeWithDefault')
        self.assertEqual(CoolEmployeeWithDefault._fields, ('name', 'cool'))
        self.assertEqual(CoolEmployeeWithDefault.__annotations__,
                         dict(name=str, cool=int))
        self.assertEqual(CoolEmployeeWithDefault._field_defaults, dict(cool=0))

        with self.assertRaises(TypeError):
            class NonDefaultAfterDefault(NamedTuple):
                x: int = 3
                y: int

    def test_annotation_usage_with_methods(self):
        self.assertEqual(XMeth(1).double(), 2)
        self.assertEqual(XMeth(42).x, XMeth(42)[0])
        self.assertEqual(str(XRepr(42)), '42 -> 1')
        self.assertEqual(XRepr(1, 2) + XRepr(3), 0)

        with self.assertRaises(AttributeError):
            class XMethBad(NamedTuple):
                x: int
                def _fields(self):
                    return 'no chance for this'

        with self.assertRaises(AttributeError):
            class XMethBad2(NamedTuple):
                x: int
                def _source(self):
                    return 'no chance for this as well'

    def test_multiple_inheritance(self):
        class A:
            pass
        with self.assertRaises(TypeError):
            class X(NamedTuple, A):
                x: int
        with self.assertRaises(TypeError):
            class X(NamedTuple, tuple):
                x: int
        with self.assertRaises(TypeError):
            class X(NamedTuple, NamedTuple):
                x: int
        class A(NamedTuple):
            x: int
        with self.assertRaises(TypeError):
            class X(NamedTuple, A):
                y: str

    def test_generic(self):
        class X(NamedTuple, Generic[T]):
            x: T
        self.assertEqual(X.__bases__, (tuple, Generic))
        self.assertEqual(X.__orig_bases__, (NamedTuple, Generic[T]))
        self.assertEqual(X.__mro__, (X, tuple, Generic, object))

        class Y(Generic[T], NamedTuple):
            x: T
        self.assertEqual(Y.__bases__, (Generic, tuple))
        self.assertEqual(Y.__orig_bases__, (Generic[T], NamedTuple))
        self.assertEqual(Y.__mro__, (Y, Generic, tuple, object))

        for G in X, Y:
            with self.subTest(type=G):
                self.assertEqual(G.__parameters__, (T,))
                A = G[int]
                self.assertIs(A.__origin__, G)
                self.assertEqual(A.__args__, (int,))
                self.assertEqual(A.__parameters__, ())

                a = A(3)
                self.assertIs(type(a), G)
                self.assertEqual(a.x, 3)

                with self.assertRaises(TypeError):
                    G[int, str]

    def test_non_generic_subscript(self):
        # For backward compatibility, subscription works
        # on arbitrary NamedTuple types.
        class Group(NamedTuple):
            key: T
            group: list[T]
        A = Group[int]
        self.assertEqual(A.__origin__, Group)
        self.assertEqual(A.__parameters__, ())
        self.assertEqual(A.__args__, (int,))
        a = A(1, [2])
        self.assertIs(type(a), Group)
        self.assertEqual(a, (1, [2]))

    def test_namedtuple_keyword_usage(self):
        LocalEmployee = NamedTuple("LocalEmployee", name=str, age=int)
        nick = LocalEmployee('Nick', 25)
        self.assertIsInstance(nick, tuple)
        self.assertEqual(nick.name, 'Nick')
        self.assertEqual(LocalEmployee.__name__, 'LocalEmployee')
        self.assertEqual(LocalEmployee._fields, ('name', 'age'))
        self.assertEqual(LocalEmployee.__annotations__, dict(name=str, age=int))
        with self.assertRaises(TypeError):
            NamedTuple('Name', [('x', int)], y=str)

    def test_namedtuple_special_keyword_names(self):
        NT = NamedTuple("NT", cls=type, self=object, typename=str, fields=list)
        self.assertEqual(NT.__name__, 'NT')
        self.assertEqual(NT._fields, ('cls', 'self', 'typename', 'fields'))
        a = NT(cls=str, self=42, typename='foo', fields=[('bar', tuple)])
        self.assertEqual(a.cls, str)
        self.assertEqual(a.self, 42)
        self.assertEqual(a.typename, 'foo')
        self.assertEqual(a.fields, [('bar', tuple)])

    def test_empty_namedtuple(self):
        NT = NamedTuple('NT')

        class CNT(NamedTuple):
            pass  # empty body

        for struct in [NT, CNT]:
            with self.subTest(struct=struct):
                self.assertEqual(struct._fields, ())
                self.assertEqual(struct._field_defaults, {})
                self.assertEqual(struct.__annotations__, {})
                self.assertIsInstance(struct(), struct)

    def test_namedtuple_errors(self):
        with self.assertRaises(TypeError):
            NamedTuple.__new__()
        with self.assertRaises(TypeError):
            NamedTuple()
        with self.assertRaises(TypeError):
            NamedTuple('Emp', [('name', str)], None)
        with self.assertRaises(ValueError):
            NamedTuple('Emp', [('_name', str)])
        with self.assertRaises(TypeError):
            NamedTuple(typename='Emp', name=str, id=int)

    def test_copy_and_pickle(self):
        global Emp  # pickle wants to reference the class by name
        Emp = NamedTuple('Emp', [('name', str), ('cool', int)])
        for cls in Emp, CoolEmployee, self.NestedEmployee:
            with self.subTest(cls=cls):
                jane = cls('jane', 37)
                for proto in range(pickle.HIGHEST_PROTOCOL + 1):
                    z = pickle.dumps(jane, proto)
                    jane2 = pickle.loads(z)
                    self.assertEqual(jane2, jane)
                    self.assertIsInstance(jane2, cls)

                jane2 = copy(jane)
                self.assertEqual(jane2, jane)
                self.assertIsInstance(jane2, cls)

                jane2 = deepcopy(jane)
                self.assertEqual(jane2, jane)
                self.assertIsInstance(jane2, cls)


class TypedDictTests(BaseTestCase):
    def test_basics_functional_syntax(self):
        Emp = TypedDict('Emp', {'name': str, 'id': int})
        self.assertIsSubclass(Emp, dict)
        self.assertIsSubclass(Emp, typing.MutableMapping)
        self.assertNotIsSubclass(Emp, collections.abc.Sequence)
        jim = Emp(name='Jim', id=1)
        self.assertIs(type(jim), dict)
        self.assertEqual(jim['name'], 'Jim')
        self.assertEqual(jim['id'], 1)
        self.assertEqual(Emp.__name__, 'Emp')
        self.assertEqual(Emp.__module__, __name__)
        self.assertEqual(Emp.__bases__, (dict,))
        self.assertEqual(Emp.__annotations__, {'name': str, 'id': int})
        self.assertEqual(Emp.__total__, True)

    def test_basics_keywords_syntax(self):
        with self.assertWarns(DeprecationWarning):
            Emp = TypedDict('Emp', name=str, id=int)
        self.assertIsSubclass(Emp, dict)
        self.assertIsSubclass(Emp, typing.MutableMapping)
        self.assertNotIsSubclass(Emp, collections.abc.Sequence)
        jim = Emp(name='Jim', id=1)
        self.assertIs(type(jim), dict)
        self.assertEqual(jim['name'], 'Jim')
        self.assertEqual(jim['id'], 1)
        self.assertEqual(Emp.__name__, 'Emp')
        self.assertEqual(Emp.__module__, __name__)
        self.assertEqual(Emp.__bases__, (dict,))
        self.assertEqual(Emp.__annotations__, {'name': str, 'id': int})
        self.assertEqual(Emp.__total__, True)

    def test_typeddict_special_keyword_names(self):
        with self.assertWarns(DeprecationWarning):
            TD = TypedDict("TD", cls=type, self=object, typename=str, _typename=int, fields=list, _fields=dict)
        self.assertEqual(TD.__name__, 'TD')
        self.assertEqual(TD.__annotations__, {'cls': type, 'self': object, 'typename': str, '_typename': int, 'fields': list, '_fields': dict})
        a = TD(cls=str, self=42, typename='foo', _typename=53, fields=[('bar', tuple)], _fields={'baz', set})
        self.assertEqual(a['cls'], str)
        self.assertEqual(a['self'], 42)
        self.assertEqual(a['typename'], 'foo')
        self.assertEqual(a['_typename'], 53)
        self.assertEqual(a['fields'], [('bar', tuple)])
        self.assertEqual(a['_fields'], {'baz', set})

    def test_typeddict_create_errors(self):
        with self.assertRaises(TypeError):
            TypedDict.__new__()
        with self.assertRaises(TypeError):
            TypedDict()
        with self.assertRaises(TypeError):
            TypedDict('Emp', [('name', str)], None)
        with self.assertRaises(TypeError):
            TypedDict(_typename='Emp', name=str, id=int)

    def test_typeddict_errors(self):
        Emp = TypedDict('Emp', {'name': str, 'id': int})
        self.assertEqual(TypedDict.__module__, 'typing')
        jim = Emp(name='Jim', id=1)
        with self.assertRaises(TypeError):
            isinstance({}, Emp)
        with self.assertRaises(TypeError):
            isinstance(jim, Emp)
        with self.assertRaises(TypeError):
            issubclass(dict, Emp)
        with self.assertRaises(TypeError):
            TypedDict('Hi', [('x', int)], y=int)

    def test_py36_class_syntax_usage(self):
        self.assertEqual(LabelPoint2D.__name__, 'LabelPoint2D')
        self.assertEqual(LabelPoint2D.__module__, __name__)
        self.assertEqual(LabelPoint2D.__annotations__, {'x': int, 'y': int, 'label': str})
        self.assertEqual(LabelPoint2D.__bases__, (dict,))
        self.assertEqual(LabelPoint2D.__total__, True)
        self.assertNotIsSubclass(LabelPoint2D, typing.Sequence)
        not_origin = Point2D(x=0, y=1)
        self.assertEqual(not_origin['x'], 0)
        self.assertEqual(not_origin['y'], 1)
        other = LabelPoint2D(x=0, y=1, label='hi')
        self.assertEqual(other['label'], 'hi')

    def test_pickle(self):
        global EmpD  # pickle wants to reference the class by name
        EmpD = TypedDict('EmpD', {'name': str, 'id': int})
        jane = EmpD({'name': 'jane', 'id': 37})
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            z = pickle.dumps(jane, proto)
            jane2 = pickle.loads(z)
            self.assertEqual(jane2, jane)
            self.assertEqual(jane2, {'name': 'jane', 'id': 37})
            ZZ = pickle.dumps(EmpD, proto)
            EmpDnew = pickle.loads(ZZ)
            self.assertEqual(EmpDnew({'name': 'jane', 'id': 37}), jane)

    def test_pickle_generic(self):
        point = Point2DGeneric(a=5.0, b=3.0)
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            z = pickle.dumps(point, proto)
            point2 = pickle.loads(z)
            self.assertEqual(point2, point)
            self.assertEqual(point2, {'a': 5.0, 'b': 3.0})
            ZZ = pickle.dumps(Point2DGeneric, proto)
            Point2DGenericNew = pickle.loads(ZZ)
            self.assertEqual(Point2DGenericNew({'a': 5.0, 'b': 3.0}), point)

    def test_optional(self):
        EmpD = TypedDict('EmpD', {'name': str, 'id': int})

        self.assertEqual(typing.Optional[EmpD], typing.Union[None, EmpD])
        self.assertNotEqual(typing.List[EmpD], typing.Tuple[EmpD])

    def test_total(self):
        D = TypedDict('D', {'x': int}, total=False)
        self.assertEqual(D(), {})
        self.assertEqual(D(x=1), {'x': 1})
        self.assertEqual(D.__total__, False)
        self.assertEqual(D.__required_keys__, frozenset())
        self.assertEqual(D.__optional_keys__, {'x'})

        self.assertEqual(Options(), {})
        self.assertEqual(Options(log_level=2), {'log_level': 2})
        self.assertEqual(Options.__total__, False)
        self.assertEqual(Options.__required_keys__, frozenset())
        self.assertEqual(Options.__optional_keys__, {'log_level', 'log_path'})

    def test_optional_keys(self):
        class Point2Dor3D(Point2D, total=False):
            z: int

        assert Point2Dor3D.__required_keys__ == frozenset(['x', 'y'])
        assert Point2Dor3D.__optional_keys__ == frozenset(['z'])

    def test_keys_inheritance(self):
        class BaseAnimal(TypedDict):
            name: str

        class Animal(BaseAnimal, total=False):
            voice: str
            tail: bool

        class Cat(Animal):
            fur_color: str

        assert BaseAnimal.__required_keys__ == frozenset(['name'])
        assert BaseAnimal.__optional_keys__ == frozenset([])
        assert BaseAnimal.__annotations__ == {'name': str}

        assert Animal.__required_keys__ == frozenset(['name'])
        assert Animal.__optional_keys__ == frozenset(['tail', 'voice'])
        assert Animal.__annotations__ == {
            'name': str,
            'tail': bool,
            'voice': str,
        }

        assert Cat.__required_keys__ == frozenset(['name', 'fur_color'])
        assert Cat.__optional_keys__ == frozenset(['tail', 'voice'])
        assert Cat.__annotations__ == {
            'fur_color': str,
            'name': str,
            'tail': bool,
            'voice': str,
        }

    def test_required_notrequired_keys(self):
        self.assertEqual(NontotalMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(NontotalMovie.__optional_keys__,
                         frozenset({"year"}))

        self.assertEqual(TotalMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(TotalMovie.__optional_keys__,
                         frozenset({"year"}))

        self.assertEqual(_typed_dict_helper.VeryAnnotated.__required_keys__,
                         frozenset())
        self.assertEqual(_typed_dict_helper.VeryAnnotated.__optional_keys__,
                         frozenset({"a"}))

        self.assertEqual(AnnotatedMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(AnnotatedMovie.__optional_keys__,
                         frozenset({"year"}))

        self.assertEqual(WeirdlyQuotedMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(WeirdlyQuotedMovie.__optional_keys__,
                         frozenset({"year"}))

        self.assertEqual(ChildTotalMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(ChildTotalMovie.__optional_keys__,
                         frozenset({"year"}))

        self.assertEqual(ChildDeeplyAnnotatedMovie.__required_keys__,
                         frozenset({"title"}))
        self.assertEqual(ChildDeeplyAnnotatedMovie.__optional_keys__,
                         frozenset({"year"}))

    def test_multiple_inheritance(self):
        class One(TypedDict):
            one: int
        class Two(TypedDict):
            two: str
        class Untotal(TypedDict, total=False):
            untotal: str
        Inline = TypedDict('Inline', {'inline': bool})
        class Regular:
            pass

        class Child(One, Two):
            child: bool
        self.assertEqual(
            Child.__required_keys__,
            frozenset(['one', 'two', 'child']),
        )
        self.assertEqual(
            Child.__optional_keys__,
            frozenset([]),
        )
        self.assertEqual(
            Child.__annotations__,
            {'one': int, 'two': str, 'child': bool},
        )

        class ChildWithOptional(One, Untotal):
            child: bool
        self.assertEqual(
            ChildWithOptional.__required_keys__,
            frozenset(['one', 'child']),
        )
        self.assertEqual(
            ChildWithOptional.__optional_keys__,
            frozenset(['untotal']),
        )
        self.assertEqual(
            ChildWithOptional.__annotations__,
            {'one': int, 'untotal': str, 'child': bool},
        )

        class ChildWithTotalFalse(One, Untotal, total=False):
            child: bool
        self.assertEqual(
            ChildWithTotalFalse.__required_keys__,
            frozenset(['one']),
        )
        self.assertEqual(
            ChildWithTotalFalse.__optional_keys__,
            frozenset(['untotal', 'child']),
        )
        self.assertEqual(
            ChildWithTotalFalse.__annotations__,
            {'one': int, 'untotal': str, 'child': bool},
        )

        class ChildWithInlineAndOptional(Untotal, Inline):
            child: bool
        self.assertEqual(
            ChildWithInlineAndOptional.__required_keys__,
            frozenset(['inline', 'child']),
        )
        self.assertEqual(
            ChildWithInlineAndOptional.__optional_keys__,
            frozenset(['untotal']),
        )
        self.assertEqual(
            ChildWithInlineAndOptional.__annotations__,
            {'inline': bool, 'untotal': str, 'child': bool},
        )

        wrong_bases = [
            (One, Regular),
            (Regular, One),
            (One, Two, Regular),
            (Inline, Regular),
            (Untotal, Regular),
        ]
        for bases in wrong_bases:
            with self.subTest(bases=bases):
                with self.assertRaisesRegex(
                    TypeError,
                    'cannot inherit from both a TypedDict type and a non-TypedDict',
                ):
                    class Wrong(*bases):
                        pass

    def test_is_typeddict(self):
        assert is_typeddict(Point2D) is True
        assert is_typeddict(Union[str, int]) is False
        # classes, not instances
        assert is_typeddict(Point2D()) is False

    def test_get_type_hints(self):
        self.assertEqual(
            get_type_hints(Bar),
            {'a': typing.Optional[int], 'b': int}
        )

    def test_get_type_hints_generic(self):
        self.assertEqual(
            get_type_hints(BarGeneric),
            {'a': typing.Optional[T], 'b': int}
        )

        class FooBarGeneric(BarGeneric[int]):
            c: str

        self.assertEqual(
            get_type_hints(FooBarGeneric),
            {'a': typing.Optional[T], 'b': int, 'c': str}
        )

    def test_generic_inheritance(self):
        class A(TypedDict, Generic[T]):
            a: T

        self.assertEqual(A.__bases__, (Generic, dict))
        self.assertEqual(A.__orig_bases__, (TypedDict, Generic[T]))
        self.assertEqual(A.__mro__, (A, Generic, dict, object))
        self.assertEqual(A.__parameters__, (T,))
        self.assertEqual(A[str].__parameters__, ())
        self.assertEqual(A[str].__args__, (str,))

        class A2(Generic[T], TypedDict):
            a: T

        self.assertEqual(A2.__bases__, (Generic, dict))
        self.assertEqual(A2.__orig_bases__, (Generic[T], TypedDict))
        self.assertEqual(A2.__mro__, (A2, Generic, dict, object))
        self.assertEqual(A2.__parameters__, (T,))
        self.assertEqual(A2[str].__parameters__, ())
        self.assertEqual(A2[str].__args__, (str,))

        class B(A[KT], total=False):
            b: KT

        self.assertEqual(B.__bases__, (Generic, dict))
        self.assertEqual(B.__orig_bases__, (A[KT],))
        self.assertEqual(B.__mro__, (B, Generic, dict, object))
        self.assertEqual(B.__parameters__, (KT,))
        self.assertEqual(B.__total__, False)
        self.assertEqual(B.__optional_keys__, frozenset(['b']))
        self.assertEqual(B.__required_keys__, frozenset(['a']))

        self.assertEqual(B[str].__parameters__, ())
        self.assertEqual(B[str].__args__, (str,))
        self.assertEqual(B[str].__origin__, B)

        class C(B[int]):
            c: int

        self.assertEqual(C.__bases__, (Generic, dict))
        self.assertEqual(C.__orig_bases__, (B[int],))
        self.assertEqual(C.__mro__, (C, Generic, dict, object))
        self.assertEqual(C.__parameters__, ())
        self.assertEqual(C.__total__, True)
        self.assertEqual(C.__optional_keys__, frozenset(['b']))
        self.assertEqual(C.__required_keys__, frozenset(['a', 'c']))
        assert C.__annotations__ == {
            'a': T,
            'b': KT,
            'c': int,
        }
        with self.assertRaises(TypeError):
            C[str]


        class Point3D(Point2DGeneric[T], Generic[T, KT]):
            c: KT

        self.assertEqual(Point3D.__bases__, (Generic, dict))
        self.assertEqual(Point3D.__orig_bases__, (Point2DGeneric[T], Generic[T, KT]))
        self.assertEqual(Point3D.__mro__, (Point3D, Generic, dict, object))
        self.assertEqual(Point3D.__parameters__, (T, KT))
        self.assertEqual(Point3D.__total__, True)
        self.assertEqual(Point3D.__optional_keys__, frozenset())
        self.assertEqual(Point3D.__required_keys__, frozenset(['a', 'b', 'c']))
        assert Point3D.__annotations__ == {
            'a': T,
            'b': T,
            'c': KT,
        }
        self.assertEqual(Point3D[int, str].__origin__, Point3D)

        with self.assertRaises(TypeError):
            Point3D[int]

        with self.assertRaises(TypeError):
            class Point3D(Point2DGeneric[T], Generic[KT]):
                c: KT

    def test_implicit_any_inheritance(self):
        class A(TypedDict, Generic[T]):
            a: T

        class B(A[KT], total=False):
            b: KT

        class WithImplicitAny(B):
            c: int

        self.assertEqual(WithImplicitAny.__bases__, (Generic, dict,))
        self.assertEqual(WithImplicitAny.__mro__, (WithImplicitAny, Generic, dict, object))
        # Consistent with GenericTests.test_implicit_any
        self.assertEqual(WithImplicitAny.__parameters__, ())
        self.assertEqual(WithImplicitAny.__total__, True)
        self.assertEqual(WithImplicitAny.__optional_keys__, frozenset(['b']))
        self.assertEqual(WithImplicitAny.__required_keys__, frozenset(['a', 'c']))
        assert WithImplicitAny.__annotations__ == {
            'a': T,
            'b': KT,
            'c': int,
        }
        with self.assertRaises(TypeError):
            WithImplicitAny[str]

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_non_generic_subscript(self):
        # For backward compatibility, subscription works
        # on arbitrary TypedDict types.
        class TD(TypedDict):
            a: T
        A = TD[int]
        self.assertEqual(A.__origin__, TD)
        self.assertEqual(A.__parameters__, ())
        self.assertEqual(A.__args__, (int,))
        a = A(a = 1)
        self.assertIs(type(a), dict)
        self.assertEqual(a, {'a': 1})


class RequiredTests(BaseTestCase):

    def test_basics(self):
        with self.assertRaises(TypeError):
            Required[NotRequired]
        with self.assertRaises(TypeError):
            Required[int, str]
        with self.assertRaises(TypeError):
            Required[int][str]

    def test_repr(self):
        self.assertEqual(repr(Required), 'typing.Required')
        cv = Required[int]
        self.assertEqual(repr(cv), 'typing.Required[int]')
        cv = Required[Employee]
        self.assertEqual(repr(cv), f'typing.Required[{__name__}.Employee]')

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(Required)):
                pass
        with self.assertRaises(TypeError):
            class C(type(Required[int])):
                pass
        with self.assertRaises(TypeError):
            class C(Required):
                pass
        with self.assertRaises(TypeError):
            class C(Required[int]):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            Required()
        with self.assertRaises(TypeError):
            type(Required)()
        with self.assertRaises(TypeError):
            type(Required[Optional[int]])()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, Required[int])
        with self.assertRaises(TypeError):
            issubclass(int, Required)


class NotRequiredTests(BaseTestCase):

    def test_basics(self):
        with self.assertRaises(TypeError):
            NotRequired[Required]
        with self.assertRaises(TypeError):
            NotRequired[int, str]
        with self.assertRaises(TypeError):
            NotRequired[int][str]

    def test_repr(self):
        self.assertEqual(repr(NotRequired), 'typing.NotRequired')
        cv = NotRequired[int]
        self.assertEqual(repr(cv), 'typing.NotRequired[int]')
        cv = NotRequired[Employee]
        self.assertEqual(repr(cv), f'typing.NotRequired[{__name__}.Employee]')

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(NotRequired)):
                pass
        with self.assertRaises(TypeError):
            class C(type(NotRequired[int])):
                pass
        with self.assertRaises(TypeError):
            class C(NotRequired):
                pass
        with self.assertRaises(TypeError):
            class C(NotRequired[int]):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            NotRequired()
        with self.assertRaises(TypeError):
            type(NotRequired)()
        with self.assertRaises(TypeError):
            type(NotRequired[Optional[int]])()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, NotRequired[int])
        with self.assertRaises(TypeError):
            issubclass(int, NotRequired)


class IOTests(BaseTestCase):

    def test_io(self):

        def stuff(a: IO) -> AnyStr:
            return a.readline()

        a = stuff.__annotations__['a']
        self.assertEqual(a.__parameters__, (AnyStr,))

    def test_textio(self):

        def stuff(a: TextIO) -> str:
            return a.readline()

        a = stuff.__annotations__['a']
        self.assertEqual(a.__parameters__, ())

    def test_binaryio(self):

        def stuff(a: BinaryIO) -> bytes:
            return a.readline()

        a = stuff.__annotations__['a']
        self.assertEqual(a.__parameters__, ())

    def test_io_submodule(self):
        with warnings.catch_warnings(record=True) as w:
            warnings.filterwarnings("default", category=DeprecationWarning)
            from typing.io import IO, TextIO, BinaryIO, __all__, __name__
            self.assertIs(IO, typing.IO)
            self.assertIs(TextIO, typing.TextIO)
            self.assertIs(BinaryIO, typing.BinaryIO)
            self.assertEqual(set(__all__), set(['IO', 'TextIO', 'BinaryIO']))
            self.assertEqual(__name__, 'typing.io')
            self.assertEqual(len(w), 1)


class RETests(BaseTestCase):
    # Much of this is really testing _TypeAlias.

    def test_basics(self):
        pat = re.compile('[a-z]+', re.I)
        self.assertIsSubclass(pat.__class__, Pattern)
        self.assertIsSubclass(type(pat), Pattern)
        self.assertIsInstance(pat, Pattern)

        mat = pat.search('12345abcde.....')
        self.assertIsSubclass(mat.__class__, Match)
        self.assertIsSubclass(type(mat), Match)
        self.assertIsInstance(mat, Match)

        # these should just work
        Pattern[Union[str, bytes]]
        Match[Union[bytes, str]]

    def test_alias_equality(self):
        self.assertEqual(Pattern[str], Pattern[str])
        self.assertNotEqual(Pattern[str], Pattern[bytes])
        self.assertNotEqual(Pattern[str], Match[str])
        self.assertNotEqual(Pattern[str], str)

    def test_errors(self):
        m = Match[Union[str, bytes]]
        with self.assertRaises(TypeError):
            m[str]
        with self.assertRaises(TypeError):
            # We don't support isinstance().
            isinstance(42, Pattern[str])
        with self.assertRaises(TypeError):
            # We don't support issubclass().
            issubclass(Pattern[bytes], Pattern[str])

    def test_repr(self):
        self.assertEqual(repr(Pattern), 'typing.Pattern')
        self.assertEqual(repr(Pattern[str]), 'typing.Pattern[str]')
        self.assertEqual(repr(Pattern[bytes]), 'typing.Pattern[bytes]')
        self.assertEqual(repr(Match), 'typing.Match')
        self.assertEqual(repr(Match[str]), 'typing.Match[str]')
        self.assertEqual(repr(Match[bytes]), 'typing.Match[bytes]')

    def test_re_submodule(self):
        with warnings.catch_warnings(record=True) as w:
            warnings.filterwarnings("default", category=DeprecationWarning)
            from typing.re import Match, Pattern, __all__, __name__
            self.assertIs(Match, typing.Match)
            self.assertIs(Pattern, typing.Pattern)
            self.assertEqual(set(__all__), set(['Match', 'Pattern']))
            self.assertEqual(__name__, 'typing.re')
            self.assertEqual(len(w), 1)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_cannot_subclass(self):
        with self.assertRaises(TypeError) as ex:

            class A(typing.Match):
                pass

        self.assertEqual(str(ex.exception),
                         "type 're.Match' is not an acceptable base type")


class AnnotatedTests(BaseTestCase):

    def test_new(self):
        with self.assertRaisesRegex(
            TypeError,
            'Type Annotated cannot be instantiated',
        ):
            Annotated()

    def test_repr(self):
        self.assertEqual(
            repr(Annotated[int, 4, 5]),
            "typing.Annotated[int, 4, 5]"
        )
        self.assertEqual(
            repr(Annotated[List[int], 4, 5]),
            "typing.Annotated[typing.List[int], 4, 5]"
        )

    def test_flatten(self):
        A = Annotated[Annotated[int, 4], 5]
        self.assertEqual(A, Annotated[int, 4, 5])
        self.assertEqual(A.__metadata__, (4, 5))
        self.assertEqual(A.__origin__, int)

    def test_specialize(self):
        L = Annotated[List[T], "my decoration"]
        LI = Annotated[List[int], "my decoration"]
        self.assertEqual(L[int], Annotated[List[int], "my decoration"])
        self.assertEqual(L[int].__metadata__, ("my decoration",))
        self.assertEqual(L[int].__origin__, List[int])
        with self.assertRaises(TypeError):
            LI[int]
        with self.assertRaises(TypeError):
            L[int, float]

    def test_hash_eq(self):
        self.assertEqual(len({Annotated[int, 4, 5], Annotated[int, 4, 5]}), 1)
        self.assertNotEqual(Annotated[int, 4, 5], Annotated[int, 5, 4])
        self.assertNotEqual(Annotated[int, 4, 5], Annotated[str, 4, 5])
        self.assertNotEqual(Annotated[int, 4], Annotated[int, 4, 4])
        self.assertEqual(
            {Annotated[int, 4, 5], Annotated[int, 4, 5], Annotated[T, 4, 5]},
            {Annotated[int, 4, 5], Annotated[T, 4, 5]}
        )

    def test_instantiate(self):
        class C:
            classvar = 4

            def __init__(self, x):
                self.x = x

            def __eq__(self, other):
                if not isinstance(other, C):
                    return NotImplemented
                return other.x == self.x

        A = Annotated[C, "a decoration"]
        a = A(5)
        c = C(5)
        self.assertEqual(a, c)
        self.assertEqual(a.x, c.x)
        self.assertEqual(a.classvar, c.classvar)

    def test_instantiate_generic(self):
        MyCount = Annotated[typing.Counter[T], "my decoration"]
        self.assertEqual(MyCount([4, 4, 5]), {4: 2, 5: 1})
        self.assertEqual(MyCount[int]([4, 4, 5]), {4: 2, 5: 1})

    def test_cannot_instantiate_forward(self):
        A = Annotated["int", (5, 6)]
        with self.assertRaises(TypeError):
            A(5)

    def test_cannot_instantiate_type_var(self):
        A = Annotated[T, (5, 6)]
        with self.assertRaises(TypeError):
            A(5)

    def test_cannot_getattr_typevar(self):
        with self.assertRaises(AttributeError):
            Annotated[T, (5, 7)].x

    def test_attr_passthrough(self):
        class C:
            classvar = 4

        A = Annotated[C, "a decoration"]
        self.assertEqual(A.classvar, 4)
        A.x = 5
        self.assertEqual(C.x, 5)

    def test_special_form_containment(self):
        class C:
            classvar: Annotated[ClassVar[int], "a decoration"] = 4
            const: Annotated[Final[int], "Const"] = 4

        self.assertEqual(get_type_hints(C, globals())['classvar'], ClassVar[int])
        self.assertEqual(get_type_hints(C, globals())['const'], Final[int])

    def test_hash_eq(self):
        self.assertEqual(len({Annotated[int, 4, 5], Annotated[int, 4, 5]}), 1)
        self.assertNotEqual(Annotated[int, 4, 5], Annotated[int, 5, 4])
        self.assertNotEqual(Annotated[int, 4, 5], Annotated[str, 4, 5])
        self.assertNotEqual(Annotated[int, 4], Annotated[int, 4, 4])
        self.assertEqual(
            {Annotated[int, 4, 5], Annotated[int, 4, 5], Annotated[T, 4, 5]},
            {Annotated[int, 4, 5], Annotated[T, 4, 5]}
        )

    def test_cannot_subclass(self):
        with self.assertRaisesRegex(TypeError, "Cannot subclass .*Annotated"):
            class C(Annotated):
                pass

    def test_cannot_check_instance(self):
        with self.assertRaises(TypeError):
            isinstance(5, Annotated[int, "positive"])

    def test_cannot_check_subclass(self):
        with self.assertRaises(TypeError):
            issubclass(int, Annotated[int, "positive"])

    def test_too_few_type_args(self):
        with self.assertRaisesRegex(TypeError, 'at least two arguments'):
            Annotated[int]

    def test_pickle(self):
        samples = [typing.Any, typing.Union[int, str],
                   typing.Optional[str], Tuple[int, ...],
                   typing.Callable[[str], bytes]]

        for t in samples:
            x = Annotated[t, "a"]

            for prot in range(pickle.HIGHEST_PROTOCOL + 1):
                with self.subTest(protocol=prot, type=t):
                    pickled = pickle.dumps(x, prot)
                    restored = pickle.loads(pickled)
                    self.assertEqual(x, restored)

        global _Annotated_test_G

        class _Annotated_test_G(Generic[T]):
            x = 1

        G = Annotated[_Annotated_test_G[int], "A decoration"]
        G.foo = 42
        G.bar = 'abc'

        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            z = pickle.dumps(G, proto)
            x = pickle.loads(z)
            self.assertEqual(x.foo, 42)
            self.assertEqual(x.bar, 'abc')
            self.assertEqual(x.x, 1)

    def test_subst(self):
        dec = "a decoration"
        dec2 = "another decoration"

        S = Annotated[T, dec2]
        self.assertEqual(S[int], Annotated[int, dec2])

        self.assertEqual(S[Annotated[int, dec]], Annotated[int, dec, dec2])
        L = Annotated[List[T], dec]

        self.assertEqual(L[int], Annotated[List[int], dec])
        with self.assertRaises(TypeError):
            L[int, int]

        self.assertEqual(S[L[int]], Annotated[List[int], dec, dec2])

        D = Annotated[typing.Dict[KT, VT], dec]
        self.assertEqual(D[str, int], Annotated[typing.Dict[str, int], dec])
        with self.assertRaises(TypeError):
            D[int]

        It = Annotated[int, dec]
        with self.assertRaises(TypeError):
            It[None]

        LI = L[int]
        with self.assertRaises(TypeError):
            LI[None]

    def test_typevar_subst(self):
        dec = "a decoration"
        Ts = TypeVarTuple('Ts')
        T = TypeVar('T')
        T1 = TypeVar('T1')
        T2 = TypeVar('T2')

        A = Annotated[tuple[*Ts], dec]
        self.assertEqual(A[int], Annotated[tuple[int], dec])
        self.assertEqual(A[str, int], Annotated[tuple[str, int], dec])
        with self.assertRaises(TypeError):
            Annotated[*Ts, dec]

        B = Annotated[Tuple[Unpack[Ts]], dec]
        self.assertEqual(B[int], Annotated[Tuple[int], dec])
        self.assertEqual(B[str, int], Annotated[Tuple[str, int], dec])
        with self.assertRaises(TypeError):
            Annotated[Unpack[Ts], dec]

        C = Annotated[tuple[T, *Ts], dec]
        self.assertEqual(C[int], Annotated[tuple[int], dec])
        self.assertEqual(C[int, str], Annotated[tuple[int, str], dec])
        self.assertEqual(
            C[int, str, float],
            Annotated[tuple[int, str, float], dec]
        )
        with self.assertRaises(TypeError):
            C[()]

        D = Annotated[Tuple[T, Unpack[Ts]], dec]
        self.assertEqual(D[int], Annotated[Tuple[int], dec])
        self.assertEqual(D[int, str], Annotated[Tuple[int, str], dec])
        self.assertEqual(
            D[int, str, float],
            Annotated[Tuple[int, str, float], dec]
        )
        with self.assertRaises(TypeError):
            D[()]

        E = Annotated[tuple[*Ts, T], dec]
        self.assertEqual(E[int], Annotated[tuple[int], dec])
        self.assertEqual(E[int, str], Annotated[tuple[int, str], dec])
        self.assertEqual(
            E[int, str, float],
            Annotated[tuple[int, str, float], dec]
        )
        with self.assertRaises(TypeError):
            E[()]

        F = Annotated[Tuple[Unpack[Ts], T], dec]
        self.assertEqual(F[int], Annotated[Tuple[int], dec])
        self.assertEqual(F[int, str], Annotated[Tuple[int, str], dec])
        self.assertEqual(
            F[int, str, float],
            Annotated[Tuple[int, str, float], dec]
        )
        with self.assertRaises(TypeError):
            F[()]

        G = Annotated[tuple[T1, *Ts, T2], dec]
        self.assertEqual(G[int, str], Annotated[tuple[int, str], dec])
        self.assertEqual(
            G[int, str, float],
            Annotated[tuple[int, str, float], dec]
        )
        self.assertEqual(
            G[int, str, bool, float],
            Annotated[tuple[int, str, bool, float], dec]
        )
        with self.assertRaises(TypeError):
            G[int]

        H = Annotated[Tuple[T1, Unpack[Ts], T2], dec]
        self.assertEqual(H[int, str], Annotated[Tuple[int, str], dec])
        self.assertEqual(
            H[int, str, float],
            Annotated[Tuple[int, str, float], dec]
        )
        self.assertEqual(
            H[int, str, bool, float],
            Annotated[Tuple[int, str, bool, float], dec]
        )
        with self.assertRaises(TypeError):
            H[int]

        # Now let's try creating an alias from an alias.

        Ts2 = TypeVarTuple('Ts2')
        T3 = TypeVar('T3')
        T4 = TypeVar('T4')

        # G is Annotated[tuple[T1, *Ts, T2], dec].
        I = G[T3, *Ts2, T4]
        J = G[T3, Unpack[Ts2], T4]

        for x, y in [
            (I,                  Annotated[tuple[T3, *Ts2, T4], dec]),
            (J,                  Annotated[tuple[T3, Unpack[Ts2], T4], dec]),
            (I[int, str],        Annotated[tuple[int, str], dec]),
            (J[int, str],        Annotated[tuple[int, str], dec]),
            (I[int, str, float], Annotated[tuple[int, str, float], dec]),
            (J[int, str, float], Annotated[tuple[int, str, float], dec]),
            (I[int, str, bool, float],
                                 Annotated[tuple[int, str, bool, float], dec]),
            (J[int, str, bool, float],
                                 Annotated[tuple[int, str, bool, float], dec]),
        ]:
            self.assertEqual(x, y)

        with self.assertRaises(TypeError):
            I[int]
        with self.assertRaises(TypeError):
            J[int]

    def test_annotated_in_other_types(self):
        X = List[Annotated[T, 5]]
        self.assertEqual(X[int], List[Annotated[int, 5]])

    def test_annotated_mro(self):
        class X(Annotated[int, (1, 10)]): ...
        self.assertEqual(X.__mro__, (X, int, object),
                         "Annotated should be transparent.")


class TypeAliasTests(BaseTestCase):
    def test_canonical_usage_with_variable_annotation(self):
        Alias: TypeAlias = Employee

    def test_canonical_usage_with_type_comment(self):
        Alias = Employee  # type: TypeAlias

    def test_cannot_instantiate(self):
        with self.assertRaises(TypeError):
            TypeAlias()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(42, TypeAlias)

    def test_stringized_usage(self):
        class A:
            a: "TypeAlias"
        self.assertEqual(get_type_hints(A), {'a': TypeAlias})

    def test_no_issubclass(self):
        with self.assertRaises(TypeError):
            issubclass(Employee, TypeAlias)

        with self.assertRaises(TypeError):
            issubclass(TypeAlias, Employee)

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(TypeAlias):
                pass

        with self.assertRaises(TypeError):
            class C(type(TypeAlias)):
                pass

    def test_repr(self):
        self.assertEqual(repr(TypeAlias), 'typing.TypeAlias')

    def test_cannot_subscript(self):
        with self.assertRaises(TypeError):
            TypeAlias[int]


class ParamSpecTests(BaseTestCase):

    def test_basic_plain(self):
        P = ParamSpec('P')
        self.assertEqual(P, P)
        self.assertIsInstance(P, ParamSpec)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_valid_uses(self):
        P = ParamSpec('P')
        T = TypeVar('T')
        C1 = Callable[P, int]
        self.assertEqual(C1.__args__, (P, int))
        self.assertEqual(C1.__parameters__, (P,))
        C2 = Callable[P, T]
        self.assertEqual(C2.__args__, (P, T))
        self.assertEqual(C2.__parameters__, (P, T))
        # Test collections.abc.Callable too.
        C3 = collections.abc.Callable[P, int]
        self.assertEqual(C3.__args__, (P, int))
        self.assertEqual(C3.__parameters__, (P,))
        C4 = collections.abc.Callable[P, T]
        self.assertEqual(C4.__args__, (P, T))
        self.assertEqual(C4.__parameters__, (P, T))

    def test_args_kwargs(self):
        P = ParamSpec('P')
        P_2 = ParamSpec('P_2')
        self.assertIn('args', dir(P))
        self.assertIn('kwargs', dir(P))
        self.assertIsInstance(P.args, ParamSpecArgs)
        self.assertIsInstance(P.kwargs, ParamSpecKwargs)
        self.assertIs(P.args.__origin__, P)
        self.assertIs(P.kwargs.__origin__, P)
        self.assertEqual(P.args, P.args)
        self.assertEqual(P.kwargs, P.kwargs)
        self.assertNotEqual(P.args, P_2.args)
        self.assertNotEqual(P.kwargs, P_2.kwargs)
        self.assertNotEqual(P.args, P.kwargs)
        self.assertNotEqual(P.kwargs, P.args)
        self.assertNotEqual(P.args, P_2.kwargs)
        self.assertEqual(repr(P.args), "P.args")
        self.assertEqual(repr(P.kwargs), "P.kwargs")

    def test_stringized(self):
        P = ParamSpec('P')
        class C(Generic[P]):
            func: Callable["P", int]
            def foo(self, *args: "P.args", **kwargs: "P.kwargs"):
                pass

        self.assertEqual(gth(C, globals(), locals()), {"func": Callable[P, int]})
        self.assertEqual(
            gth(C.foo, globals(), locals()), {"args": P.args, "kwargs": P.kwargs}
        )

    def test_user_generics(self):
        T = TypeVar("T")
        P = ParamSpec("P")
        P_2 = ParamSpec("P_2")

        class X(Generic[T, P]):
            f: Callable[P, int]
            x: T
        G1 = X[int, P_2]
        self.assertEqual(G1.__args__, (int, P_2))
        self.assertEqual(G1.__parameters__, (P_2,))
        with self.assertRaisesRegex(TypeError, "few arguments for"):
            X[int]
        with self.assertRaisesRegex(TypeError, "many arguments for"):
            X[int, P_2, str]

        G2 = X[int, Concatenate[int, P_2]]
        self.assertEqual(G2.__args__, (int, Concatenate[int, P_2]))
        self.assertEqual(G2.__parameters__, (P_2,))

        G3 = X[int, [int, bool]]
        self.assertEqual(G3.__args__, (int, (int, bool)))
        self.assertEqual(G3.__parameters__, ())

        G4 = X[int, ...]
        self.assertEqual(G4.__args__, (int, Ellipsis))
        self.assertEqual(G4.__parameters__, ())

        class Z(Generic[P]):
            f: Callable[P, int]

        G5 = Z[[int, str, bool]]
        self.assertEqual(G5.__args__, ((int, str, bool),))
        self.assertEqual(G5.__parameters__, ())

        G6 = Z[int, str, bool]
        self.assertEqual(G6.__args__, ((int, str, bool),))
        self.assertEqual(G6.__parameters__, ())

        # G5 and G6 should be equivalent according to the PEP
        self.assertEqual(G5.__args__, G6.__args__)
        self.assertEqual(G5.__origin__, G6.__origin__)
        self.assertEqual(G5.__parameters__, G6.__parameters__)
        self.assertEqual(G5, G6)

        G7 = Z[int]
        self.assertEqual(G7.__args__, ((int,),))
        self.assertEqual(G7.__parameters__, ())

        with self.assertRaisesRegex(TypeError, "many arguments for"):
            Z[[int, str], bool]
        with self.assertRaisesRegex(TypeError, "many arguments for"):
            Z[P_2, bool]

    def test_multiple_paramspecs_in_user_generics(self):
        P = ParamSpec("P")
        P2 = ParamSpec("P2")

        class X(Generic[P, P2]):
            f: Callable[P, int]
            g: Callable[P2, str]

        G1 = X[[int, str], [bytes]]
        G2 = X[[int], [str, bytes]]
        self.assertNotEqual(G1, G2)
        self.assertEqual(G1.__args__, ((int, str), (bytes,)))
        self.assertEqual(G2.__args__, ((int,), (str, bytes)))

    def test_typevartuple_and_paramspecs_in_user_generics(self):
        Ts = TypeVarTuple("Ts")
        P = ParamSpec("P")

        class X(Generic[*Ts, P]):
            f: Callable[P, int]
            g: Tuple[*Ts]

        G1 = X[int, [bytes]]
        self.assertEqual(G1.__args__, (int, (bytes,)))
        G2 = X[int, str, [bytes]]
        self.assertEqual(G2.__args__, (int, str, (bytes,)))
        G3 = X[[bytes]]
        self.assertEqual(G3.__args__, ((bytes,),))
        G4 = X[[]]
        self.assertEqual(G4.__args__, ((),))
        with self.assertRaises(TypeError):
            X[()]

        class Y(Generic[P, *Ts]):
            f: Callable[P, int]
            g: Tuple[*Ts]

        G1 = Y[[bytes], int]
        self.assertEqual(G1.__args__, ((bytes,), int))
        G2 = Y[[bytes], int, str]
        self.assertEqual(G2.__args__, ((bytes,), int, str))
        G3 = Y[[bytes]]
        self.assertEqual(G3.__args__, ((bytes,),))
        G4 = Y[[]]
        self.assertEqual(G4.__args__, ((),))
        with self.assertRaises(TypeError):
            Y[()]

    def test_typevartuple_and_paramspecs_in_generic_aliases(self):
        P = ParamSpec('P')
        T = TypeVar('T')
        Ts = TypeVarTuple('Ts')

        for C in Callable, collections.abc.Callable:
            with self.subTest(generic=C):
                A = C[P, Tuple[*Ts]]
                B = A[[int, str], bytes, float]
                self.assertEqual(B.__args__, (int, str, Tuple[bytes, float]))

        class X(Generic[T, P]):
            pass

        A = X[Tuple[*Ts], P]
        B = A[bytes, float, [int, str]]
        self.assertEqual(B.__args__, (Tuple[bytes, float], (int, str,)))

        class Y(Generic[P, T]):
            pass

        A = Y[P, Tuple[*Ts]]
        B = A[[int, str], bytes, float]
        self.assertEqual(B.__args__, ((int, str,), Tuple[bytes, float]))

    def test_var_substitution(self):
        T = TypeVar("T")
        P = ParamSpec("P")
        subst = P.__typing_subst__
        self.assertEqual(subst((int, str)), (int, str))
        self.assertEqual(subst([int, str]), (int, str))
        self.assertEqual(subst([None]), (type(None),))
        self.assertIs(subst(...), ...)
        self.assertIs(subst(P), P)
        self.assertEqual(subst(Concatenate[int, P]), Concatenate[int, P])

    def test_bad_var_substitution(self):
        T = TypeVar('T')
        P = ParamSpec('P')
        bad_args = (42, int, None, T, int|str, Union[int, str])
        for arg in bad_args:
            with self.subTest(arg=arg):
                with self.assertRaises(TypeError):
                    P.__typing_subst__(arg)
                with self.assertRaises(TypeError):
                    typing.Callable[P, T][arg, str]
                with self.assertRaises(TypeError):
                    collections.abc.Callable[P, T][arg, str]

    def test_paramspec_in_nested_generics(self):
        # Although ParamSpec should not be found in __parameters__ of most
        # generics, they probably should be found when nested in
        # a valid location.
        T = TypeVar("T")
        P = ParamSpec("P")
        C1 = Callable[P, T]
        G1 = List[C1]
        G2 = list[C1]
        G3 = list[C1] | int
        self.assertEqual(G1.__parameters__, (P, T))
        self.assertEqual(G2.__parameters__, (P, T))
        self.assertEqual(G3.__parameters__, (P, T))
        C = Callable[[int, str], float]
        self.assertEqual(G1[[int, str], float], List[C])
        self.assertEqual(G2[[int, str], float], list[C])
        self.assertEqual(G3[[int, str], float], list[C] | int)

    def test_paramspec_gets_copied(self):
        # bpo-46581
        P = ParamSpec('P')
        P2 = ParamSpec('P2')
        C1 = Callable[P, int]
        self.assertEqual(C1.__parameters__, (P,))
        self.assertEqual(C1[P2].__parameters__, (P2,))
        self.assertEqual(C1[str].__parameters__, ())
        self.assertEqual(C1[str, T].__parameters__, (T,))
        self.assertEqual(C1[Concatenate[str, P2]].__parameters__, (P2,))
        self.assertEqual(C1[Concatenate[T, P2]].__parameters__, (T, P2))
        self.assertEqual(C1[...].__parameters__, ())

        C2 = Callable[Concatenate[str, P], int]
        self.assertEqual(C2.__parameters__, (P,))
        self.assertEqual(C2[P2].__parameters__, (P2,))
        self.assertEqual(C2[str].__parameters__, ())
        self.assertEqual(C2[str, T].__parameters__, (T,))
        self.assertEqual(C2[Concatenate[str, P2]].__parameters__, (P2,))
        self.assertEqual(C2[Concatenate[T, P2]].__parameters__, (T, P2))


class ConcatenateTests(BaseTestCase):
    def test_basics(self):
        P = ParamSpec('P')
        class MyClass: ...
        c = Concatenate[MyClass, P]
        self.assertNotEqual(c, Concatenate)

    def test_valid_uses(self):
        P = ParamSpec('P')
        T = TypeVar('T')
        C1 = Callable[Concatenate[int, P], int]
        self.assertEqual(C1.__args__, (Concatenate[int, P], int))
        self.assertEqual(C1.__parameters__, (P,))
        C2 = Callable[Concatenate[int, T, P], T]
        self.assertEqual(C2.__args__, (Concatenate[int, T, P], T))
        self.assertEqual(C2.__parameters__, (T, P))

        # Test collections.abc.Callable too.
        C3 = collections.abc.Callable[Concatenate[int, P], int]
        self.assertEqual(C3.__args__, (Concatenate[int, P], int))
        self.assertEqual(C3.__parameters__, (P,))
        C4 = collections.abc.Callable[Concatenate[int, T, P], T]
        self.assertEqual(C4.__args__, (Concatenate[int, T, P], T))
        self.assertEqual(C4.__parameters__, (T, P))

    def test_var_substitution(self):
        T = TypeVar('T')
        P = ParamSpec('P')
        P2 = ParamSpec('P2')
        C = Concatenate[T, P]
        self.assertEqual(C[int, P2], Concatenate[int, P2])
        self.assertEqual(C[int, [str, float]], (int, str, float))
        self.assertEqual(C[int, []], (int,))
        self.assertEqual(C[int, Concatenate[str, P2]],
                         Concatenate[int, str, P2])
        self.assertEqual(C[int, ...], Concatenate[int, ...])

        C = Concatenate[int, P]
        self.assertEqual(C[P2], Concatenate[int, P2])
        self.assertEqual(C[[str, float]], (int, str, float))
        self.assertEqual(C[str, float], (int, str, float))
        self.assertEqual(C[[]], (int,))
        self.assertEqual(C[Concatenate[str, P2]], Concatenate[int, str, P2])
        self.assertEqual(C[...], Concatenate[int, ...])

class TypeGuardTests(BaseTestCase):
    def test_basics(self):
        TypeGuard[int]  # OK

        def foo(arg) -> TypeGuard[int]: ...
        self.assertEqual(gth(foo), {'return': TypeGuard[int]})

        with self.assertRaises(TypeError):
            TypeGuard[int, str]

    def test_repr(self):
        self.assertEqual(repr(TypeGuard), 'typing.TypeGuard')
        cv = TypeGuard[int]
        self.assertEqual(repr(cv), 'typing.TypeGuard[int]')
        cv = TypeGuard[Employee]
        self.assertEqual(repr(cv), 'typing.TypeGuard[%s.Employee]' % __name__)
        cv = TypeGuard[tuple[int]]
        self.assertEqual(repr(cv), 'typing.TypeGuard[tuple[int]]')

    def test_cannot_subclass(self):
        with self.assertRaises(TypeError):
            class C(type(TypeGuard)):
                pass
        with self.assertRaises(TypeError):
            class C(type(TypeGuard[int])):
                pass

    def test_cannot_init(self):
        with self.assertRaises(TypeError):
            TypeGuard()
        with self.assertRaises(TypeError):
            type(TypeGuard)()
        with self.assertRaises(TypeError):
            type(TypeGuard[Optional[int]])()

    def test_no_isinstance(self):
        with self.assertRaises(TypeError):
            isinstance(1, TypeGuard[int])
        with self.assertRaises(TypeError):
            issubclass(int, TypeGuard)


SpecialAttrsP = typing.ParamSpec('SpecialAttrsP')
SpecialAttrsT = typing.TypeVar('SpecialAttrsT', int, float, complex)


class SpecialAttrsTests(BaseTestCase):

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_special_attrs(self):
        cls_to_check = {
            # ABC classes
            typing.AbstractSet: 'AbstractSet',
            typing.AsyncContextManager: 'AsyncContextManager',
            typing.AsyncGenerator: 'AsyncGenerator',
            typing.AsyncIterable: 'AsyncIterable',
            typing.AsyncIterator: 'AsyncIterator',
            typing.Awaitable: 'Awaitable',
            typing.ByteString: 'ByteString',
            typing.Callable: 'Callable',
            typing.ChainMap: 'ChainMap',
            typing.Collection: 'Collection',
            typing.Container: 'Container',
            typing.ContextManager: 'ContextManager',
            typing.Coroutine: 'Coroutine',
            typing.Counter: 'Counter',
            typing.DefaultDict: 'DefaultDict',
            typing.Deque: 'Deque',
            typing.Dict: 'Dict',
            typing.FrozenSet: 'FrozenSet',
            typing.Generator: 'Generator',
            typing.Hashable: 'Hashable',
            typing.ItemsView: 'ItemsView',
            typing.Iterable: 'Iterable',
            typing.Iterator: 'Iterator',
            typing.KeysView: 'KeysView',
            typing.List: 'List',
            typing.Mapping: 'Mapping',
            typing.MappingView: 'MappingView',
            typing.MutableMapping: 'MutableMapping',
            typing.MutableSequence: 'MutableSequence',
            typing.MutableSet: 'MutableSet',
            typing.OrderedDict: 'OrderedDict',
            typing.Reversible: 'Reversible',
            typing.Sequence: 'Sequence',
            typing.Set: 'Set',
            typing.Sized: 'Sized',
            typing.Tuple: 'Tuple',
            typing.Type: 'Type',
            typing.ValuesView: 'ValuesView',
            # Subscribed ABC classes
            typing.AbstractSet[Any]: 'AbstractSet',
            typing.AsyncContextManager[Any]: 'AsyncContextManager',
            typing.AsyncGenerator[Any, Any]: 'AsyncGenerator',
            typing.AsyncIterable[Any]: 'AsyncIterable',
            typing.AsyncIterator[Any]: 'AsyncIterator',
            typing.Awaitable[Any]: 'Awaitable',
            typing.Callable[[], Any]: 'Callable',
            typing.Callable[..., Any]: 'Callable',
            typing.ChainMap[Any, Any]: 'ChainMap',
            typing.Collection[Any]: 'Collection',
            typing.Container[Any]: 'Container',
            typing.ContextManager[Any]: 'ContextManager',
            typing.Coroutine[Any, Any, Any]: 'Coroutine',
            typing.Counter[Any]: 'Counter',
            typing.DefaultDict[Any, Any]: 'DefaultDict',
            typing.Deque[Any]: 'Deque',
            typing.Dict[Any, Any]: 'Dict',
            typing.FrozenSet[Any]: 'FrozenSet',
            typing.Generator[Any, Any, Any]: 'Generator',
            typing.ItemsView[Any, Any]: 'ItemsView',
            typing.Iterable[Any]: 'Iterable',
            typing.Iterator[Any]: 'Iterator',
            typing.KeysView[Any]: 'KeysView',
            typing.List[Any]: 'List',
            typing.Mapping[Any, Any]: 'Mapping',
            typing.MappingView[Any]: 'MappingView',
            typing.MutableMapping[Any, Any]: 'MutableMapping',
            typing.MutableSequence[Any]: 'MutableSequence',
            typing.MutableSet[Any]: 'MutableSet',
            typing.OrderedDict[Any, Any]: 'OrderedDict',
            typing.Reversible[Any]: 'Reversible',
            typing.Sequence[Any]: 'Sequence',
            typing.Set[Any]: 'Set',
            typing.Tuple[Any]: 'Tuple',
            typing.Tuple[Any, ...]: 'Tuple',
            typing.Type[Any]: 'Type',
            typing.ValuesView[Any]: 'ValuesView',
            # Special Forms
            typing.Annotated: 'Annotated',
            typing.Any: 'Any',
            typing.ClassVar: 'ClassVar',
            typing.Concatenate: 'Concatenate',
            typing.Final: 'Final',
            typing.ForwardRef: 'ForwardRef',
            typing.Literal: 'Literal',
            typing.NewType: 'NewType',
            typing.NoReturn: 'NoReturn',
            typing.Never: 'Never',
            typing.Optional: 'Optional',
            typing.TypeAlias: 'TypeAlias',
            typing.TypeGuard: 'TypeGuard',
            typing.TypeVar: 'TypeVar',
            typing.Union: 'Union',
            typing.Self: 'Self',
            # Subscribed special forms
            typing.Annotated[Any, "Annotation"]: 'Annotated',
            typing.ClassVar[Any]: 'ClassVar',
            typing.Concatenate[Any, SpecialAttrsP]: 'Concatenate',
            typing.Final[Any]: 'Final',
            typing.Literal[Any]: 'Literal',
            typing.Literal[1, 2]: 'Literal',
            typing.Literal[True, 2]: 'Literal',
            typing.Optional[Any]: 'Optional',
            typing.TypeGuard[Any]: 'TypeGuard',
            typing.Union[Any]: 'Any',
            typing.Union[int, float]: 'Union',
            # Incompatible special forms (tested in test_special_attrs2)
            # - typing.ForwardRef('set[Any]')
            # - typing.NewType('TypeName', Any)
            # - typing.ParamSpec('SpecialAttrsP')
            # - typing.TypeVar('T')
        }

        for cls, name in cls_to_check.items():
            with self.subTest(cls=cls):
                self.assertEqual(cls.__name__, name, str(cls))
                self.assertEqual(cls.__qualname__, name, str(cls))
                self.assertEqual(cls.__module__, 'typing', str(cls))
                for proto in range(pickle.HIGHEST_PROTOCOL + 1):
                    s = pickle.dumps(cls, proto)
                    loaded = pickle.loads(s)
                    self.assertIs(cls, loaded)

    TypeName = typing.NewType('SpecialAttrsTests.TypeName', Any)

    # TODO: RUSTPYTHON
    @unittest.expectedFailure
    def test_special_attrs2(self):
        # Forward refs provide a different introspection API. __name__ and
        # __qualname__ make little sense for forward refs as they can store
        # complex typing expressions.
        fr = typing.ForwardRef('set[Any]')
        self.assertFalse(hasattr(fr, '__name__'))
        self.assertFalse(hasattr(fr, '__qualname__'))
        self.assertEqual(fr.__module__, 'typing')
        # Forward refs are currently unpicklable.
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            with self.assertRaises(TypeError) as exc:
                pickle.dumps(fr, proto)

        self.assertEqual(SpecialAttrsTests.TypeName.__name__, 'TypeName')
        self.assertEqual(
            SpecialAttrsTests.TypeName.__qualname__,
            'SpecialAttrsTests.TypeName',
        )
        self.assertEqual(
            SpecialAttrsTests.TypeName.__module__,
            __name__,
        )
        # NewTypes are picklable assuming correct qualname information.
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            s = pickle.dumps(SpecialAttrsTests.TypeName, proto)
            loaded = pickle.loads(s)
            self.assertIs(SpecialAttrsTests.TypeName, loaded)

        # Type variables don't support non-global instantiation per PEP 484
        # restriction that "The argument to TypeVar() must be a string equal
        # to the variable name to which it is assigned".  Thus, providing
        # __qualname__ is unnecessary.
        self.assertEqual(SpecialAttrsT.__name__, 'SpecialAttrsT')
        self.assertFalse(hasattr(SpecialAttrsT, '__qualname__'))
        self.assertEqual(SpecialAttrsT.__module__, __name__)
        # Module-level type variables are picklable.
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            s = pickle.dumps(SpecialAttrsT, proto)
            loaded = pickle.loads(s)
            self.assertIs(SpecialAttrsT, loaded)

        self.assertEqual(SpecialAttrsP.__name__, 'SpecialAttrsP')
        self.assertFalse(hasattr(SpecialAttrsP, '__qualname__'))
        self.assertEqual(SpecialAttrsP.__module__, __name__)
        # Module-level ParamSpecs are picklable.
        for proto in range(pickle.HIGHEST_PROTOCOL + 1):
            s = pickle.dumps(SpecialAttrsP, proto)
            loaded = pickle.loads(s)
            self.assertIs(SpecialAttrsP, loaded)

    def test_genericalias_dir(self):
        class Foo(Generic[T]):
            def bar(self):
                pass
            baz = 3
        # The class attributes of the original class should be visible even
        # in dir() of the GenericAlias. See bpo-45755.
        self.assertIn('bar', dir(Foo[int]))
        self.assertIn('baz', dir(Foo[int]))


class RevealTypeTests(BaseTestCase):
    def test_reveal_type(self):
        obj = object()
        with captured_stderr() as stderr:
            self.assertIs(obj, reveal_type(obj))
        self.assertEqual(stderr.getvalue(), "Runtime type is 'object'\n")


class DataclassTransformTests(BaseTestCase):
    def test_decorator(self):
        def create_model(*, frozen: bool = False, kw_only: bool = True):
            return lambda cls: cls

        decorated = dataclass_transform(kw_only_default=True, order_default=False)(create_model)

        class CustomerModel:
            id: int

        self.assertIs(decorated, create_model)
        self.assertEqual(
            decorated.__dataclass_transform__,
            {
                "eq_default": True,
                "order_default": False,
                "kw_only_default": True,
                "field_specifiers": (),
                "kwargs": {},
            }
        )
        self.assertIs(
            decorated(frozen=True, kw_only=False)(CustomerModel),
            CustomerModel
        )

    def test_base_class(self):
        class ModelBase:
            def __init_subclass__(cls, *, frozen: bool = False): ...

        Decorated = dataclass_transform(
            eq_default=True,
            order_default=True,
            # Arbitrary unrecognized kwargs are accepted at runtime.
            make_everything_awesome=True,
        )(ModelBase)

        class CustomerModel(Decorated, frozen=True):
            id: int

        self.assertIs(Decorated, ModelBase)
        self.assertEqual(
            Decorated.__dataclass_transform__,
            {
                "eq_default": True,
                "order_default": True,
                "kw_only_default": False,
                "field_specifiers": (),
                "kwargs": {"make_everything_awesome": True},
            }
        )
        self.assertIsSubclass(CustomerModel, Decorated)

    def test_metaclass(self):
        class Field: ...

        class ModelMeta(type):
            def __new__(
                cls, name, bases, namespace, *, init: bool = True,
            ):
                return super().__new__(cls, name, bases, namespace)

        Decorated = dataclass_transform(
            order_default=True, field_specifiers=(Field,)
        )(ModelMeta)

        class ModelBase(metaclass=Decorated): ...

        class CustomerModel(ModelBase, init=False):
            id: int

        self.assertIs(Decorated, ModelMeta)
        self.assertEqual(
            Decorated.__dataclass_transform__,
            {
                "eq_default": True,
                "order_default": True,
                "kw_only_default": False,
                "field_specifiers": (Field,),
                "kwargs": {},
            }
        )
        self.assertIsInstance(CustomerModel, Decorated)


class AllTests(BaseTestCase):
    """Tests for __all__."""

    def test_all(self):
        from typing import __all__ as a
        # Just spot-check the first and last of every category.
        self.assertIn('AbstractSet', a)
        self.assertIn('ValuesView', a)
        self.assertIn('cast', a)
        self.assertIn('overload', a)
        # Context managers.
        self.assertIn('ContextManager', a)
        self.assertIn('AsyncContextManager', a)
        # Check that io and re are not exported.
        self.assertNotIn('io', a)
        self.assertNotIn('re', a)
        # Spot-check that stdlib modules aren't exported.
        self.assertNotIn('os', a)
        self.assertNotIn('sys', a)
        # Check that Text is defined.
        self.assertIn('Text', a)
        # Check previously missing classes.
        self.assertIn('SupportsBytes', a)
        self.assertIn('SupportsComplex', a)

    def test_all_exported_names(self):
        actual_all = set(typing.__all__)
        computed_all = {
            k for k, v in vars(typing).items()
            # explicitly exported, not a thing with __module__
            if k in actual_all or (
                # avoid private names
                not k.startswith('_') and
                k not in {'io', 're'} and
                # there's a few types and metaclasses that aren't exported
                not k.endswith(('Meta', '_contra', '_co')) and
                not k.upper() == k and
                # but export all things that have __module__ == 'typing'
                getattr(v, '__module__', None) == typing.__name__
            )
        }
        self.assertSetEqual(computed_all, actual_all)


class TypeIterationTests(BaseTestCase):
    _UNITERABLE_TYPES = (
        Any,
        Union,
        Union[str, int],
        Union[str, T],
        List,
        Tuple,
        Callable,
        Callable[..., T],
        Callable[[T], str],
        Annotated,
        Annotated[T, ''],
    )

    def test_cannot_iterate(self):
        expected_error_regex = "object is not iterable"
        for test_type in self._UNITERABLE_TYPES:
            with self.subTest(type=test_type):
                with self.assertRaisesRegex(TypeError, expected_error_regex):
                    iter(test_type)
                with self.assertRaisesRegex(TypeError, expected_error_regex):
                    list(test_type)
                with self.assertRaisesRegex(TypeError, expected_error_regex):
                    for _ in test_type:
                        pass

    def test_is_not_instance_of_iterable(self):
        for type_to_test in self._UNITERABLE_TYPES:
            self.assertNotIsInstance(type_to_test, collections.abc.Iterable)


if __name__ == '__main__':
    main()
