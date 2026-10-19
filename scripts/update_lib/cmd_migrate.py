#!/usr/bin/env python
"""
Migrate Python Lib files from CPython, preserving explicit local adaptations.

Usage:
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py --patches remaining.json
    python scripts/update_lib migrate cpython/Lib/test/test_foo --canonical-removals removals.json --dry-run

Remaining metadata names the exact repository-relative target and its existing
decorator patches. An optional "doctests" object selects existing guarded inputs
by exact source text, mapping each to a single-line reason. Guarded doctests need
this explicit selection; an empty object retires all their guards. Canonical
expected output is never copied from the old file. Optional "blocks" lists
[canonical, local] source pairs to preserve exact existing changes.
Both fragments must match uniquely; no module-specific defaults are inferred.
Non-marker adaptations require reviewed blocks on each migration, for example
{"target": "Lib/example.py", "patches": {}, "blocks": [["old = 1", "old = 2"]]}.
Use a host Python that parses the source version (Python 3.15 for 3.15 syntax),
as required by the existing marker extraction and application steps.

Canonical removals are a separate, explicit operation. JSON source metadata
pins path (Lib/test/test_*), commit and tree; removals maps explicit relative
filenames to sha256 and upstream_commit. The caller must first verify and
review historical provenance, including release-branch evidence. This command
checks pinned canonical absence and exact local bytes; it does not authenticate
historical evidence. --dry-run performs all validation without deleting files.

This will:
    1. Extract patches from Lib/test/test_foo.py (if exists)
    2. Apply them to cpython/Lib/test/test_foo.py
    3. Write result to Lib/test/test_foo.py
"""

import argparse
import ast
import builtins
import collections
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib.file_utils import parse_lib_path


def _preserve_block(old, new, canonical, local):
    """Preserve an explicit existing fragment; do not infer module adaptations."""
    if old.count(local) != 1 or new.count(canonical) != 1:
        raise ValueError("Changed or ambiguous preservation block")
    return new.replace(canonical, local, 1)


