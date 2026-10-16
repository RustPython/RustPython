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
import doctest
import enum
import re
import textwrap
import typing
from pathlib import Path

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
        if ut_method.has_args() and not (
            ut_method is UtMethod.Skip and isinstance(node, ast.Attribute)
        ):
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

            # Keep conditional methods in this namespace, but do not enter
            # nested classes or the bodies of methods.
            for item in _iter_class_methods(node):
                self.visit(item)


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


def _iter_module_classes(node: ast.AST) -> "Iterator[ast.ClassDef]":
    """Find module-scope classes, including conditional definitions.

    Function-local and class-nested definitions have different namespaces and
    cannot be addressed by the patch format's unqualified class names.
    """
    for child in ast.iter_child_nodes(node):
        if isinstance(child, ast.ClassDef):
            yield child
        elif not isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            yield from _iter_module_classes(child)


def _iter_class_methods(
    node: ast.AST,
) -> "Iterator[ast.FunctionDef | ast.AsyncFunctionDef]":
    for child in ast.iter_child_nodes(node):
        if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef)):
            yield child
        elif not isinstance(child, (ast.ClassDef, ast.Lambda)):
            yield from _iter_class_methods(child)


def iter_tests(
    tree: ast.Module,
) -> "Iterator[tuple[ast.ClassDef, ast.FunctionDef | ast.AsyncFunctionDef]]":
    for cls_node in _iter_module_classes(tree):
        for fn_node in _iter_class_methods(cls_node):
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
    all_class_nodes = list(_iter_module_classes(tree))
    # Only names used to place a patch (including inherited methods) need a
    # unique definition. Conditional, unpatched helpers are common upstream.
    relevant_classes = set(patches)
    while True:
        ancestors = {
            base.id
            for node in all_class_nodes
            if node.name in relevant_classes
            for base in node.bases
            if isinstance(base, ast.Name)
        }
        expanded = relevant_classes | ancestors
        if expanded == relevant_classes:
            break
        relevant_classes = expanded
    for node in all_class_nodes:
        if node.name in all_classes and node.name in relevant_classes:
            raise ValueError(f"Ambiguous module-scope class name: {node.name}")
        all_classes.add(node.name)
        method_counts = collections.Counter(
            item.name for item in _iter_class_methods(node)
        )
        for method in patches.get(node.name, {}):
            if method_counts[method] > 1:
                raise ValueError(f"Ambiguous method name: {node.name}.{method}")

    for node in all_class_nodes:
        if isinstance(node, ast.ClassDef):
            cache[node.name] = (node.end_lineno, node.col_offset + len(DEFAULT_INDENT))
            class_bases[node.name] = [
                base.id
                for base in node.bases
                if isinstance(base, ast.Name) and base.id in all_classes
            ]
            cls_async: set[str] = set()
            for item in _iter_class_methods(node):
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
        location = cache.get(cls_name)
        if not location:
            print(f"WARNING: {cls_name} does not exist in remote file", file=sys.stderr)
            continue
        lineno, indent = location

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
            yield (lineno, textwrap.indent(patch_lines, " " * indent))

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


def _expand_inline_class_suites(contents: str, patches: Patches) -> str:
    """Make room for inherited overrides without changing existing class bodies."""
    tree = ast.parse(contents)
    lines = contents.splitlines()
    replacements = []
    for node in _iter_module_classes(tree):
        methods = {method.name for method in _iter_class_methods(node)}
        inherited = set(patches.get(node.name, {})) - methods - {"__self__"}
        if not inherited:
            continue
        first = node.body[0]
        line = lines[first.lineno - 1].encode("utf-8")
        header = line[: first.col_offset].decode("utf-8")
        if not header.strip():
            continue
        body = line[first.col_offset :].decode("utf-8")
        replacements.append(
            (
                first.lineno - 1,
                header.rstrip() + "\n" + " " * (node.col_offset + 4) + body,
            )
        )
    for index, replacement in sorted(replacements, reverse=True):
        lines[index] = replacement
    return "\n".join(lines) + "\n"


def apply_patches(contents: str, patches: Patches) -> str:
    """Apply patches to file contents and return modified contents."""
    contents = _expand_inline_class_suites(contents, patches)
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
    ast.parse(joined)
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


