#!/usr/bin/env python
import argparse
import ast
import dataclasses
import enum
import json
import re
import sys
from typing import TYPE_CHECKING, Self

if TYPE_CHECKING:
    from collections.abc import Iterator

COMMENT = "TODO: RUSTPYTHON"


@enum.unique
class ProgName(enum.StrEnum):
    Gen = enum.auto()
    Patch = enum.auto()


@enum.unique
class UtMethod(enum.StrEnum):
    """
    UnitTest Method.
    """

    def _generate_next_value_(name, start, count, last_values) -> str:
        return name[0].lower() + name[1:]

    ExpectedFailure = enum.auto()
    ExpectedFailureIf = enum.auto()
    ExpectedFailureIfWindows = enum.auto()
    Skip = enum.auto()
    SkipIf = enum.auto()
    SkipUnless = enum.auto()


@dataclasses.dataclass(frozen=True, slots=True)
class PatchEntry:
    """
    Stores patch metadata.

    Attributes
    ----------
    parent_class : str
        Parent class of test.
    test_name : str
        Test name.
    ut_method : UtMethod
        unittest method.
    cond : str, optional
        `ut_method` condition. Relevant only for UtMethod.{expectedFailureIf,skipIf}.
    reason : str, optional
        Reason for why the test is patched in this way.
    """

    parent_class: str
    test_name: str
    ut_method: UtMethod
    cond: str | None = None
    reason: str = ""

    @classmethod
    def iter_patch_entires(cls, tree: ast.Module, lines: list[str]) -> "Iterator[Self]":
        for cls_node, fn_node in iter_tests(tree):
            parent_class = cls_node.name
            for dec_node in fn_node.decorator_list:
                if not isinstance(dec_node, (ast.Attribute, ast.Call)):
                    continue

                attr_node = (
                    dec_node if isinstance(dec_node, ast.Attribute) else dec_node.func
                )

                if isinstance(attr_node, ast.Name) or attr_node.value.id != "unittest":
                    continue

                cond = None
                match attr_node.attr:
                    case UtMethod.ExpectedFailure:
                        for line in lines[dec_node.lineno - 2 : dec_node.lineno]:
                            if COMMENT not in line:
                                continue
                            reason = "".join(re.findall(rf"{COMMENT} (.*)", line))
                            break
                        else:
                            continue
                    case (
                        UtMethod.Skip
                        | UtMethod.SkipIf
                        | UtMethod.ExpectedFailureIf
                        | UtMethod.ExpectedFailureIfWindows
                    ):
                        reason = next(
                            (
                                node.value
                                for node in ast.walk(dec_node)
                                if isinstance(node, ast.Constant)
                                and isinstance(node.value, str)
                                and node.value.startswith(COMMENT)
                            ),
                            None,
                        )

                        # If we didn't find a constant with the COMMENT, then we didn't put this decorator
                        if not reason:
                            continue

                        if attr_node.attr not in (
                            UtMethod.Skip,
                            UtMethod.ExpectedFailureIfWindows,
                        ):
                            cond = ast.unparse(dec_node.args[0])
                    case _:
                        continue

                yield cls(
                    parent_class,
                    fn_node.name,
                    UtMethod(attr_node.attr),
                    cond,
                    reason.replace(COMMENT, "").strip().lstrip(";").lstrip(":").strip(),
                )


def iter_tests(
    tree: ast.Module,
) -> "Iterator[tuple[ast.ClassDef, ast.FunctionDef | ast.AsyncFunctionDef]]":
    for key, nodes in ast.iter_fields(tree):
        if key != "body":
            continue

        for cls_node in nodes:
            if not isinstance(cls_node, ast.ClassDef):
                continue

            for fn_node in cls_node.body:
                if not isinstance(fn_node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue

                yield (cls_node, fn_node)


def iter_patches(contents: str) -> "Iterator[PatchEntry]":
    lines = contents.splitlines()
    tree = ast.parse(contents)
    yield from PatchEntry.iter_patch_entires(tree, lines)


def read_infile(infile: str) -> str:
    if infile == "-":
        return sys.stdin.read()

    with open(infile, mode="r", encoding="utf-8") as fd:
        return fd.read()


def build_argparse() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Helper tool for updating files under Lib/"
    )

    subparsers = parser.add_subparsers(dest="pname", required=True)

    # Gen
    parser_gen = subparsers.add_parser(ProgName.Gen)
    parser_gen.add_argument(
        "infile",
        default="-",
        help="File path to generate patches from, can get from stdin",
        nargs="?",
    )

    # Patch
    parser_patch = subparsers.add_parser(ProgName.Patch)
    parser_patch.add_argument("src", help="File path to apply patches for")
    parser_patch.add_argument(
        "infile",
        default="-",
        help="File path containing patches, can get from stdin",
        nargs="?",
    )

    return parser


if __name__ == "__main__":
    parser = build_argparse()
    args = parser.parse_args()

    contents = read_infile(args.infile)
    match args.pname:
        case ProgName.Gen:
            patches = list(map(dataclasses.asdict, iter_patches(contents)))
            output = json.dumps(patches, indent=4)
        case ProgName.Patch:
            pass  # TODO

    sys.stdout.write(f"{output}\n")
    sys.stdout.flush()
