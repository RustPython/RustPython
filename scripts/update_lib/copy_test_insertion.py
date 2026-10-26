"""Insert an absent canonical method before an unchanged class anchor."""

import ast

from update_lib.cmd_copy_test import _definition


def _class_scope_nodes(cls):
    """Visit class bindings, not bindings inside nested lexical scopes.

    Definition names and header expressions still belong to the surrounding
    scope. Inspect annotations conservatively too, even when evaluation is
    deferred; do not inspect nested bodies or type-parameter scopes.
    """

    def visit(node):
        yield node
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            children = [node.args, *node.decorator_list]
            if node.returns is not None:
                children.append(node.returns)
        elif isinstance(node, ast.Lambda):
            children = [node.args]
        elif isinstance(node, ast.ClassDef):
            children = [*node.bases, *node.keywords, *node.decorator_list]
        elif isinstance(
            node, (ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)
        ):
            # Only the outermost iterable is evaluated outside the implicit
            # comprehension scope. Its iteration targets are not class bindings.
            children = [node.generators[0].iter]
        else:
            children = ast.iter_child_nodes(node)
        for child in children:
            yield from visit(child)

    for node in cls.body:
        yield from visit(node)


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
        for node in _class_scope_nodes(cls)
    ):
        raise ValueError("Insertion does not support scope declarations in classes")
    # Nodes come from separate parses; compare their unique source locations.
    members = {node.lineno: index for index, node in enumerate(src_class.body)}
    if members.get(anchor.lineno) != members.get(method.lineno, -2) + 1:
        raise ValueError("Source test must immediately precede its direct anchor")
    if not any(node.lineno == dst_anchor.lineno for node in dst_class.body):
        raise ValueError("Destination anchor must be a direct class member")
    for node in _class_scope_nodes(dst_class):
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