def copy_selected_assignments(source, destination, names, exports):
    """Copy contiguous canonical assignments and export lines, failing closed.

    Only additions are supported. Dependencies must already be imported with the
    same bindings, or be builtins/earlier selected assignments. The next unchanged
    top-level statement is the insertion anchor; unrelated bytes stay untouched.
    """
    if (
        not names
        or len(set(names)) != len(names)
        or not all(name.isidentifier() for name in names)
    ):
        raise ValueError("Assignment names must be unique identifiers")
    if len(set(exports)) != len(exports) or not set(exports) <= set(names):
        raise ValueError("Exports must be unique selected assignment names")
    src, dst = ast.parse(source), ast.parse(destination)

    def assignment(tree, name):
        matches = [
            node
            for node in ast.walk(tree)
            if isinstance(node, ast.Name)
            and isinstance(node.ctx, ast.Store)
            and node.id == name
        ]
        nodes = [
            node
            for node in tree.body
            if isinstance(node, ast.Assign)
            and len(node.targets) == 1
            and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == name
        ]
        rebound = any(
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            and node.name == name
            or isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) == name
            for node in ast.walk(tree)
        )
        if len(nodes) != 1 or matches != [nodes[0].targets[0]] or rebound:
            raise ValueError(f"Expected one top-level assignment to {name}")
        return nodes[0]

    selected = [assignment(src, name) for name in names]
    start = src.body.index(selected[0])
    if src.body[start : start + len(names)] != selected or start + len(names) == len(
        src.body
    ):
        raise ValueError("Assignments must be contiguous and followed by an anchor")
    if (start and src.body[start - 1].end_lineno >= selected[0].lineno) or (
        selected[-1].end_lineno >= src.body[start + len(names)].lineno
    ):
        raise ValueError("Selected assignments must occupy complete source lines")
    for node in ast.walk(dst):
        if (
            isinstance(node, ast.Name)
            and isinstance(node.ctx, ast.Store)
            and node.id in names
            or isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))
            and node.name in names
            or isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) in names
        ):
            raise ValueError("A selected assignment already exists in the destination")

    anchor = src.body[start + len(names)]
    matches = [node for node in dst.body if ast.dump(node) == ast.dump(anchor)]
    if len(matches) != 1:
        raise ValueError("Expected one unchanged destination anchor")
    for tree, node in ((src, anchor), (dst, matches[0])):
        index = tree.body.index(node)
        if (
            getattr(node, "decorator_list", ())
            or node.col_offset
            or index
            and tree.body[index - 1].end_lineno >= node.lineno
            or index + 1 < len(tree.body)
            and tree.body[index + 1].lineno <= node.end_lineno
        ):
            raise ValueError(
                "Anchor must be undecorated and occupy complete source lines"
            )
    available = set(vars(builtins))
    imports = (set(), set())
    for node in src.body:
        matching = [
            other
            for other in dst.body
            if ast.dump(node) == ast.dump(other) and other.lineno < matches[0].lineno
        ]
        if (
            isinstance(node, (ast.Import, ast.ImportFrom))
            and node.lineno < selected[0].lineno
            and len(matching) == 1
        ):
            imports[0].update(node.names)
            imports[1].update(matching[0].names)
            available.update(
                alias.asname or alias.name.split(".")[0] for alias in node.names
            )
    dependencies = set()
    for name, node in zip(names, selected):
        used = {
            child.id
            for child in ast.walk(node.value)
            if isinstance(child, ast.Name) and isinstance(child.ctx, ast.Load)
        }
        missing = used - available
        if missing:
            raise ValueError(f"Missing dependencies for {name}: {sorted(missing)}")
        dependencies.update(used - set(names))
        available.add(name)
    for tree, boundary, allowed in (
        (src, selected[0].lineno, imports[0]),
        (dst, matches[0].lineno, imports[1]),
    ):
        if dependencies and any(
            isinstance(node, ast.ImportFrom)
            and node.lineno < boundary
            and any(alias.name == "*" for alias in node.names)
            for node in ast.walk(tree)
        ):
            raise ValueError("Wildcard import makes dependency bindings ambiguous")
        for dependency in dependencies:
            bindings = []
            for node in ast.walk(tree):
                if getattr(node, "lineno", boundary) >= boundary:
                    continue
                if isinstance(node, ast.Name) and isinstance(
                    node.ctx, (ast.Store, ast.Del)
                ):
                    name = node.id
                elif isinstance(node, ast.alias):
                    name = node.asname or node.name.split(".")[0]
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
                    name = node.name
                elif isinstance(node, ast.arg):
                    name = node.arg
                elif isinstance(node, ast.MatchMapping):
                    name = node.rest
                else:
                    continue
                if name == dependency:
                    bindings.append(node)
            if len(bindings) > 1 or any(node not in allowed for node in bindings):
                raise ValueError(f"Ambiguous dependency binding: {dependency}")

    source_lines, lines = source.splitlines(True), destination.splitlines(True)
    additions = [
        (
            matches[0].lineno - 1,
            source_lines[selected[0].lineno - 1 : anchor.lineno - 1],
        )
    ]
    if exports:
        src_all, dst_all = assignment(src, "__all__"), assignment(dst, "__all__")
        if not isinstance(src_all.value, ast.List) or not isinstance(
            dst_all.value, ast.List
        ):
            raise ValueError("__all__ must be a literal list")
        if any(
            not isinstance(node, ast.Constant) or not isinstance(node.value, str)
            for node in src_all.value.elts + dst_all.value.elts
        ):
            raise ValueError("__all__ entries must be literal strings")
        src_values, dst_values = (
            ast.literal_eval(src_all.value),
            ast.literal_eval(dst_all.value),
        )
        if any(src_values.count(name) != 1 or name in dst_values for name in exports):
            raise ValueError(
                "Exports must occur once in canonical __all__ and be absent locally"
            )
        export_nodes = [node for node in src_all.value.elts if node.value in exports]
        export_lines = sorted({node.lineno - 1 for node in export_nodes})
        fragment = "".join(source_lines[line] for line in export_lines)
        if ast.literal_eval("[\n" + fragment + "]") != exports:
            raise ValueError(
                "Canonical export lines include unselected entries or differ in order"
            )
        if lines[dst_all.lineno - 1].strip() != "__all__ = [":
            raise ValueError("Expected a multiline destination __all__ list")
        additions.append((dst_all.lineno, [fragment]))
    for offset, fragment in sorted(additions, reverse=True):
        lines[offset:offset] = fragment
    result = "".join(lines)
    ast.parse(result)
    return result


