"""Conservatively append an absent canonical method to an unchanged class."""

import ast
import builtins
import textwrap

from update_lib.cmd_migrate import _function_dependencies, _module_bindings


def _name(value):
    parts = value.split(".")
    if len(parts) != 2 or not all(part.isidentifier() for part in parts):
        raise ValueError("test and anchor names must be ClassName.method_name")
    return parts


def _headers(method):
    arguments = method.args.posonlyargs + method.args.args + method.args.kwonlyargs
    arguments += [arg for arg in (method.args.vararg, method.args.kwarg) if arg]
    return (
        method.decorator_list
        + method.args.defaults
        + [value for value in method.args.kw_defaults if value is not None]
        + [arg.annotation for arg in arguments if arg.annotation is not None]
        + ([method.returns] if method.returns is not None else [])
    )


def _class(tree, name):
    bindings = _module_bindings(tree)
    matches = bindings.get(name, [])
    if (
        len(matches) != 1
        or not isinstance(matches[0], ast.ClassDef)
        or matches[0] not in tree.body
    ):
        raise ValueError(f"Expected one direct module class binding: {name}")
    node = matches[0]
    if (
        node.col_offset
        or node.decorator_list
        or node.keywords
        or getattr(node, "type_params", ())
        or node.body[0].lineno == node.lineno
        or any(
            not isinstance(child, (ast.Name, ast.Attribute, ast.Load))
            for base in node.bases
            for child in ast.walk(base)
        )
    ):
        raise ValueError(
            "Class requires a plain, complete-line header and simple bases"
        )
    methods = {}
    for index, member in enumerate(node.body):
        if (
            index == 0
            and isinstance(member, ast.Expr)
            and isinstance(member.value, ast.Constant)
            and isinstance(member.value.value, str)
        ):
            continue
        if not isinstance(member, ast.FunctionDef) or member.name in methods:
            raise ValueError("Class body must contain unique, direct ordinary methods")
        methods[member.name] = member
    return node, methods, bindings


def _local_imports(method, allowed):
    detected = [
        node
        for node in ast.walk(method)
        if isinstance(node, (ast.Import, ast.ImportFrom))
    ]
    acknowledged = []
    for fragment in allowed:
        statements = ast.parse(fragment).body
        if len(statements) != 1 or not isinstance(
            statements[0], (ast.Import, ast.ImportFrom)
        ):
            raise ValueError("Each allowed local import must be one import statement")
        acknowledged.append(statements[0])
    for nodes in (detected, acknowledged):
        if any(
            isinstance(node, ast.ImportFrom)
            and (node.level or any(alias.name == "*" for alias in node.names))
            for node in nodes
        ):
            raise ValueError(
                "Relative and wildcard local imports require a full migration"
            )
        keys = [ast.dump(node) for node in nodes]
        if len(keys) != len(set(keys)):
            raise ValueError("Duplicate local import statements are ambiguous")
        names = [
            alias.asname or alias.name.split(".")[0]
            for node in nodes
            for alias in node.names
        ]
        if len(names) != len(set(names)):
            raise ValueError("Duplicate local import bindings are ambiguous")
    imported = {
        alias.asname or alias.name.split(".")[0]
        for node in detected
        for alias in node.names
    }
    rebound = set()
    for node in ast.walk(method):
        if isinstance(node, ast.arg):
            rebound.add(node.arg)
        elif isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            rebound.add(node.id)
        elif isinstance(
            node,
            (
                ast.FunctionDef,
                ast.AsyncFunctionDef,
                ast.ClassDef,
                ast.ExceptHandler,
                ast.MatchAs,
                ast.MatchStar,
            ),
        ):
            rebound.add(node.name)
        elif isinstance(node, ast.MatchMapping):
            rebound.add(node.rest)
        elif isinstance(node, (ast.Global, ast.Nonlocal)):
            rebound.update(node.names)
    if imported & rebound:
        raise ValueError("Local import bindings cannot shadow arguments or be rebound")
    if {ast.dump(node) for node in detected} != {
        ast.dump(node) for node in acknowledged
    }:
        raise ValueError("Local imports require an exact explicit allowlist")
    return [ast.unparse(node) for node in detected]


