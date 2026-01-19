"""
Library for updating Python test files with RustPython-specific patches.
"""

from .patch_spec import (
    COMMENT,
    DEFAULT_INDENT,
    UT,
    PatchEntry,
    Patches,
    PatchSpec,
    UtMethod,
    apply_patches,
    build_patch_dict,
    extract_patches,
    iter_patches,
    iter_tests,
    patches_from_json,
    patches_to_json,
)

__all__ = [
    "COMMENT",
    "DEFAULT_INDENT",
    "UT",
    "Patches",
    "PatchEntry",
    "PatchSpec",
    "UtMethod",
    "apply_patches",
    "build_patch_dict",
    "extract_patches",
    "iter_patches",
    "iter_tests",
    "patches_from_json",
    "patches_to_json",
]
