#!/usr/bin/env python
__doc__ = """
This tool helps with updating test files from CPython.

Quick Upgrade
-------------
    ./{fname} --quick-upgrade cpython/Lib/test/test_threading.py
    ./{fname} --quick-upgrade ../somewhere/Lib/threading.py

    Any path containing `/Lib/` will auto-detect the target:
    -> Extracts patches from Lib/... (auto-detected from path)
    -> Applies them to the source file
    -> Writes result to Lib/...

Examples
--------
To move the patches found in `Lib/test/foo.py` to `~/cpython/Lib/test/foo.py` then write the contents back to `Lib/test/foo.py`

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
import textwrap
import typing

if typing.TYPE_CHECKING:
    from collections.abc import Iterator

type Patches = dict[str, dict[str, list["PatchSpec"]]]

DEFAULT_INDENT = " " * 4
COMMENT = "TODO: RUSTPYTHON"
UT = "unittest"


@enum.unique
class UtMethod(enum.StrEnum):
    """
    UnitTest Method.
    """

    def _generate_next_value_(name, start, count, last_values) -> str:
        return name[0].lower() + name[1:]

    def has_args(self) -> bool:
        return self != self.ExpectedFailure

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

    @property
    def _reason(self) -> str:
        return f"{COMMENT}; {self.reason}".strip(" ;")

    @property
    def _attr_node(self) -> ast.Attribute:
        return ast.Attribute(value=ast.Name(id=UT), attr=self.ut_method)

    def as_ast_node(self) -> ast.Attribute | ast.Call:
        if not self.ut_method.has_args():
            return self._attr_node

        args = []
        if self.cond:
            args.append(ast.parse(self.cond).body[0].value)
        args.append(ast.Constant(value=self._reason))

        return ast.Call(func=self._attr_node, args=args, keywords=[])

    def as_decorator(self) -> str:
        unparsed = ast.unparse(self.as_ast_node())

        if not self.ut_method.has_args():
            unparsed = f"{unparsed} # {self._reason}"

        return f"@{unparsed}"


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
                    or getattr(attr_node.value, "id", None) != UT
                ):
                    continue

                cond = None
                try:
                    ut_method = UtMethod(attr_node.attr)
                except ValueError:
                    continue

                # If our ut_method has args then,
                # we need to search for a constant that contains our `COMMENT`.
                # Otherwise we need to search it in the raw source code :/
                if ut_method.has_args():
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
                else:
                    # Search first on decorator line, then in the line before
                    for line in lines[dec_node.lineno - 1 : dec_node.lineno - 3 : -1]:
                        if found := re.search(rf"{COMMENT}.?(.*)", line):
                            reason = found.group()
                            break
                    else:
                        # Didn't find our `COMMENT` :)
                        continue

                reason = reason.removeprefix(COMMENT).strip(";:, ")
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
    cache = {}  # Used in phase 2. Stores the end line location of a class name.

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
        patch_lines = "\n".join(spec.as_decorator() for spec in specs)
        yield (lineno - 1, textwrap.indent(patch_lines, indent))

    # Phase 2: Iterate and mark inhereted tests
    for cls_name, tests in patches.items():
        lineno = cache.get(cls_name)
        if not lineno:
            print(f"WARNING: {cls_name} does not exist in remote file", file=sys.stderr)
            continue

        for test_name, specs in tests.items():
            decorators = "\n".join(spec.as_decorator() for spec in specs)
            patch_lines = f"""
{decorators}
def {test_name}(self):
{DEFAULT_INDENT}return super().{test_name}()
""".rstrip()
            yield (lineno, textwrap.indent(patch_lines, DEFAULT_INDENT))


def has_unittest_import(tree: ast.Module) -> bool:
    """Check if 'import unittest' is already present in the file."""
    for node in tree.body:
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == UT and alias.asname is None:
                    return True
    return False


def find_import_insert_line(tree: ast.Module) -> int:
    """Find the line number after the last import statement."""
    last_import_line = None
    for node in tree.body:
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            last_import_line = node.end_lineno or node.lineno
    assert last_import_line is not None
    return last_import_line


def apply_patches(contents: str, patches: Patches) -> str:
    tree = ast.parse(contents)
    lines = contents.splitlines()

    modifications = list(iter_patch_lines(tree, patches))

    # If we have modifications and unittest is not imported, add it
    if modifications and not has_unittest_import(tree):
        import_line = find_import_insert_line(tree)
        modifications.append(
            (
                import_line,
                "\nimport unittest # XXX: RUSTPYTHON; importing to be able to skip tests",
            )
        )

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
        "--quick-upgrade",
        help="Quick upgrade: path containing /Lib/ (e.g., cpython/Lib/test/foo.py)",
        type=pathlib.Path,
        metavar="PATH",
    )
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

    group = parser.add_mutually_exclusive_group(required=False)
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

    # Quick upgrade: auto-fill --from, --to, -o from path
    if args.quick_upgrade is not None:
        # Normalize path separators to forward slashes for cross-platform support
        path_str = str(args.quick_upgrade).replace("\\", "/")
        lib_marker = "/Lib/"

        if lib_marker not in path_str:
            parser.error(
                f"--quick-upgrade path must contain '/Lib/' or '\\Lib\\' (got: {args.quick_upgrade})"
            )

        idx = path_str.index(lib_marker)
        lib_path = pathlib.Path(path_str[idx + 1 :])

        args.gather_from = lib_path
        args.to = args.quick_upgrade
        if args.output == "-":
            args.output = str(lib_path)

    # Validate required arguments
    if args.patches is None and args.gather_from is None:
        parser.error("--from or --patches is required (or use --quick-upgrade)")
    if args.to is None and not args.show_patches:
        parser.error("--to or --show-patches is required")

    if args.patches:
        patches = {
            cls_name: {
                test_name: [
                    PatchSpec(**spec)._replace(ut_method=UtMethod(spec["ut_method"]))
                    for spec in specs
                ]
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
