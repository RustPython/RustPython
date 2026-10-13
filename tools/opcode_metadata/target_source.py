"""Fail closed before importing any target CPython generator code."""

import hashlib
import json
import sys
from pathlib import Path


def verify_source(root):
    caches = list((root / "Tools/cases_generator").rglob("*.pyc"))
    if caches:
        raise ValueError(
            "target generator source must be cache-free; found " + str(caches[0])
        )
    manifest = json.loads(Path(__file__).with_suffix(".json").read_text())
    for relative, expected in manifest["files"].items():
        path = root / relative
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError(f"wrong {manifest['tag']} source: {relative}: {actual}")
    return manifest


def verify_import_origins(root):
    manifest = json.loads(Path(__file__).with_suffix(".json").read_text())
    for relative in manifest["files"]:
        path = Path(relative)
        if path.parent != Path("Tools/cases_generator") or path.suffix != ".py":
            continue
        module = sys.modules.get(path.stem)
        if module is not None:
            origin = getattr(module, "__file__", None)
            if origin is None or Path(origin).resolve() != (root / path).resolve():
                raise ValueError(
                    "generator dependency loaded outside pinned source: " + path.stem
                )
