from __future__ import annotations

import argparse
import ast
import sys
from pathlib import Path
from typing import Iterable


SUMMARY_FIELDS: dict[type[ast.AST], tuple[str, ...]] = {
    ast.FunctionDef: ("name",),
    ast.AsyncFunctionDef: ("name",),
    ast.ClassDef: ("name",),
    ast.Name: ("id", "ctx"),
    ast.arg: ("arg",),
    ast.Attribute: ("attr", "ctx"),
    ast.Constant: ("value",),
    ast.Import: ("names",),
    ast.ImportFrom: ("module", "names", "level"),
    ast.alias: ("name", "asname"),
    ast.Assign: ("targets",),
    ast.AnnAssign: ("target",),
    ast.Call: ("func",),
}


def read_source(args: argparse.Namespace) -> tuple[str, str]:
    if args.file and args.code:
        raise SystemExit("choose either --file or --code")

    if args.file:
        path = Path(args.file)
        return path.read_text(encoding="utf-8"), str(path)

    if args.code:
        return args.code, "<string>"

    return sys.stdin.read(), "<stdin>"


def truncate(text: str, limit: int = 60) -> str:
    if len(text) <= limit:
        return text
    return text[: limit - 3] + "..."


def format_value(value: object) -> str:
    if isinstance(value, ast.AST):
        return type(value).__name__
    if isinstance(value, list):
        if value and all(isinstance(item, ast.alias) for item in value):
            parts = []
            for item in value:
                if item.asname:
                    parts.append(f"{item.name} as {item.asname}")
                else:
                    parts.append(item.name)
            return "[" + ", ".join(parts) + "]"
        return f"list[{len(value)}]"
    if value is None:
        return "None"
    return truncate(repr(value))


def node_summary(node: ast.AST, show_attrs: bool) -> str:
    fields = SUMMARY_FIELDS.get(type(node), ())
    parts: list[str] = []
    for field in fields:
        value = getattr(node, field, None)
        if field == "ctx" and isinstance(value, ast.AST):
            parts.append(f"{field}={type(value).__name__}")
        else:
            parts.append(f"{field}={format_value(value)}")

    if show_attrs:
        lineno = getattr(node, "lineno", None)
        col = getattr(node, "col_offset", None)
        if lineno is not None and col is not None:
            parts.append(f"@{lineno}:{col}")

    if parts:
        return f"{type(node).__name__} " + " ".join(parts)
    return type(node).__name__


def render_tree(
    node: ast.AST,
    lines: list[str],
    prefix: str,
    is_last: bool,
    max_depth: int,
    show_attrs: bool,
    depth: int = 0,
) -> None:
    connector = "`-- " if is_last else "|-- "
    lines.append(prefix + connector + node_summary(node, show_attrs))

    if depth >= max_depth:
        if list(ast.iter_child_nodes(node)):
            lines.append(prefix + ("    " if is_last else "|   ") + "`-- ...")
        return

    children = list(ast.iter_child_nodes(node))
    for idx, child in enumerate(children):
        last = idx == len(children) - 1
        next_prefix = prefix + ("    " if is_last else "|   ")
        render_tree(child, lines, next_prefix, last, max_depth, show_attrs, depth + 1)


def to_tree_text(tree: ast.AST, max_depth: int, show_attrs: bool) -> str:
    lines: list[str] = []
    render_tree(tree, lines, "", True, max_depth, show_attrs)
    return "\n".join(lines)


def dump_node(node: object, show_attrs: bool, indent: int, level: int) -> str:
    if isinstance(node, ast.AST):
        parts: list[str] = []
        for name, value in ast.iter_fields(node):
            parts.append(f"{name}={dump_node(value, show_attrs, indent, level + 1)}")
        if show_attrs:
            for name in getattr(node, "_attributes", ()):
                if hasattr(node, name):
                    value = getattr(node, name)
                    parts.append(f"{name}={dump_node(value, show_attrs, indent, level + 1)}")
        if indent <= 0 or not parts:
            inner = ", ".join(parts)
            return f"{type(node).__name__}({inner})"
        pad = " " * (indent * (level + 1))
        inner = ",\n".join(pad + part for part in parts)
        closing = " " * (indent * level)
        return f"{type(node).__name__}(\n{inner}\n{closing})"
    if isinstance(node, list):
        if not node:
            return "[]"
        if indent <= 0:
            inner = ", ".join(dump_node(item, show_attrs, indent, level + 1) for item in node)
            return f"[{inner}]"
        pad = " " * (indent * (level + 1))
        inner = ",\n".join(pad + dump_node(item, show_attrs, indent, level + 1) for item in node)
        closing = " " * (indent * level)
        return f"[\n{inner}\n{closing}]"
    return repr(node)


def to_dump_text(tree: ast.AST, show_attrs: bool) -> str:
    return dump_node(tree, show_attrs, indent=2, level=0)


def escape_dot_label(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', "\\\"")


def to_dot(tree: ast.AST, show_attrs: bool) -> str:
    lines = ["digraph AST {", "node [shape=box];"]
    counter = 0

    def add_node(node: ast.AST) -> int:
        nonlocal counter
        node_id = counter
        counter += 1
        label = escape_dot_label(node_summary(node, show_attrs))
        lines.append(f'n{node_id} [label="{label}"];')
        for child in ast.iter_child_nodes(node):
            child_id = add_node(child)
            lines.append(f"n{node_id} -> n{child_id};")
        return node_id

    add_node(tree)
    lines.append("}")
    return "\n".join(lines)


def write_output(text: str, output: str | None) -> None:
    if output:
        Path(output).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
        if not text.endswith("\n"):
            sys.stdout.write("\n")


def main() -> int:
    parser = argparse.ArgumentParser(description="AST view utility")
    parser.add_argument("--file", help="python source file")
    parser.add_argument("--code", help="inline python code")
    parser.add_argument(
        "--mode",
        default="exec",
        choices=["exec", "eval", "single"],
        help="ast.parse mode",
    )
    parser.add_argument(
        "--format",
        default="tree",
        choices=["tree", "dump", "dot"],
        help="output format",
    )
    parser.add_argument("--output", help="output file path")
    parser.add_argument("--max-depth", type=int, default=20)
    parser.add_argument("--attrs", action="store_true", help="include line/col info")
    args = parser.parse_args()

    source, source_name = read_source(args)
    tree = ast.parse(source, filename=source_name, mode=args.mode)

    if args.format == "dump":
        text = to_dump_text(tree, args.attrs)
    elif args.format == "dot":
        text = to_dot(tree, args.attrs)
    else:
        text = to_tree_text(tree, args.max_depth, args.attrs)

    write_output(text, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
