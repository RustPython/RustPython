"""
Low-level module for converting between test files and JSON patches.

This module handles:
- Extracting patches from test files (file -> JSON)
- Applying patches to test files (JSON -> file)
"""

from __future__ import annotations

import ast
import collections
import contextlib
import enum
import re
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
        return self not in (self.ExpectedFailure, self.ExpectedSuccess)

    def has_cond(self) -> bool:
        return self.endswith(("If", "Unless"))

    ExpectedFailure = enum.auto()
    ExpectedFailureIf = enum.auto()
    ExpectedFailureIfWindows = enum.auto()
    ExpectedSuccess = enum.auto()
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
        # ast.unparse uses single quotes; convert to double quotes for ruff compatibility
        unparsed = _single_to_double_quotes(unparsed)

        if not self.ut_method.has_args():
            unparsed = f"{unparsed}  # {self._reason}"

        return f"@{unparsed}"

    @classmethod
    def try_from_ast_node(
        cls, node: ast.Attribute | ast.Call, lines: list[str]
    ) -> typing.Self | None:
        if isinstance(node, ast.Attribute):
            attr_node = node
        elif isinstance(node, ast.Call):
            attr_node = node.func
        else:
            return

        if (
            isinstance(attr_node, ast.Name)
            or getattr(attr_node.value, "id", None) != UT
        ):
            return

        cond = None
        try:
            ut_method = UtMethod(attr_node.attr)
        except ValueError:
            return

        # If our ut_method has args then,
        # we need to search for a constant that contains our `COMMENT`.
        # Otherwise we need to search it in the raw source code :/
        if ut_method.has_args():
            reason = next(
                (
                    inner_node.value
                    for inner_node in ast.walk(node)
                    if isinstance(inner_node, ast.Constant)
                    and isinstance(inner_node.value, str)
                    and COMMENT in inner_node.value
                ),
                None,
            )

            # If we didn't find a constant containing <COMMENT>,
            # then we didn't put this decorator
            if not reason:
                return

            if ut_method.has_cond():
                cond = ast.unparse(node.args[0])
        else:
            pattern = re.compile(rf"{COMMENT}.?(.*)")
            dec_lineno = node.lineno

            curr_line = lines[dec_lineno - 1]
            prev_line = lines[dec_lineno - 2]

            # If we see our comment at the decorator line, take it
            if found := pattern.search(curr_line):
                reason = found.group()
            elif prev_line.strip().startswith("#") and (
                found := pattern.search(prev_line)
            ):
                # Search the previous line of the decorator,
                # only take the comment if the line starts with a `#`
                reason = found.group()
            else:
                # Didn't find our `COMMENT`, so the patch isn't ours :)
                return

        reason = reason.removeprefix(COMMENT).strip(";:, ")
        return cls(ut_method, cond, reason)


class PatchEntryVisitor(ast.NodeVisitor):
    def __init__(self, lines: list[str]):
        self.current_class = None
        self.patches = []
        self.lines = lines

    def patches_from_node(
        self, node: ast.FunctionDef | ast.AsyncFunctionDef
    ) -> Iterator[PatchEntry]:
        for dec_node in node.decorator_list:
            spec = PatchSpec.try_from_ast_node(dec_node, self.lines)

            if spec is None:
                continue

            yield PatchEntry(self.current_class, node.name, spec)

    def visit_AsyncFunctionDef(self, node: ast.AsyncFunctionDef):
        self.patches.extend(self.patches_from_node(node))
        # TODO: Support nested classes/methods
        # self.generic_visit(node)

    def visit_FunctionDef(self, node: ast.FunctionDef):
        self.patches.extend(self.patches_from_node(node))
        # TODO: Support nested classes/methods
        # self.generic_visit(node)

    def visit_ClassDef(self, node: ast.ClassDef):
        with temp_attr(self, "current_class", node.name):
            for patch in self.patches_from_node(node):
                patch = patch._replace(test_name="__self__")
                self.patches.append(patch)

            self.generic_visit(node)


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
    def iter_patch_entries(
        cls, tree: ast.Module, lines: list[str]
    ) -> "Iterator[typing.Self]":

        visitor = PatchEntryVisitor(lines)
        visitor.visit(tree)
        yield from visitor.patches


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
    yield from PatchEntry.iter_patch_entries(tree, lines)


def build_patch_dict(it: "Iterator[PatchEntry]") -> Patches:
    patches = collections.defaultdict(lambda: collections.defaultdict(list))
    for entry in it:
        patches[entry.parent_class][entry.test_name].append(entry.spec)

    return {k: dict(v) for k, v in patches.items()}


def extract_patches(contents: str) -> Patches:
    """Extract patches from file contents and return as dict."""
    return build_patch_dict(iter_patches(contents))


def modification_from_node_specs(node, specs):
    lineno = min(
        (dec_node.lineno for dec_node in node.decorator_list), default=node.lineno
    )
    indent = " " * node.col_offset
    patch_lines = "\n".join(spec.as_decorator() for spec in specs)
    return (lineno - 1, textwrap.indent(patch_lines, indent))