def migrate_selected_assignments(path, names, exports, source_hash, destination_hash):
    target = parse_lib_path(path)
    if target.parts[:1] != ("Lib",) or ".." in target.parts or target.suffix != ".py":
        raise ValueError("Selective assignments require an explicit Lib Python file")
    source, destination = path.read_bytes(), target.read_bytes()
    for label, contents, expected in (
        ("source", source, source_hash),
        ("destination", destination, destination_hash),
    ):
        if hashlib.sha256(contents).hexdigest() != expected:
            raise ValueError(f"{label} SHA256 does not match the pinned input")
    result = copy_selected_assignments(
        source.decode("utf-8"), destination.decode("utf-8"), names, exports
    ).encode("utf-8")
    if target.read_bytes() != destination:
        raise ValueError("Destination changed during assignment preparation")
    target.write_bytes(result)
    print(
        json.dumps(
            {
                "assignments": names,
                "exports": exports,
                "source_sha256": source_hash,
                "destination_before_sha256": destination_hash,
                "destination_after_sha256": hashlib.sha256(result).hexdigest(),
            },
            sort_keys=True,
        )
    )


def patch_single_content(
    src_path: pathlib.Path,
    lib_path: pathlib.Path,
    patches_document: dict | None = None,
) -> str:
    """
    Patch content without writing to disk.

    Args:
        src_path: Source file path (e.g., cpython/Lib/test/foo.py)
        lib_path: Lib path to extract patches from (e.g., Lib/test/foo.py)

    Returns:
        The patched content.
    """
    from update_lib import apply_patches, extract_patches
    from update_lib.patch_spec import apply_doctest_metadata, extract_doctest_guards

    # Extract patches from existing file (if exists)
    if lib_path.exists():
        old_content = lib_path.read_text(encoding="utf-8")
        patches = extract_patches(old_content)
    else:
        patches = {}

    repository = pathlib.Path(__file__).resolve().parents[2]
    try:
        relative_path = lib_path.resolve().relative_to(repository)
    except ValueError:
        # Explicit output paths outside this repository are not its Lib.
        relative_path = pathlib.Path("__external_output__")
    if patches_document is not None:
        from update_lib.patch_spec import select_remaining_patches

        patches = select_remaining_patches(
            patches,
            patches_document,
            relative_path,
            old_content=old_content if lib_path.exists() else None,
        )

    blocks = patches_document.get("blocks", []) if patches_document else []
    if not isinstance(blocks, list) or any(
        not isinstance(pair, list)
        or len(pair) != 2
        or any(not isinstance(text, str) or not text.strip() for text in pair)
        for pair in blocks
    ):
        raise ValueError("Blocks must be canonical/local source pairs")
    if blocks and not lib_path.exists():
        raise ValueError("Blocks require an existing local file")

    # Apply patches to source content
    src_content = src_path.read_text(encoding="utf-8")
    if lib_path.exists():
        if any(src_content.count(canonical) != 1 for canonical, _ in blocks):
            raise ValueError("Changed or ambiguous canonical preservation block")
        for canonical, local in blocks:
            src_content = _preserve_block(old_content, src_content, canonical, local)
        guards = extract_doctest_guards(old_content)
        selection = patches_document.get("doctests") if patches_document else None
        if guards and selection is None:
            raise ValueError(
                "Existing doctest guards require explicit doctests metadata"
            )
        if selection is not None:
            src_content = apply_doctest_metadata(old_content, src_content, selection)
    if not src_content and not patches:
        return src_content
    try:
        result = apply_patches(src_content, patches)
    except SyntaxError as error:
        if blocks:
            raise ValueError("Preserved blocks produced invalid Python") from error
        raise
    return result


