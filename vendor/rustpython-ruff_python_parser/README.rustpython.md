# RustPython parser backport

This source-only copy of `rustpython-ruff_python_parser` 0.16.5 carries the
upstream fixes for positive literal patterns in Python 3.15. The workspace's
`[patch.crates-io]` selects this crate without changing its AST dependencies.

## Source and integrity

- Package: <https://crates.io/crates/rustpython-ruff_python_parser/0.16.5>
- Archive: <https://static.crates.io/crates/rustpython-ruff_python_parser/rustpython-ruff_python_parser-0.16.5.crate>
- Archive SHA-256: `480eec9ac4dda77b6724f0c9a54a81d3c57868d6a707c3d7c5133fd83c933f66`
- Source revision: <https://github.com/RustPython/ruff/commit/de398d0cb1ef35e69d91ebdb8c5ac4e900f61710>
- Source path: `crates/ruff_python_parser`
- License: `LICENSE`, copied from the source revision above; upstream Git blob
  `655a0c76fc5539b2f8e63b673741a5fd0e0c5799`

`UPSTREAM-SHA256.json` records the archive's original bytes for every included
file. The copy retains `src/`, `README.md`, both manifests, and the upstream
revision metadata. The `src/` tree includes its unit tests and snapshots.
The separate `tests/` and `resources/` trees are not included; their two
integration-test target declarations are consequently omitted from the
normalized `Cargo.toml`. No unit-test code or assertions were changed. The
vendor is excluded from the workspace member list, as registry dependencies
also are. This keeps the import to roughly 1.1 MB of file content (about 1.7 MB
on disk) instead of the full package's roughly 14 MB on disk.

## Backported source changes

The source hunks from these two upstream commits are retained in `patches/`
and applied in this order (context-free diff formatting preserves the exact
source changes):

1. <https://github.com/astral-sh/ruff/commit/eff1c88d7018293a52edf5e61d99a1a9061fc1e4>
   allows unary plus at every literal-pattern position and records its version
   requirement through `UnsupportedSyntaxErrorKind::UnaryPlusMatchPattern`
2. <https://github.com/astral-sh/ruff/commit/0ecff877901a0377ad966ffab528488ce872b50e>
   requires the operand after either sign to be a numeric literal

Only `src/error.rs`, `src/parser/mod.rs`, and `src/parser/pattern.rs` differ
from their original source bytes. The patches retain the upstream inline
examples; external fixture and snapshot updates are not imported.

To reproduce the source, verify the archive checksum, extract the paths above,
and apply each recorded patch from the RustPython root with
`git apply --unidiff-zero -p3 --directory=vendor/rustpython-ruff_python_parser PATCH_PATH`.
Remove the normalized manifest's `fixtures` and `generate_inline_tests` target
declarations, and copy the original license from the recorded source revision.

## Removing the backport

Once a compatible published RustPython Ruff parser includes both fixes,
replace this override with that version, remove this directory and its
workspace exclusion, and regenerate the lockfile. RustPython no longer
suppresses the parser's unary-plus syntax diagnostic; all patterns pass
through the parser normally. The VM retains its existing policy for which
feature-version diagnostics match the CPython reference.
