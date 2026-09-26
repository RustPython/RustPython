#!/usr/bin/env python3
"""Regenerate crates/doc/used_keys.txt.

Builds the crates that expand the doc macros and records every key they
resolve. Also keeps every database key whose module or class is declared in
the tree, including declarations that this host does not compile.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DB_PATH = ROOT / "crates/doc/src/data.inc.rs"
OUT_PATH = ROOT / "crates/doc/used_keys.txt"
ENV = "RUSTPYTHON_DOC_KEYS_OUT"

ITEM_RE = re.compile(
    r"(?:struct|enum|fn|mod|impl|union|type)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
NAME_RE = re.compile(r'\bname\s*=\s*"([^"]*)"')
MODULE_RE = re.compile(r"\bmodule\s*=\s*(false|\"([^\"]*)\")")


def parse_db_keys() -> list[str]:
    keys = []
    for line in DB_PATH.read_text().splitlines():
        line = line.strip()
        if not line.startswith('("'):
            continue
        end = line.find('",')
        if end < 0:
            continue
        keys.append(line[2:end])
    return keys


def attr_bodies(text: str):
    i = 0
    while True:
        start = text.find("#[", i)
        if start < 0:
            return
        depth = 0
        j = start + 1
        while j < len(text):
            ch = text[j]
            if ch == "[":
                depth += 1
            elif ch == "]":
                depth -= 1
                if depth == 0:
                    yield text[start : j + 1], j + 1
                    i = j + 1
                    break
            j += 1
        else:
            return


def kind_of(attr: str) -> str | None:
    head = attr[2:].lstrip()
    for kind in ("pystruct_sequence", "pymodule", "pyclass"):
        if head.startswith(kind):
            return kind
    return None


def item_ident(text: str, end: int) -> str | None:
    window = text[end : end + 500]
    match = ITEM_RE.search(window)
    return match.group(1) if match else None


def module_aliases(module: str) -> set[str]:
    names = {module}
    if module in {"os", "_os", "posix", "nt"}:
        names.update({"posix", "nt", "os", "_os"})
    if module.startswith("_") and len(module) > 1:
        names.add(module[1:])
    elif module and not module.startswith("_"):
        names.add("_" + module)
    return names


def declared_owners(keys: set[str]) -> tuple[set[tuple[str, str]], set[str]]:
    classes: set[tuple[str, str]] = set()
    modules: set[str] = set()
    for path in (ROOT / "crates").rglob("*.rs"):
        if "target" in path.parts:
            continue
        text = path.read_text(errors="replace")
        current = "builtins"
        for attr, end in attr_bodies(text):
            kind = kind_of(attr)
            if kind is None:
                continue
            ident = item_ident(text, end) or ""
            name_match = NAME_RE.search(attr)
            name = name_match.group(1) if name_match else ident
            module_match = MODULE_RE.search(attr)
            if kind == "pymodule":
                module = name_match.group(1) if name_match else ident
                if module:
                    modules.add(module)
                    current = module
                continue
            if module_match is None:
                module = current
            elif module_match.group(1) == "false":
                module = "builtins"
            else:
                module = module_match.group(2) or current
            if name:
                classes.add((module, name))
    return classes, modules


def keys_for_owners(all_keys: list[str], classes, modules) -> set[str]:
    wanted = set()
    class_prefixes = set()
    for module, name in classes:
        for alias in module_aliases(module):
            class_prefixes.add(f"{alias}.{name}")
    module_names = set()
    for module in modules:
        module_names.update(module_aliases(module))
    # Type names passed as string literals to registration macros, such as
    # "dict_keys", rather than as a #[pyclass] name.
    literals = set()
    for path in (ROOT / "crates").rglob("*.rs"):
        if "target" in path.parts:
            continue
        for lit in re.findall(
            r'"([A-Za-z_][A-Za-z0-9_]*)"', path.read_text(errors="replace")
        ):
            if "_" in lit:
                literals.add(lit)
    for key in all_keys:
        parts = key.split(".")
        if len(parts) >= 2 and parts[1] in literals and parts[0] in module_names:
            class_prefixes.add(f"{parts[0]}.{parts[1]}")
    for key in all_keys:
        head, _, rest = key.partition(".")
        if not rest:
            if head in module_names or head in {module for module, _ in classes}:
                wanted.add(key)
            continue
        owner = key.rsplit(".", 1)[0] if key.count(".") >= 2 else head
        # module.class.attr -> module.class; module.func -> module
        if key.count(".") >= 2:
            owner = key.rsplit(".", 1)[0]
            if owner in class_prefixes:
                wanted.add(key)
        elif head in module_names:
            wanted.add(key)
    for prefix in class_prefixes:
        # The class object itself, when the database has that key.
        if prefix in set(all_keys):
            wanted.add(prefix)
    return wanted


def run_build(args: list[str], key_file: Path) -> None:
    env = os.environ.copy()
    env[ENV] = str(key_file)
    print("running", " ".join(args), flush=True)
    result = subprocess.run(args, cwd=ROOT, env=env)
    if result.returncode != 0:
        print(
            f"warning: {' '.join(args)} failed; scanned declarations still apply",
            file=sys.stderr,
        )


def main() -> None:
    all_keys = parse_db_keys()
    known = set(all_keys)
    classes, modules = declared_owners(known)
    scanned = keys_for_owners(all_keys, classes, modules)
    print(
        f"scanned owners: {len(classes)} classes, {len(modules)} modules, {len(scanned)} keys"
    )

    with tempfile.NamedTemporaryFile("w+", delete=False) as tmp:
        raw_path = Path(tmp.name)
    # Changing the proc-macro source forces dependents to expand again so the
    # env var is observed even when their inputs are otherwise fresh.
    (ROOT / "crates/derive-impl/src/doc_use.rs").touch()
    try:
        builds = [
            [
                "cargo",
                "check",
                "-p",
                "rustpython",
                "--offline",
                "--all-targets",
                "--features",
                "threading,stdlib,stdio,importlib,ssl-rustls-aws-lc,host_env,mimalloc,sqlite,doc",
            ],
            [
                "cargo",
                "check",
                "-p",
                "rustpython-stdlib",
                "--offline",
                "--all-targets",
                "--features",
                "compiler,host_env,doc,threading,sqlite,ssl-rustls",
            ],
            [
                "cargo",
                "check",
                "-p",
                "rustpython-vm",
                "--offline",
                "--all-targets",
                "--features",
                "doc,compiler,encodings,wasmbind,host_env,stdio,gc",
            ],
            [
                "cargo",
                "check",
                "-p",
                "rustpython_wasm",
                "--offline",
                "--target",
                "wasm32-unknown-unknown",
            ],
            [
                "cargo",
                "check",
                "--offline",
                "--manifest-path",
                "crates/capi/Cargo.toml",
            ],
        ]
        for args in builds:
            run_build(args, raw_path)
        resolved = set()
        if raw_path.exists():
            for line in raw_path.read_text(errors="replace").splitlines():
                line = line.strip()
                if line:
                    resolved.add(line)
    finally:
        raw_path.unlink(missing_ok=True)

    print(f"resolved on this host: {len(resolved)}")
    unknown = sorted(key for key in resolved if key not in known)
    if unknown:
        print(
            f"warning: {len(unknown)} resolved keys are not in the database",
            file=sys.stderr,
        )
        for key in unknown[:20]:
            print(f"  {key}", file=sys.stderr)
    merged = sorted(key for key in (resolved | scanned) if key in known)
    missing_scan = sorted(scanned - resolved)
    if missing_scan:
        print(
            f"keys added from source declarations (not resolved on this host): {len(missing_scan)}"
        )
    OUT_PATH.write_text(
        "# Regenerated by scripts/update_doc_keys.py\n" + "\n".join(merged) + "\n"
    )
    print(f"wrote {len(merged)} keys to {OUT_PATH.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