def patch_file(
    src_path: pathlib.Path,
    lib_path: pathlib.Path | None = None,
    verbose: bool = True,
    patches_document: dict | None = None,
) -> None:
    """
    Patch a single file from source to lib.

    Args:
        src_path: Source file path (e.g., cpython/Lib/test/foo.py)
        lib_path: Target lib path. If None, derived from src_path.
        verbose: Print progress messages
    """
    if lib_path is None:
        lib_path = parse_lib_path(src_path)

    if lib_path.exists():
        if verbose:
            print(f"Patching: {src_path} -> {lib_path}")
        content = patch_single_content(src_path, lib_path, patches_document)
    else:
        if verbose:
            print(f"Copying: {src_path} -> {lib_path}")
        if patches_document is not None:
            raise ValueError("Marker override requires an existing target")
        content = src_path.read_text(encoding="utf-8")

    lib_path.parent.mkdir(parents=True, exist_ok=True)
    lib_path.write_text(content, encoding="utf-8")


def patch_directory(
    src_dir: pathlib.Path,
    lib_dir: pathlib.Path | None = None,
    verbose: bool = True,
) -> None:
    """
    Patch all files in a directory from source to lib.

    Args:
        src_dir: Source directory path (e.g., cpython/Lib/test/test_foo/)
        lib_dir: Target lib directory. If None, derived from src_dir.
        verbose: Print progress messages
    """
    if lib_dir is None:
        lib_dir = parse_lib_path(src_dir)

    src_files = sorted(
        f
        for f in src_dir.glob("**/*")
        if f.is_file() and "__pycache__" not in f.relative_to(src_dir).parts[:-1]
    )

    for src_file in src_files:
        rel_path = src_file.relative_to(src_dir)
        lib_file = lib_dir / rel_path

        if src_file.suffix == ".py":
            if lib_file.exists():
                if verbose:
                    print(f"Patching: {src_file} -> {lib_file}")
                content = patch_single_content(src_file, lib_file)
            else:
                if verbose:
                    print(f"Copying: {src_file} -> {lib_file}")
                content = src_file.read_text(encoding="utf-8")

            lib_file.parent.mkdir(parents=True, exist_ok=True)
            lib_file.write_text(content, encoding="utf-8")
        else:
            if verbose:
                print(f"Copying: {src_file} -> {lib_file}")
            lib_file.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src_file, lib_file)