def _iter_patch_lines(
    tree: ast.Module, patches: Patches
) -> "Iterator[tuple[int, str]]":
    import sys

    # Build cache of all classes (for Phase 2 to find classes without methods)
    cache = {}
    # Build per-class set of async method names (for Phase 2 to generate correct override)
    async_methods: dict[str, set[str]] = {}
    # Track class bases for inherited async method lookup
    class_bases: dict[str, list[str]] = {}
    all_classes = set()
    all_class_nodes = []
    for node in tree.body:
        if not isinstance(node, ast.ClassDef):
            continue

        all_classes.add(node.name)
        all_class_nodes.append(node)

    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            cache[node.name] = node.end_lineno
            class_bases[node.name] = [
                base.id
                for base in node.bases
                if isinstance(base, ast.Name) and base.id in all_classes
            ]
            cls_async: set[str] = set()
            for item in node.body:
                if isinstance(item, ast.AsyncFunctionDef):
                    cls_async.add(item.name)
            if cls_async:
                async_methods[node.name] = cls_async

    # Phase 1: Iterate and mark existing tests
    for cls_node, fn_node in iter_tests(tree):
        specs = patches.get(cls_node.name, {}).pop(fn_node.name, None)
        if not specs:
            continue

        yield modification_from_node_specs(fn_node, specs)

    # Phase 2: Iterate and mark inherited tests
    for cls_name, tests in sorted(patches.items()):
        lineno = cache.get(cls_name)
        if not lineno:
            print(f"WARNING: {cls_name} does not exist in remote file", file=sys.stderr)
            continue

        for test_name, specs in sorted(tests.items()):
            if test_name == "__self__":
                # Yielding modifications for the class itself should be done during phase 3
                continue

            decorators = "\n".join(spec.as_decorator() for spec in specs)
            # Check current class and ancestors for async method
            is_async = False
            queue = [cls_name]
            visited: set[str] = set()
            while queue:
                cur = queue.pop(0)
                if cur in visited:
                    continue
                visited.add(cur)
                if test_name in async_methods.get(cur, set()):
                    is_async = True
                    break
                queue.extend(class_bases.get(cur, []))

            if is_async:
                patch_lines = f"""
{decorators}
async def {test_name}(self):
{DEFAULT_INDENT}return await super().{test_name}()
""".rstrip()
            else:
                patch_lines = f"""
{decorators}
def {test_name}(self):
{DEFAULT_INDENT}return super().{test_name}()
""".rstrip()
            yield (lineno, textwrap.indent(patch_lines, DEFAULT_INDENT))

    # Phase 3: Mark the class itself
    for cls_node in all_class_nodes:
        if cls_specs := patches.get(cls_node.name, {}).pop("__self__", None):
            yield modification_from_node_specs(cls_node, cls_specs)


def _has_unittest_import(tree: ast.Module) -> bool:
    """Check if 'import unittest' is already present in the file."""
    for node in tree.body:
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name == UT and alias.asname is None:
                    return True
    return False


def _find_import_insert_line(tree: ast.Module) -> int:
    """Find the line number after the last import statement."""
    last_import_line = None
    for node in tree.body:
        if isinstance(node, (ast.Import, ast.ImportFrom)):
            last_import_line = node.end_lineno or node.lineno
    if last_import_line is not None:
        return last_import_line
    # No imports found - insert after module docstring if present, else at top
    if (
        tree.body
        and isinstance(tree.body[0], ast.Expr)
        and isinstance(tree.body[0].value, ast.Constant)
        and isinstance(tree.body[0].value.value, str)
    ):
        return tree.body[0].end_lineno or tree.body[0].lineno
    return 0


def apply_patches(contents: str, patches: Patches) -> str:
    """Apply patches to file contents and return modified contents."""
    tree = ast.parse(contents)
    lines = contents.splitlines()

    modifications = list(_iter_patch_lines(tree, patches))

    # If we have modifications and unittest is not imported, add it
    if modifications and not _has_unittest_import(tree):
        import_line = _find_import_insert_line(tree)
        modifications.append(
            (
                import_line,
                "\nimport unittest  # XXX: RUSTPYTHON; importing to be able to skip tests",
            )
        )

    # Going in reverse to not disrupt the line offset
    for lineno, patch in sorted(modifications, reverse=True):
        lines.insert(lineno, patch)

    joined = "\n".join(lines)
    return f"{joined}\n"


def patches_to_json(patches: Patches) -> dict:
    """Convert patches to JSON-serializable dict."""
    return {
        cls_name: {
            test_name: [spec._asdict() for spec in specs]
            for test_name, specs in tests.items()
        }
        for cls_name, tests in patches.items()
    }


def patches_from_json(data: dict) -> Patches:
    """Convert JSON dict back to Patches."""
    return {
        cls_name: {
            test_name: [
                PatchSpec(**spec)._replace(ut_method=UtMethod(spec["ut_method"]))
                for spec in specs
            ]
            for test_name, specs in tests.items()
        }
        for cls_name, tests in data.items()
    }


def _single_to_double_quotes(s: str) -> str:
    """
    Convert single-quoted strings to double-quoted strings.

    Falls back to original if conversion breaks the AST equivalence.
    """
    import re

    def replace_string(match: re.Match) -> str:
        content = match.group(1)
        # Unescape single quotes and escape double quotes
        content = content.replace("\\'", "'").replace('"', '\\"')
        return f'"{content}"'

    # Match single-quoted strings (handles escaped single quotes inside)
    converted = re.sub(r"'((?:[^'\\]|\\.)*)'", replace_string, s)

    # Verify: parse converted and unparse should equal original
    try:
        converted_ast = ast.parse(converted, mode="eval")
        if ast.unparse(converted_ast) == s:
            return converted
    except SyntaxError:
        pass

    # Fall back to original if conversion failed
    return s


@contextlib.contextmanager
def temp_attr(obj: object, attr: str, value: object):
    old = getattr(obj, attr, None)
    setattr(obj, attr, value)
    try:
        yield obj
    finally:
        setattr(obj, attr, old)
