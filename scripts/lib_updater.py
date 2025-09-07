#!/usr/bin/env python
__doc__ = """
This tool helps with updating test files from CPython.

Examples
--------
To move the patches found in `Lib/test/foo.py` to ` ~/cpython/Lib/test/foo.py` then write the contents back to `Lib/test/foo.py`

>>> ./{fname} --from Lib/test/foo.py --to ~/cpython/Lib/test/foo.py -o Lib/test/foo.py

You can run the same command without `-o` to override the `--from` path:

>>> ./{fname} --from Lib/test/foo.py --to ~/cpython/Lib/test/foo.py

To get a baseline of patches, you can alter the patches file with your favorite tool/script/etc and then reapply it with:

>>> ./{fname} --from Lib/test/foo.py --show-patches -o my_patches.json

(By default the output is set to print to stdout).

When you want to apply your own patches:

>>> ./{fname} -p my_patches.json --to Lib/test/foo.py
""".format(fname=__import__("os").path.basename(__file__))


import argparse
import ast
import collections
import enum
import json
import pathlib
import re
import sys
import typing

if typing.TYPE_CHECKING:
    from collections.abc import Iterator

type Patches = dict[str, dict[str, list["PatchSpec"]]]

COL_OFFSET = 4
INDENT1 = " " * COL_OFFSET
INDENT2 = INDENT1 * 2
COMMENT = "TODO: RUSTPYTHON"


@enum.unique
class UtMethod(enum.StrEnum):
    """
    UnitTest Method.
    """

    def _generate_next_value_(name, start, count, last_values) -> str:
        return name[0].lower() + name[1:]

    def has_cond(self) -> bool:
        return self.endswith(("If", "Unless"))

    ExpectedFailure = enum.auto()
    ExpectedFailureIf = enum.auto()
    ExpectedFailureIfWindows = enum.auto()
    Skip = enum.auto()
    SkipIf = enum.auto()
    SkipUnless = enum.auto()


class PatchSpec(typing.NamedTuple):
    """
    Attributes
    ----------
    ut_method : UtMethod
        unittest method.
    cond : str, optional
        `ut_method` condition. Relevant only for some of `ut_method` types.
    reason : str, optional
        Reason for why the test is patched in this way.
    """

    ut_method: UtMethod
    cond: str | None = None
    reason: str = ""

    def fmt(self) -> str:
        prefix = f"@unittest.{self.ut_method}"
        match self.ut_method:
            case UtMethod.ExpectedFailure:
                line = f"{prefix} # {COMMENT}; {self.reason}"
            case UtMethod.ExpectedFailureIfWindows | UtMethod.Skip:
                line = f'{prefix}("{COMMENT}; {self.reason}")'
            case UtMethod.SkipIf | UtMethod.SkipUnless | UtMethod.ExpectedFailureIf:
                line = f'{prefix}({self.cond}, "{COMMENT}; {self.reason}")'

        return line.strip().rstrip(";").strip()


class PatchEntry(typing.NamedTuple):
    """
    Stores patch metadata.

    Attributes
    ----------
    parent_class : str
        Parent class of test.
    test_name : str
        Test name.
    spec : PatchSpec
        Patch spec.
    """

    parent_class: str
    test_name: str
    spec: PatchSpec

    @classmethod
    def iter_patch_entires(
        cls, tree: ast.Module, lines: list[str]
    ) -> "Iterator[typing.Self]":
        for cls_node, fn_node in iter_tests(tree):
            parent_class = cls_node.name
            for dec_node in fn_node.decorator_list:
                if not isinstance(dec_node, (ast.Attribute, ast.Call)):
                    continue

                attr_node = (
                    dec_node if isinstance(dec_node, ast.Attribute) else dec_node.func
                )

                if (
                    isinstance(attr_node, ast.Name)
                    or getattr(attr_node.value, "id", None) != "unittest"
                ):
                    continue

                cond = None
                try:
                    ut_method = UtMethod(attr_node.attr)
                except ValueError:
                    continue

                match ut_method:
                    case UtMethod.ExpectedFailure:
                        # Search first on decorator line, then in the line before
                        for line in lines[
                            dec_node.lineno - 1 : dec_node.lineno - 3 : -1
                        ]:
                            if COMMENT not in line:
                                continue
                            reason = "".join(re.findall(rf"{COMMENT}.?(.*)", line))
                            break
                        else:
                            continue
                    case _:
                        reason = next(
                            (
                                node.value
                                for node in ast.walk(dec_node)
                                if isinstance(node, ast.Constant)
                                and isinstance(node.value, str)
                                and COMMENT in node.value
                            ),
                            None,
                        )

                        # If we didn't find a constant containing <COMMENT>,
                        # then we didn't put this decorator
                        if not reason:
                            continue

                        if ut_method.has_cond():
                            cond = ast.unparse(dec_node.args[0])

                reason = (
                    reason.replace(COMMENT, "").strip().lstrip(";").lstrip(":").strip()
                )
                spec = PatchSpec(ut_method, cond, reason)
                yield cls(parent_class, fn_node.name, spec)


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


