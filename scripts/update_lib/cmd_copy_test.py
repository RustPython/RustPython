"""Copy one complete canonical test definition without refreshing its module."""

import argparse
import ast
import hashlib
import json
from pathlib import Path

from update_lib.patch_spec import _iter_class_methods, _iter_module_classes


def _definition(contents: str, qualified_name: str):
    parts = qualified_name.split(".")
    if len(parts) != 2 or not all(part.isidentifier() for part in parts):
        raise ValueError("test name must be ClassName.method_name")
    tree = ast.parse(contents)
    classes = [node for node in _iter_module_classes(tree) if node.name == parts[0]]
    if len(classes) != 1:
        raise ValueError(f"Expected one class named {parts[0]}, found {len(classes)}")
    methods = [
        node for node in _iter_class_methods(classes[0]) if node.name == parts[1]
    ]
    if len(methods) != 1:
        raise ValueError(
            f"Expected one method named {qualified_name}, found {len(methods)}"
        )
    method = methods[0]
    parents = {
        child: parent
        for parent in ast.walk(tree)
        for child in ast.iter_child_nodes(parent)
    }
    guards = []
    child = method
    while child in parents:
        parent = parents[child]
        if isinstance(parent, ast.If):
            branch = "body" if child in parent.body else "orelse"
            guards.append((ast.dump(parent.test), branch))
        elif not isinstance(parent, (ast.Module, ast.ClassDef)):
            raise ValueError("Only module/class scopes and if guards are supported")
        child = parent
    start = min([method.lineno, *(node.lineno for node in method.decorator_list)]) - 1
    return method, guards, start, method.end_lineno


