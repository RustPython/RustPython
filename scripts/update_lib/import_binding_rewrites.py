"""Validate explicitly declared import-to-capability-binding adaptations.

This is a source-shape check, not execution or a general proof of import safety.
Only absolute, undotted imports and the canonical test.support helpers are
supported. Unrecognized binding forms fail closed for names used by a rewrite.
"""

import ast


def _dump(node):
    return ast.dump(node, include_attributes=False)


def _statement(text):
    body = ast.parse(text).body
    if len(body) != 1:
        raise ValueError("Rewrite declarations require one import statement")
    return body[0]


def _unique(body, expected):
    found = [i for i, node in enumerate(body) if _dump(node) == _dump(expected)]
    if len(found) != 1:
        raise ValueError("Missing or duplicate rewrite import/binding")
    return found[0]


def _binding_names(node):
    """Conservatively collect writes in module-executed syntax, not def bodies."""
    if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
        yield node.name
        # Decorators/defaults/bases execute now. A class suite has its own scope,
        # but global declarations can escape it; reject those conservatively.
        expressions = list(node.decorator_list)
        if isinstance(node, ast.ClassDef):
            if any(isinstance(n, (ast.Global, ast.Nonlocal)) for n in ast.walk(node)):
                raise ValueError("Unsupported global binding in class scope")
            expressions += node.bases + [k.value for k in node.keywords]
            # Class suites execute at import time. Conservatively reject
            # collisions even when a plain class-local name would be isolated.
            for statement in node.body:
                yield from _binding_names(statement)
        else:
            expressions += node.args.defaults + [
                n for n in node.args.kw_defaults if n is not None
            ]
        for expression in expressions:
            yield from _binding_names(expression)
        return
    if isinstance(node, (ast.Import, ast.ImportFrom)):
        for alias in node.names:
            if alias.name == "*":
                raise ValueError("Star imports cannot establish rewrite provenance")
            yield alias.asname or alias.name.split(".")[0]
        return
    if isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
        yield node.id
    if isinstance(node, (ast.ExceptHandler, ast.MatchAs, ast.MatchStar)) and node.name:
        yield node.name
    if isinstance(node, ast.MatchMapping) and node.rest:
        yield node.rest
    if isinstance(node, (ast.Global, ast.Nonlocal)):
        yield from node.names
    # Attribute writes can replace the helper itself or its module loader.
    if isinstance(node, ast.Attribute) and isinstance(node.ctx, (ast.Store, ast.Del)):
        value = node.value
        while isinstance(value, ast.Attribute):
            value = value.value
        if isinstance(value, ast.Name):
            yield value.id
    for child in ast.iter_child_nodes(node):
        yield from _binding_names(child)