def build_patch_dict(it: "Iterator[PatchEntry]") -> Patches:
    patches = collections.defaultdict(lambda: collections.defaultdict(list))
    for entry in it:
        patches[entry.parent_class][entry.test_name].append(entry.spec)

    return {k: dict(v) for k, v in patches.items()}


def iter_patch_lines(tree: ast.Module, patches: Patches) -> "Iterator[tuple[int, str]]":
    cache = {}  # Used in phase 2

    # Phase 1: Iterate and mark existing tests
    for cls_node, fn_node in iter_tests(tree):
        cache[cls_node.name] = cls_node.end_lineno
        specs = patches.get(cls_node.name, {}).pop(fn_node.name, None)
        if not specs:
            continue

        lineno = min(
            (dec_node.lineno for dec_node in fn_node.decorator_list),
            default=fn_node.lineno,
        )
        indent = " " * fn_node.col_offset
        yield (lineno - 1, "\n".join(f"{indent}{spec.fmt()}" for spec in specs))

    # Phase 2: Iterate and mark inhereted tests
    for cls_name, tests in patches.items():
        lineno = cache.get(cls_name)
        if not lineno:
            print(f"WARNING: {cls_name} does not exist in remote file", file=sys.stderr)
            continue
        for test_name, specs in tests.items():
            patch_lines = "\n".join(f"{INDENT1}{spec.fmt()}" for spec in specs)
            yield (
                lineno,
                f"""
{patch_lines}
{INDENT1}def {test_name}(self):
{INDENT2}return super().{test_name}()
""".rstrip(),
            )


def apply_patches(contents: str, patches: Patches) -> str:
    tree = ast.parse(contents)
    lines = contents.splitlines()

    modifications = list(iter_patch_lines(tree, patches))
    # Going in reverse to not distrupt the line offset
    for lineno, patch in sorted(modifications, reverse=True):
        lines.insert(lineno, patch)

    joined = "\n".join(lines)
    return f"{joined}\n"


def write_output(data: str, dest: str) -> None:
    if dest == "-":
        print(data, end="")
        return

    with open(dest, "w") as fd:
        fd.write(data)


def build_argparse() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )

    patches_group = parser.add_mutually_exclusive_group(required=True)
    patches_group.add_argument(
        "-p",
        "--patches",
        help="File path to file containing patches in a JSON format",
        type=pathlib.Path,
    )
    patches_group.add_argument(
        "--from",
        help="File to gather patches from",
        dest="gather_from",
        type=pathlib.Path,
    )

    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--to",
        help="File to apply patches to",
        type=pathlib.Path,
    )
    group.add_argument(
        "--show-patches", action="store_true", help="Show the patches and exit"
    )

    parser.add_argument(
        "-o", "--output", default="-", help="Output file. Set to '-' for stdout"
    )

    return parser


if __name__ == "__main__":
    parser = build_argparse()
    args = parser.parse_args()

    if args.patches:
        patches = {
            cls_name: {
                test_name: [PatchSpec(**spec) for spec in specs]
                for test_name, specs in tests.items()
            }
            for cls_name, tests in json.loads(args.patches.read_text()).items()
        }
    else:
        patches = build_patch_dict(iter_patches(args.gather_from.read_text()))

    if args.show_patches:
        patches = {
            cls_name: {
                test_name: [spec._asdict() for spec in specs]
                for test_name, specs in tests.items()
            }
            for cls_name, tests in patches.items()
        }
        output = json.dumps(patches, indent=4) + "\n"
        write_output(output, args.output)
        sys.exit(0)

    patched = apply_patches(args.to.read_text(), patches)
    write_output(patched, args.output)