def insert_test(source: str, destination: str, qualified_name: str, before: str) -> str:
    """Insert an absent direct method before an unchanged canonical anchor.

    This intentionally excludes guarded classes/methods and non-LF inputs.
    It copies no imports or helpers: callers must review those dependencies.
    """
    if any("\r" in text or not text.endswith("\n") for text in (source, destination)):
        raise ValueError("Insertion requires LF input with a final newline")
    method, guards, start, end = _definition(source, qualified_name)
    anchor, anchor_guards, anchor_start, anchor_end = _definition(source, before)
    dst_anchor, dst_guards, dst_start, dst_end = _definition(destination, before)
    class_name, name = qualified_name.split(".")
    if before.split(".")[0] != class_name:
        raise ValueError("Insertion anchor must be in the same class")
    if guards or anchor_guards or dst_guards:
        raise ValueError("Insertion does not support guarded definitions")
    src_tree, dst_tree = ast.parse(source), ast.parse(destination)
    src_class = next(
        node
        for node in src_tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    dst_class = next(
        node
        for node in dst_tree.body
        if isinstance(node, ast.ClassDef) and node.name == class_name
    )
    if any(
        isinstance(node, (ast.Global, ast.Nonlocal))
        for cls in (src_class, dst_class)
        for node in ast.walk(cls)
    ):
        raise ValueError("Insertion does not support scope declarations in classes")
    # Nodes come from separate parses; compare their unique source locations.
    members = {node.lineno: index for index, node in enumerate(src_class.body)}
    if members.get(anchor.lineno) != members.get(method.lineno, -2) + 1:
        raise ValueError("Source test must immediately precede its direct anchor")
    if not any(node.lineno == dst_anchor.lineno for node in dst_class.body):
        raise ValueError("Destination anchor must be a direct class member")
    for node in ast.walk(dst_class):
        if (
            (
                isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
                and node.name == name
            )
            or (
                isinstance(node, ast.Name)
                and isinstance(node.ctx, ast.Store)
                and node.id == name
            )
            or (
                isinstance(node, ast.alias)
                and (node.asname or node.name.split(".")[0]) == name
            )
            or (
                isinstance(node, (ast.MatchAs, ast.MatchStar, ast.ExceptHandler))
                and node.name == name
            )
            or (isinstance(node, ast.MatchMapping) and node.rest == name)
        ):
            raise ValueError("Insertion test name already exists in destination class")

    # AST line numbers count LF, not Unicode separators embedded in strings.
    def lines(text):
        return [line + "\n" for line in text.split("\n")[:-1]]

    src_lines, dst_lines = lines(source), lines(destination)

    def header(cls, lines):
        first = min([cls.lineno, *(node.lineno for node in cls.decorator_list)]) - 1
        child = cls.body[0]
        child_start = (
            min(
                [
                    child.lineno,
                    *(node.lineno for node in getattr(child, "decorator_list", [])),
                ]
            )
            - 1
        )
        return lines[first:child_start]

    if header(src_class, src_lines) != header(dst_class, dst_lines):
        raise ValueError("Insertion class headers differ")
    if src_lines[anchor_start:anchor_end] != dst_lines[dst_start:dst_end]:
        raise ValueError("Insertion anchor text differs")
    if (
        method.col_offset != anchor.col_offset
        or method.col_offset != dst_anchor.col_offset
    ):
        raise ValueError("Insertion indentation differs")
    # Do not split or silently omit comments attached to either definition.
    previous = [node.end_lineno for node in src_class.body if node.end_lineno <= start]
    previous_end = max(previous, default=src_class.lineno)
    dst_previous = [
        node.end_lineno for node in dst_class.body if node.end_lineno <= dst_start
    ]
    dst_previous_end = max(dst_previous, default=dst_class.lineno)
    for gap in (
        src_lines[previous_end:start],
        src_lines[end:anchor_start],
        dst_lines[dst_previous_end:dst_start],
    ):
        if any(line.strip() for line in gap):
            raise ValueError("Insertion requires blank definition boundaries")
    block = src_lines[start:anchor_start]
    result = "".join(dst_lines[:dst_start] + block + dst_lines[dst_start:])
    _, _, result_start, result_end = _definition(result, qualified_name)
    if lines(result)[result_start:result_end] != src_lines[start:end]:
        raise ValueError("Inserted definition is not byte-identical to the source")
    return result


def copy_test(source: str, destination: str, qualified_name: str) -> str:
    """Replace an unambiguous whole definition, preserving all surrounding bytes.

    Guard, signature, decorator and indentation differences require a broader
    migration rather than silently changing the selected test's execution context.
    """
    src, src_guards, src_start, src_end = _definition(source, qualified_name)
    dst, dst_guards, dst_start, dst_end = _definition(destination, qualified_name)
    if src_guards != dst_guards:
        raise ValueError("Source and destination test guards differ")

    def signature(node):
        return (
            ast.dump(node.args),
            ast.dump(node.returns) if node.returns is not None else None,
            [ast.dump(param) for param in getattr(node, "type_params", [])],
        )

    if type(src) is not type(dst) or signature(src) != signature(dst):
        raise ValueError("Source and destination test signatures differ")
    if [ast.dump(node) for node in src.decorator_list] != [
        ast.dump(node) for node in dst.decorator_list
    ]:
        raise ValueError(
            "Source and destination decorators differ; preserve markers explicitly"
        )
    if src.col_offset != dst.col_offset:
        raise ValueError("Source and destination indentation differs")
    source_lines = source.splitlines(keepends=True)
    destination_lines = destination.splitlines(keepends=True)
    # AST equality omits comments, including project marker rationales. Require
    # the complete decorator block to match before replacing the definition.
    if (
        source_lines[src_start : src.lineno - 1]
        != destination_lines[dst_start : dst.lineno - 1]
    ):
        raise ValueError(
            "Source and destination decorator text differs; preserve markers explicitly"
        )
    definition = source_lines[src_start:src_end]
    result = "".join(
        destination_lines[:dst_start] + definition + destination_lines[dst_end:]
    )
    _, _, result_start, result_end = _definition(result, qualified_name)
    if result.splitlines(keepends=True)[result_start:result_end] != definition:
        raise ValueError("Copied definition is not byte-identical to the source")
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--test", required=True, help="Exact ClassName.method_name")
    parser.add_argument(
        "--insert-before",
        help="Insert an absent method before ClassName.anchor; direct unguarded classes only",
    )
    parser.add_argument("--source-sha256", required=True)
    parser.add_argument("--destination-sha256", required=True)
    args = parser.parse_args(argv)
    try:
        source = args.source.read_bytes()
        destination = args.destination.read_bytes()
        for label, contents, expected in (
            ("source", source, args.source_sha256),
            ("destination", destination, args.destination_sha256),
        ):
            if hashlib.sha256(contents).hexdigest() != expected:
                raise ValueError(f"{label} SHA256 does not match the pinned input")
        inputs = (source.decode("utf-8"), destination.decode("utf-8"), args.test)
        result = (
            insert_test(*inputs, args.insert_before)
            if args.insert_before
            else copy_test(*inputs)
        ).encode("utf-8")
        # Recheck the destination immediately before writing so stale invocations
        # fail instead of overwriting an intervening edit.
        if args.destination.read_bytes() != destination:
            raise ValueError("Destination changed during test preparation")
        args.destination.write_bytes(result)
    except (OSError, UnicodeError, SyntaxError, ValueError) as exc:
        parser.exit(1, f"Error: {exc}\n")
    print(
        json.dumps(
            {
                "test": args.test,
                "source_sha256": args.source_sha256,
                "destination_before_sha256": args.destination_sha256,
                "destination_after_sha256": hashlib.sha256(result).hexdigest(),
            },
            sort_keys=True,
        )
    )
    return 0