def normalize_import_binding_rewrites(canonical, adapted, declarations):
    """Return trees with only fully validated rewrite groups made identical."""
    if not isinstance(declarations, list) or not declarations:
        raise ValueError("Import binding rewrites must be a nonempty list")
    original, changed = ast.parse(canonical), ast.parse(adapted)
    originals, replacements = {}, {}
    allowed_original, allowed_changed = {}, {}
    protected_names = set()
    helper_pairs = set()

    def allow(mapping, index, names):
        mapping.setdefault(index, set()).update(names)
        protected_names.update(names)

    for declaration in declarations:
        if not isinstance(declaration, dict):
            raise ValueError("Invalid import binding rewrite declaration")
        kind = declaration.get("kind")
        keys = {"canonical_import", "kind"}
        if kind == "required_attributes":
            keys.add("module_binding")
        if (
            kind not in {"optional_module", "required_attributes"}
            or set(declaration) != keys
        ):
            raise ValueError("Invalid import binding rewrite schema")
        text = declaration["canonical_import"]
        if not isinstance(text, str):
            raise ValueError("Expected canonical import text")
        source = _statement(text)
        if kind == "optional_module":
            if not isinstance(source, ast.Import) or len(source.names) != 1:
                raise ValueError("Optional module rewrite requires one module import")
            alias = source.names[0]
            module = alias.name
            targets = [alias.asname or module]
            expected = [
                _statement(f"{targets[0]} = import_helper.import_module({module!r})")
            ]
        else:
            if not isinstance(source, ast.ImportFrom) or source.level:
                raise ValueError("Attribute rewrite requires an absolute from import")
            module = source.module
            binding = declaration["module_binding"]
            if (
                not isinstance(binding, str)
                or not binding.isidentifier()
                or binding in {"support", "import_helper"}
            ):
                raise ValueError("Invalid module binding")
            targets = [alias.asname or alias.name for alias in source.names]
            if any(not alias.name.isidentifier() for alias in source.names):
                raise ValueError("Only named attributes are supported")
            expected = [
                _statement(
                    f"{target} = support.get_attribute({binding}, {alias.name!r})"
                )
                for target, alias in zip(targets, source.names)
            ]
        if not module or not module.isidentifier() or len(set(targets)) != len(targets):
            raise ValueError("Dotted, star or duplicate imports are unsupported")
        reserved = {"support", "import_helper"}
        if kind == "required_attributes":
            reserved.add(binding)
        if set(targets) & (protected_names | reserved):
            raise ValueError("Overlapping import binding rewrites")
        old_index = _unique(original.body, source)
        if old_index in originals or any(
            _dump(n) == _dump(source) for n in changed.body
        ):
            raise ValueError("Duplicate declaration or original import not removed")
        new_index = _unique(changed.body, expected[0])
        if [_dump(n) for n in changed.body[new_index : new_index + len(expected)]] != [
            _dump(n) for n in expected
        ]:
            raise ValueError("Rewrite assignments must match names, calls and order")
        if any(i in replacements for i in range(new_index, new_index + len(expected))):
            raise ValueError("Overlapping rewrite assignments")
        marker = ast.Expr(value=ast.Constant(value="import-binding:" + _dump(source)))
        originals[old_index] = marker
        for offset, target in enumerate(targets):
            replacements[new_index + offset] = marker if offset == 0 else None
            allow(allowed_changed, new_index + offset, [target])
        allow(allowed_original, old_index, targets)

        helpers = [("import_helper", "from test.support import import_helper")]
        if kind == "required_attributes":
            helpers.append(("support", "from test import support"))
            load = _statement(f"{binding} = import_helper.import_module({module!r})")
            old_load, new_load = (
                _unique(original.body, load),
                _unique(changed.body, load),
            )
            if old_load >= old_index or new_load >= new_index:
                raise ValueError("Module provenance must precede attribute access")
            allow(allowed_original, old_load, [binding])
            allow(allowed_changed, new_load, [binding])
        for helper, provenance in helpers:
            old_helper = _unique(original.body, _statement(provenance))
            new_helper = _unique(changed.body, _statement(provenance))
            if new_helper >= new_index:
                raise ValueError("Helper import must precede its use")
            if kind == "required_attributes" and helper == "import_helper":
                if old_helper >= old_load or new_helper >= new_load:
                    raise ValueError("Module loader helper must precede module binding")
            allow(allowed_original, old_helper, [helper])
            allow(allowed_changed, new_helper, [helper])
            helper_pairs.add((old_helper, new_helper))

    # Validate every module-scope binding, including compound statements. Never
    # descend into function bodies, whose locals do not establish provenance.
    for tree, permitted in [(original, allowed_original), (changed, allowed_changed)]:
        for index, node in enumerate(tree.body):
            touched = set(_binding_names(node)) & protected_names
            if touched != permitted.get(index, set()):
                raise ValueError("Rewrite target/helper/module is shadowed or rebound")

    def normalized(body, markers):
        return [
            markers.get(i, node)
            for i, node in enumerate(body)
            if markers.get(i, node) is not None
        ]

    # Helpers may move within an import region, not across protected execution.
    def region(body, index, markers):
        return sum(
            not isinstance(node, (ast.Import, ast.ImportFrom)) and i not in markers
            for i, node in enumerate(body[:index])
        )

    for old_helper, new_helper in helper_pairs:
        if region(original.body, old_helper, originals) != region(
            changed.body, new_helper, replacements
        ):
            raise ValueError("Helper import moved across protected statements")
    original.body = normalized(original.body, originals)
    changed.body = normalized(changed.body, replacements)
    return original, changed
