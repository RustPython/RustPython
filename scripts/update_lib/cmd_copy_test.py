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
        result = copy_test(
            source.decode("utf-8"), destination.decode("utf-8"), args.test
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