def migrate_package(
    src_dir: pathlib.Path, lib_dir: pathlib.Path, mapping_path: pathlib.Path
) -> None:
    """Validate the complete migration before writing or removing legacy files.

    The JSON map names legacy files beside lib_dir, mapping every marked class
    to its canonical file relative to src_dir. Empty maps retire unmarked files.
    No test modules are imported or executed.
    """
    from update_lib.patch_spec import (
        _iter_class_methods,
        _iter_module_classes,
        apply_patches,
        build_patch_dict,
        extract_doctest_guards,
        iter_patches,
    )

    mapping = json.loads(mapping_path.read_text(encoding="utf-8"))
    if not isinstance(mapping, dict) or not mapping:
        raise ValueError("Legacy mapping must be a nonempty object")
    if lib_dir.exists():
        raise ValueError(f"Destination already exists: {lib_dir}")
    if (
        src_dir.resolve() == lib_dir.resolve()
        or src_dir.resolve() in lib_dir.resolve().parents
    ):
        raise ValueError("Source and destination overlap")
    source_files = sorted(
        p
        for p in src_dir.rglob("*")
        if p.is_file() and "__pycache__" not in p.relative_to(src_dir).parts[:-1]
    )
    if not (src_dir / "__init__.py").is_file():
        raise ValueError("Source must be a Python package")
    if any(
        p.is_symlink() or src_dir.resolve() not in p.resolve().parents
        for p in source_files
    ):
        raise ValueError("Source files must remain inside the canonical package")
    contents = {p.relative_to(src_dir): p.read_bytes() for p in source_files}
    entries = collections.defaultdict(list)
    owners = {}
    legacy_files = []
    for name, class_map in mapping.items():
        if pathlib.Path(name).name != name or not name.endswith(".py"):
            raise ValueError(f"Legacy file must be a sibling .py file: {name}")
        if not isinstance(class_map, dict):
            raise ValueError(f"Class map must be an object: {name}")
        legacy = lib_dir.parent / name
        if legacy.is_symlink() or legacy.resolve().parent != lib_dir.parent.resolve():
            raise ValueError(f"Legacy file escapes destination parent: {name}")
        if (
            legacy.resolve() == lib_dir.resolve()
            or src_dir.resolve() in legacy.resolve().parents
        ):
            raise ValueError(f"Legacy file overlaps canonical package: {name}")
        legacy_files.append(legacy)
        legacy_content = legacy.read_text(encoding="utf-8")
        if extract_doctest_guards(legacy_content):
            raise ValueError("Legacy package migration cannot select doctest guards")
        patches = list(iter_patches(legacy_content))
        legacy_classes = collections.defaultdict(list)
        for cls in _iter_module_classes(ast.parse(legacy_content)):
            legacy_classes[cls.name].append(cls)
        if set(class_map) != {p.parent_class for p in patches}:
            raise ValueError(f"Missing or unused class mapping: {name}")
        for entry in patches:
            source_classes = legacy_classes[entry.parent_class]
            if len(source_classes) != 1:
                raise ValueError(f"Ambiguous legacy class: {entry.parent_class}")
            if (
                entry.test_name != "__self__"
                and sum(
                    method.name == entry.test_name
                    for method in _iter_class_methods(source_classes[0])
                )
                != 1
            ):
                raise ValueError(
                    f"Ambiguous legacy method: {entry.parent_class}.{entry.test_name}"
                )
            target = pathlib.Path(class_map[entry.parent_class])
            if target.is_absolute() or ".." in target.parts or target not in contents:
                raise ValueError(f"Invalid canonical destination: {target}")
            if target.suffix != ".py":
                raise ValueError(f"Marker destination is not Python: {target}")
            tree = ast.parse(contents[target].decode("utf-8"))
            classes = collections.defaultdict(list)
            for cls in _iter_module_classes(tree):
                classes[cls.name].append(cls)

            def has_method(class_name, seen):
                if class_name in seen:
                    return False
                seen = seen | {class_name}
                nodes = classes.get(class_name, [])
                if len(nodes) != 1:
                    raise ValueError(f"Missing or ambiguous class: {class_name}")
                cls = nodes[0]
                methods = [
                    m for m in _iter_class_methods(cls) if m.name == entry.test_name
                ]
                if len(methods) > 1:
                    raise ValueError(
                        f"Ambiguous method: {class_name}.{entry.test_name}"
                    )
                if methods or entry.test_name == "__self__":
                    return True
                return any(
                    has_method(base.id, seen)
                    for base in cls.bases
                    if isinstance(base, ast.Name) and base.id in classes
                )

            if not has_method(entry.parent_class, set()):
                raise ValueError(
                    f"Missing method: {entry.parent_class}.{entry.test_name}"
                )
            owner_key = (target, entry.parent_class, entry.test_name)
            if owner_key in owners and owners[owner_key] != legacy:
                raise ValueError(
                    f"Duplicate mapped marker: {entry.parent_class}.{entry.test_name}"
                )
            owners[owner_key] = legacy
            entries[target].append(entry)
    # Render and validate everything before changing the destination.
    for target, patches in entries.items():
        contents[target] = apply_patches(
            contents[target].decode("utf-8"), build_patch_dict(iter(patches))
        ).encode("utf-8")
    for target, data in contents.items():
        if target.suffix == ".py":
            text = data.decode("utf-8")
            ast.parse(text)
            if collections.Counter(iter_patches(text)) != collections.Counter(
                entries[target]
            ):
                raise ValueError(f"Marker roundtrip mismatch: {target}")
    for target, data in contents.items():
        destination = lib_dir / target
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    for legacy in legacy_files:
        legacy.unlink()


