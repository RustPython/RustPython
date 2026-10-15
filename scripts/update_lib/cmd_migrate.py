#!/usr/bin/env python
"""
Migrate test file(s) from CPython, preserving RustPython markers.

Usage:
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py
    python scripts/update_lib migrate cpython/Lib/test/test_foo.py --patches remaining.json

Remaining metadata names the exact repository-relative target and its existing
decorator patches. An optional "doctests" object selects existing guarded inputs
by exact source text, mapping each to a single-line reason. Guarded doctests need
this explicit selection; an empty object retires all their guards. Canonical
expected output is never copied from the old file.

This will:
    1. Extract patches from Lib/test/test_foo.py (if exists)
    2. Apply them to cpython/Lib/test/test_foo.py
    3. Write result to Lib/test/test_foo.py
"""

import argparse
import ast
import collections
import json
import pathlib
import shutil
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent.parent))

from update_lib.file_utils import parse_lib_path

SYSCONFIG_UPSTREAM = "def _get_implementation():\n    return 'Python'\n"
SYSCONFIG_LOCAL = (
    "def _get_implementation():\n"
    "    return 'RustPython'  # XXX: RUSTPYTHON; For site-packages\n"
)
JIT_ERROR = """# NOTE: RUSTPYTHON exceptions
try:
    JitError
except NameError:
    pass
else:
    PYTHON2_EXCEPTIONS += ("JitError",)

"""
EXCEPTION_LOOP = """for excname in PYTHON2_EXCEPTIONS:
    NAME_MAPPING[("exceptions", excname)] = ("builtins", excname)
"""


def _unique_anchor(contents, root, predicate, *, direct=False, exact=None):
    """Select one real AST anchor, optionally in the immediate owner body."""
    matches = [node for node in ast.walk(root) if predicate(node)]
    if (
        len(matches) != 1
        or (direct and matches[0] not in root.body)
        or (exact is not None and ast.get_source_segment(contents, matches[0]) != exact)
    ):
        raise ValueError("Missing, changed, or ambiguous migration anchor")
    return matches[0]


def _unique_function(contents: str, expected: str) -> bool:
    tree = ast.parse(contents)
    target = ast.parse(expected).body[0]
    node = _unique_anchor(
        contents,
        tree,
        lambda node: (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == target.name
        ),
        direct=True,
    )
    return ast.dump(node) == ast.dump(target) and not any(
        (
            isinstance(node, ast.Name)
            and node.id == target.name
            and isinstance(node.ctx, (ast.Store, ast.Del))
        )
        or (isinstance(node, ast.ClassDef) and node.name == target.name)
        or (
            isinstance(node, ast.alias)
            and (node.asname or node.name.split(".")[0]) == target.name
        )
        for node in ast.walk(tree)
    )


def _exception_loop(contents: str) -> bool:
    _unique_anchor(
        contents,
        ast.parse(contents),
        lambda node: (
            isinstance(node, ast.For)
            and isinstance(node.iter, ast.Name)
            and node.iter.id == "PYTHON2_EXCEPTIONS"
        ),
        direct=True,
        exact=EXCEPTION_LOOP.rstrip(),
    )
    return True


def _jit_block(contents: str) -> bool:
    tree = ast.parse(contents)
    expected = ast.dump(ast.parse(JIT_ERROR).body[0])
    _unique_anchor(contents, tree, lambda node: ast.dump(node) == expected, direct=True)
    return (
        sum(
            isinstance(node, ast.Name) and node.id == "JitError"
            for node in ast.walk(tree)
        )
        == 1
    )


def preserve_migration_customizations(path: pathlib.Path, old: str, new: str) -> str:
    """Fail closed if either known local patch or its canonical anchor changes."""
    parts = path.parts
    if path.is_absolute():
        raise ValueError("Runtime patch target must be repository-relative")
    if parts == ("Lib", "sysconfig", "__init__.py"):
        if old.count(SYSCONFIG_LOCAL) != 1 or not _unique_function(
            old, SYSCONFIG_LOCAL
        ):
            raise ValueError("Unrecognized sysconfig implementation identity")
        if (
            new.count(SYSCONFIG_UPSTREAM) != 1
            or not _unique_function(new, SYSCONFIG_UPSTREAM)
            or "RustPython" in new
        ):
            raise ValueError("Unrecognized canonical sysconfig implementation anchor")
        return new.replace(SYSCONFIG_UPSTREAM, SYSCONFIG_LOCAL, 1)
    if parts == ("Lib", "_compat_pickle.py"):
        if (
            old.count(JIT_ERROR + EXCEPTION_LOOP) != 1
            or not _jit_block(old)
            or not _exception_loop(old)
        ):
            raise ValueError("Unrecognized local JitError compatibility mapping")
        if (
            new.count(EXCEPTION_LOOP) != 1
            or not _exception_loop(new)
            or "JitError" in new
        ):
            raise ValueError("Unrecognized canonical exception mapping anchor")
        return new.replace(EXCEPTION_LOOP, JIT_ERROR + EXCEPTION_LOOP, 1)
    if parts == ("Lib", "test", "test_cmd_line.py"):
        _verify_call(old, LOCAL_CALL)
        _verify_call(new, CANONICAL_CALL)
        return new.replace(CANONICAL_CALL, LOCAL_CALL, 1)
    return new


CANONICAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="")"""
LOCAL_CALL = """assert_python_ok('-c', code, PYTHONPATH="", RUSTPYTHONPATH="")"""


def _verify_call(contents: str, expected: str) -> None:
    tree = ast.parse(contents)
    owner = _unique_anchor(
        contents,
        tree,
        lambda node: isinstance(node, ast.ClassDef) and node.name == "CmdLineTest",
        direct=True,
    )
    method = _unique_anchor(
        contents,
        owner,
        lambda node: (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name == "test_empty_PYTHONPATH_issue16309"
        ),
        direct=True,
    )
    target = ast.dump(ast.parse(expected, mode="eval").body)
    _unique_anchor(
        contents,
        method,
        lambda node: isinstance(node, ast.Call) and ast.dump(node) == target,
        exact=expected,
    )
    if contents.count(expected) != 1:
        raise ValueError("Ambiguous environment-reset call text")


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

        patches = select_remaining_patches(patches, patches_document, relative_path)

    # Apply patches to source content
    src_content = src_path.read_text(encoding="utf-8")
    if lib_path.exists():
        src_content = preserve_migration_customizations(
            relative_path, old_content, src_content
        )
        guards = extract_doctest_guards(old_content)
        selection = patches_document.get("doctests") if patches_document else None
        if guards and selection is None:
            raise ValueError(
                "Existing doctest guards require explicit doctests metadata"
            )
        if selection is not None:
            src_content = apply_doctest_metadata(old_content, src_content, selection)
    return apply_patches(src_content, patches)


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
        "--patches",
        type=pathlib.Path,
        help="Single-file target, remaining decorator patches, and optional doctests JSON",
    )
    args = parser.parse_args(argv)

    try:
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