def select_remaining_patches(current, document, relative_path: Path):
    if not isinstance(document, dict) or (
        set(document) not in ({"target", "patches"}, {"target", "patches", "doctests"})
    ):
        raise ValueError("Expected target and patches metadata")
    if (
        relative_path.parts[:2] != ("Lib", "test")
        or relative_path.suffix != ".py"
        or document["target"] != relative_path.as_posix()
    ):
        raise ValueError("Marker override target does not match the test file")
    if "doctests" in document and not isinstance(document["doctests"], dict):
        raise ValueError("Doctests metadata must be an object")
    try:
        remaining = patches_from_json(document["patches"])
    except (TypeError, KeyError, AttributeError, ValueError) as error:
        raise ValueError("Invalid remaining marker metadata") from error
    for cls, methods in remaining.items():
        if cls not in current or any(method not in current[cls] for method in methods):
            raise ValueError("Marker override cannot add targets")
    for cls, methods in current.items():
        for method, existing in methods.items():
            pending = list(existing)
            omitted = []
            for marker in remaining.get(cls, {}).get(method, []):
                if marker not in pending:
                    raise ValueError(
                        "Marker override cannot add, change, or reorder markers"
                    )
                index = pending.index(marker)
                omitted.extend(pending[:index])
                pending = pending[index + 1 :]
            omitted.extend(pending)
            if any(marker.ut_method != UtMethod.ExpectedFailure for marker in omitted):
                raise ValueError(
                    "Only unconditional expectedFailure markers may be removed"
                )
    return remaining


EXPECTED_FAILURE = doctest.register_optionflag("EXPECTED_FAILURE")
CANONICAL_LOAD_TESTS = """def load_tests(loader, tests, pattern):
    tests.addTest(doctest.DocTestSuite())
    return tests"""
PATCHED_LOAD_TESTS = """def load_tests(loader, tests, pattern):
    from test.support.rustpython import DocTestChecker  # TODO: RUSTPYTHON
    tests.addTest(doctest.DocTestSuite(checker=DocTestChecker()))  # TODO: RUSTPYTHON
    return tests"""


def _examples(contents):
    tree = ast.parse(contents)
    if not tree.body or not isinstance(tree.body[0], ast.Expr):
        raise ValueError("Missing module doctest")
    doc = tree.body[0].value
    if not isinstance(doc, ast.Constant) or not isinstance(doc.value, str):
        raise ValueError("Missing module doctest")
    # Raw docstring offsets map directly to source lines, unlike cleaned docs.
    return tree, doc, doctest.DocTestParser().get_examples(doc.value)


def apply_doctest_metadata(old, canonical, metadata):
    """Preserve only listed guards, without carrying old expected output."""
    if not isinstance(metadata, dict) or any(
        not isinstance(source, str)
        or not isinstance(reason, str)
        or "\n" in reason
        or "\r" in reason
        for source, reason in metadata.items()
    ):
        raise ValueError("Doctest metadata must map exact input to single-line reasons")
    _, _, old_examples = _examples(old)
    tree, doc, examples = _examples(canonical)
    lines = canonical.splitlines(keepends=True)
    for source in metadata:
        old_matches = [
            example
            for example in old_examples
            if re.sub(r"\s+# TODO: RUSTPYTHON[^\n]*", "", example.source) == source
        ]
        if len(old_matches) != 1 or not old_matches[0].options.get(EXPECTED_FAILURE):
            raise ValueError("Missing or ambiguous existing doctest guard")
        matches = [example for example in examples if example.source == source]
        if len(matches) != 1 or matches[0].options:
            raise ValueError("Missing, ambiguous, or already marked canonical doctest")
        index = doc.lineno - 1 + matches[0].lineno
        line = lines[index]
        if (
            ">>>" not in line
            or line.split(">>>", 1)[1].strip() != source.splitlines()[0]
        ):
            raise ValueError("Unrecognized canonical doctest prompt")
        lines[index] = (
            line.rstrip("\n")
            + "  # TODO: RUSTPYTHON; "
            + metadata[source]
            + " # doctest: +EXPECTED_FAILURE\n"
        )

    if not metadata:
        return canonical

    loaders = [
        node
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        and node.name == "load_tests"
    ]
    if len(loaders) != 1 or loaders[0] not in tree.body:
        raise ValueError("Missing or ambiguous doctest loader")
    if ast.get_source_segment(canonical, loaders[0]) != CANONICAL_LOAD_TESTS:
        raise ValueError("Changed canonical doctest loader")
    result = "".join(lines)
    if result.count(CANONICAL_LOAD_TESTS) != 1:
        raise ValueError("Ambiguous canonical doctest loader text")
    return result.replace(CANONICAL_LOAD_TESTS, PATCHED_LOAD_TESTS, 1)


def extract_doctest_guards(contents: str) -> dict[str, str]:
    """Extract guarded module-docstring inputs without executing examples."""
    tree = ast.parse(contents)
    if not tree.body or not isinstance(tree.body[0], ast.Expr):
        return {}
    doc = tree.body[0].value
    if not isinstance(doc, ast.Constant) or not isinstance(doc.value, str):
        return {}
    guards = {}
    for example in doctest.DocTestParser().get_examples(doc.value):
        if not example.options.get(EXPECTED_FAILURE):
            continue
        source = re.sub(r"\s+# TODO: RUSTPYTHON[^\n]*", "", example.source)
        if source in guards:
            raise ValueError("Ambiguous existing doctest guard")
        guards[source] = example.source
    return guards