def remove_canonical_files(src_dir, manifest_path, *, dry_run=False):
    """Remove only explicitly reviewed files, validating the whole batch first."""

    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"Duplicate removal metadata key: {key}")
            result[key] = value
        return result

    def relative(name):
        path = pathlib.PurePosixPath(name) if isinstance(name, str) else None
        if (
            not path
            or not path.parts
            or path.is_absolute()
            or ".." in path.parts
            or "\\" in name
            or path.as_posix() != name
            or "\0" in name
        ):
            raise ValueError("Expected an exact relative POSIX filename")
        return pathlib.Path(path)

    def no_symlinks(path):
        if any(p.is_symlink() or p.is_junction() for p in (path, *path.parents)):
            raise ValueError(f"Symlink or junction in removal path: {path}")

    def git(*args):
        result = subprocess.run(
            ["git", "-C", str(src_dir), *args],
            env={
                **os.environ,
                "GIT_NO_LAZY_FETCH": "1",
                "GIT_NO_REPLACE_OBJECTS": "1",
                "GIT_TERMINAL_PROMPT": "0",
            },
            capture_output=True,
            check=False,
        )
        if result.returncode:
            raise ValueError("Cannot verify pinned canonical source")
        return result.stdout.decode().strip()

    document = json.loads(
        manifest_path.read_text(encoding="utf-8"), object_pairs_hook=unique_object
    )
    if not isinstance(document, dict) or set(document) != {"source", "removals"}:
        raise ValueError("Expected source and removals metadata")
    source, removals = document["source"], document["removals"]
    if not isinstance(source, dict) or set(source) != {"path", "commit", "tree"}:
        raise ValueError("Expected source path, commit and tree")
    package = relative(source["path"])
    if (
        len(package.parts) != 3
        or package.parts[:2] != ("Lib", "test")
        or not package.name.startswith("test_")
        or not package.name.isidentifier()
    ):
        raise ValueError("Removals require an exact Lib/test/test_* package")
    if any(
        not isinstance(source[k], str) or not re.fullmatch(r"[0-9a-f]{40}", source[k])
        for k in ("commit", "tree")
    ):
        raise ValueError("Source commit and tree must be full Git hashes")
    if not isinstance(removals, dict) or not removals:
        raise ValueError("Removals must be a nonempty filename map")
    src_dir = src_dir.absolute()
    destination = pathlib.Path.cwd() / package
    for path in (src_dir, destination):
        no_symlinks(path)
        if not path.is_dir():
            raise ValueError("Source and destination packages must exist")
    if (
        src_dir == destination
        or src_dir in destination.parents
        or destination in src_dir.parents
    ):
        raise ValueError("Source and destination overlap")
    if (
        src_dir != pathlib.Path(git("rev-parse", "--show-toplevel")) / package
        or git("rev-parse", "HEAD") != source["commit"]
        or git("rev-parse", f"HEAD:{package.as_posix()}") != source["tree"]
    ):
        raise ValueError("Canonical source identity differs from the pinned input")
    canonical = set(
        git(
            "ls-tree",
            "--full-tree",
            "-r",
            "--name-only",
            "-z",
            "HEAD",
            "--",
            package.as_posix(),
        ).split("\0")
    )
    snapshots, receipt = {}, {"source": source, "dry_run": dry_run, "removals": {}}
    for name, entry in sorted(removals.items()):
        path = relative(name)
        if (
            not isinstance(entry, dict)
            or set(entry) != {"sha256", "upstream_commit"}
            or any(
                not isinstance(entry[k], str)
                or not re.fullmatch(f"[0-9a-f]{{{length}}}", entry[k])
                for k, length in (("sha256", 64), ("upstream_commit", 40))
            )
        ):
            raise ValueError(f"Missing or invalid hash/provenance pins: {name}")
        target = destination / path
        no_symlinks(target)
        no_symlinks(src_dir / path)
        if (package / path).as_posix() in canonical or (src_dir / path).exists():
            raise ValueError(f"Selected file is still canonical: {name}")
        if target.exists() and not target.is_file():
            raise ValueError(f"Removal target is not a regular file: {name}")
        data = target.read_bytes() if target.exists() else None
        if data is not None and (
            hashlib.sha256(data).hexdigest() != entry["sha256"]
            or b"rustpython" in data.lower()
        ):
            raise ValueError(f"Changed bytes or RustPython customization: {name}")
        snapshots[target] = data
        receipt["removals"][name] = {
            **entry,
            "status": "already_absent"
            if data is None
            else "would_remove"
            if dry_run
            else "removed",
        }
    if git("rev-parse", "HEAD") != source["commit"]:
        raise ValueError("Canonical source changed during validation")
    for target, data in snapshots.items():
        no_symlinks(target)
        canonical_path = src_dir / target.relative_to(destination)
        no_symlinks(canonical_path)
        if canonical_path.exists():
            raise ValueError("Canonical source changed during validation")
        if (target.exists() and not target.is_file()) or (
            target.read_bytes() if target.exists() else None
        ) != data:
            raise ValueError(f"Removal target changed during validation: {target}")
    if not dry_run:
        for target, data in snapshots.items():
            if data is not None:
                target.unlink()
    print(json.dumps(receipt, sort_keys=True))
    return receipt


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "path",
        type=pathlib.Path,
        help="Source path containing /Lib/ (file or directory)",
    )

    parser.add_argument(
        "--legacy-map",
        type=pathlib.Path,
        help="Explicit legacy-file/class to package-file JSON mapping",
    )
    parser.add_argument(
        "--canonical-removals",
        type=pathlib.Path,
        help="Remove only selected unchanged historical files absent from a pinned canonical tree",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Validate canonical removals without deleting",
    )

    parser.add_argument(
        "--patches",
        type=pathlib.Path,
        help="Single-file target, remaining decorator patches, and optional doctests JSON",
    )
    parser.add_argument(
        "--assignments",
        nargs="+",
        help="Copy selected canonical assignments",
    )
    parser.add_argument("--exports", nargs="+", default=[])
    parser.add_argument("--source-sha256")
    parser.add_argument("--destination-sha256")
    args = parser.parse_args(argv)

    try:
        if args.canonical_removals is not None:
            if (
                args.legacy_map is not None
                or args.patches is not None
                or args.assignments
                or args.exports
                or args.source_sha256
                or args.destination_sha256
            ):
                raise ValueError(
                    "Canonical removals cannot combine with other migration modes"
                )
            try:
                remove_canonical_files(
                    args.path, args.canonical_removals, dry_run=args.dry_run
                )
            except (OSError, UnicodeError) as exc:
                raise ValueError(str(exc)) from exc
            return 0
        if args.dry_run:
            raise ValueError("--dry-run requires --canonical-removals")
        if args.assignments:
            if args.legacy_map is not None or args.patches is not None:
                raise ValueError(
                    "Selective imports cannot combine with --legacy-map or --patches"
                )
            try:
                migrate_selected_assignments(
                    args.path,
                    args.assignments,
                    args.exports,
                    args.source_sha256,
                    args.destination_sha256,
                )
            except (SyntaxError, UnicodeError) as exc:
                raise ValueError(str(exc)) from exc
            return 0
        if args.exports or args.source_sha256 or args.destination_sha256:
            raise ValueError("Selective options require --assignments")
        if args.patches is not None and (
            args.path.is_dir() or args.legacy_map is not None
        ):
            raise ValueError("--patches requires a single file without --legacy-map")
        document = (
            json.loads(args.patches.read_text()) if args.patches is not None else None
        )
        if args.legacy_map is not None:
            migrate_package(args.path, parse_lib_path(args.path), args.legacy_map)
        elif args.path.is_dir():
            patch_directory(args.path)
        else:
            patch_file(args.path, patches_document=document)
        return 0
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1
    except FileNotFoundError as e:
        print(f"Error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