def add_test(source, destination, qualified_name, anchor_name, *, local_imports=()):
    """Append one absent tail method without executing either input.

    Existing class bytes must match exactly. Dependencies are restricted to
    unshadowed builtins and identical earlier imports, not arbitrary helpers.
    Local-import acknowledgment records provenance, not runtime authorization.
    """
    class_name, method_name = _name(qualified_name)
    anchor_class, anchor_method = _name(anchor_name)
    if anchor_class != class_name or method_name == anchor_method:
        raise ValueError("Test and distinct anchor must belong to the same class")
    # str.splitlines recognizes more separators than Python source line
    # numbering. Reject those rather than slicing a different definition.
    if any(
        character in text
        for text in (source, destination)
        for character in "\v\f\x1c\x1d\x1e\x85\u2028\u2029"
    ):
        raise ValueError("Nonstandard line separators require a full migration")
    src_tree, dst_tree = ast.parse(source), ast.parse(destination)
    src, src_methods, src_bindings = _class(src_tree, class_name)
    dst, dst_methods, dst_bindings = _class(dst_tree, class_name)
    if any(
        isinstance(node, ast.Name) and node.id == class_name
        for tree in (src_tree, dst_tree)
        for node in ast.walk(tree)
    ):
        raise ValueError("Class-name references or aliases require a full migration")
    if method_name in dst_methods:
        raise ValueError("Selected method already exists in the destination")
    if method_name not in src_methods or anchor_method not in src_methods:
        raise ValueError("Source test or anchor is missing")
    if anchor_method not in dst_methods:
        raise ValueError("Destination anchor is missing")
    method, anchor, dst_anchor = (
        src_methods[method_name],
        src_methods[anchor_method],
        dst_methods[anchor_method],
    )
    position = src.body.index(anchor)
    if (
        src.body[position + 1 : position + 2] != [method]
        or dst.body[-1] is not dst_anchor
    ):
        raise ValueError(
            "Test must follow the source anchor and destination must end there"
        )
    if (
        not method_name.startswith("test_")
        or method.decorator_list
        or getattr(method, "type_params", ())
        or method.returns is not None
        or ast.dump(method.args)
        != ast.dump(ast.parse("def f(self): pass").body[0].args)
    ):
        raise ValueError(
            "New method must be an undecorated, unannotated def test_*(self)"
        )
    src_lines, dst_lines = source.splitlines(True), destination.splitlines(True)
    if (
        src_lines[src.lineno - 1 : anchor.end_lineno]
        != dst_lines[dst.lineno - 1 : dst_anchor.end_lineno]
    ):
        raise ValueError("Complete class prefix through the anchor must match exactly")
    if method.col_offset != anchor.col_offset or method.lineno <= anchor.end_lineno:
        raise ValueError("Test and anchor must occupy separate equally indented lines")
    gap = src_lines[anchor.end_lineno : method.lineno - 1]
    indent = " " * method.col_offset
    if any(line.strip() and not line.startswith(indent + "#") for line in gap):
        raise ValueError("Ambiguous comments or statements between anchor and test")
    if not src_lines[method.end_lineno - 1].endswith("\n"):
        raise ValueError("Canonical method must end with a complete source line")
    _local_imports(method, local_imports)

    futures = [
        [
            node
            for node in tree.body
            if isinstance(node, ast.ImportFrom) and node.module == "__future__"
        ]
        for tree in (src_tree, dst_tree)
    ]
    if [ast.dump(node) for node in futures[0]] != [
        ast.dump(node) for node in futures[1]
    ]:
        raise ValueError("Source and destination future imports differ")
    prefix = "".join(ast.unparse(node) + "\n" for node in futures[0])
    dependencies = {
        node.id
        for base in src.bases
        for node in ast.walk(base)
        if isinstance(node, ast.Name)
    }
    for current in src.body[: position + 2]:
        if not isinstance(current, ast.FunctionDef):
            continue
        headers = _headers(current)
        if (
            getattr(current, "type_params", ())
            or any(
                isinstance(
                    node,
                    (
                        ast.Lambda,
                        ast.ListComp,
                        ast.SetComp,
                        ast.DictComp,
                        ast.GeneratorExp,
                    ),
                )
                for header in headers
                for node in ast.walk(header)
            )
            or any(isinstance(node, ast.ClassDef) for node in ast.walk(current))
        ):
            raise ValueError(
                "Nested classes, generics and scoped headers require a full migration"
            )
        first = min([current.lineno, *(item.lineno for item in current.decorator_list)])
        immediate, deferred = _function_dependencies(
            prefix + textwrap.dedent("".join(src_lines[first - 1 : current.end_lineno]))
        )
        dependencies.update(immediate | deferred)
        dependencies.update(
            node.id
            for header in headers
            for node in ast.walk(header)
            if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load)
        )
        for node in ast.walk(current):
            if (
                isinstance(node, ast.Attribute)
                and isinstance(node.value, ast.Name)
                and node.value.id == "self"
                and node.attr in src_methods
                and node.attr not in dst_methods
            ):
                raise ValueError("Methods cannot depend on new or later source methods")
    if dependencies & {"__import__", "getattr", "setattr", "delattr"}:
        raise ValueError("Dynamic namespace dependencies require a full migration")
    if "*" in src_bindings or "*" in dst_bindings:
        raise ValueError("Wildcard import makes dependency bindings ambiguous")
    for name in dependencies:
        declarations = []
        for tree, bindings, cls in (
            (src_tree, src_bindings, src),
            (dst_tree, dst_bindings, dst),
        ):
            bound = bindings.get(name, [])
            if not bound and name in vars(builtins):
                declarations.append(None)
                continue
            matches = [
                node
                for node in tree.body
                if isinstance(node, (ast.Import, ast.ImportFrom))
                and len(bound) == 1
                and bound[0] in node.names
                and node.lineno < cls.lineno
            ]
            if len(matches) != 1:
                raise ValueError(f"Expected one earlier import dependency: {name}")
            declarations.append(ast.dump(matches[0]))
        if declarations[0] != declarations[1]:
            raise ValueError(
                f"Source and destination import dependency differs: {name}"
            )
    for tree in (src_tree, dst_tree):
        for node in ast.walk(tree):
            if not isinstance(node, ast.Attribute) or not isinstance(
                node.ctx, (ast.Store, ast.Del)
            ):
                continue
            root = node.value
            while isinstance(root, ast.Attribute):
                root = root.value
            if isinstance(root, ast.Name) and root.id in dependencies:
                raise ValueError(
                    "Explicit dependency attribute mutation requires a full migration"
                )

    addition = "".join(src_lines[anchor.end_lineno : method.end_lineno])
    before, after = (
        "".join(dst_lines[: dst_anchor.end_lineno]),
        "".join(dst_lines[dst_anchor.end_lineno :]),
    )
    result = before + addition + after
    # Parsing verifies insertion boundaries; removing exactly the new span must
    # recover every original byte, including unrelated local markers/comments.
    result_class, result_methods, _ = _class(ast.parse(result), class_name)
    copied = result_methods[method_name]
    if (
        result.splitlines(True)[copied.lineno - 1 : copied.end_lineno]
        != src_lines[method.lineno - 1 : method.end_lineno]
        or result[: len(before)] + result[len(before) + len(addition) :] != destination
        or result_class.body[-1] is not copied
    ):
        raise ValueError(
            "Insertion did not preserve the exact canonical definition and surroundings"
        )
    return result
