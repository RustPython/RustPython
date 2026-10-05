# RustPython parser backport

This source-only copy of `rustpython-ruff_python_parser` 0.16.5 carries the
upstream fixes for positive literal patterns and a local unpacking-diagnostic
patch for Python 3.15. The workspace
dependency uses a path and version so the backports also apply when RustPython
is a path or Git dependency of a separate workspace. Its AST dependencies are
unchanged.

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

The upstream patches change `src/error.rs`, `src/parser/mod.rs`, and
`src/parser/pattern.rs`. They retain the upstream inline examples; external
fixture and snapshot updates are not imported.

### Local RustPython diagnostic patch

`patches/rustpython-rc3-unpack-diagnostics.patch` is a local RustPython change,
not an upstream Ruff commit. Apply it after the two upstream patches, from the
RustPython root with `git apply --unidiff-zero PATCH_PATH`. It changes `src/error.rs` and
`src/parser/expression.rs`; the original archive checksum, source revision,
license, and `UPSTREAM-SHA256.json` remain unchanged.

The patch records structured parser errors and token/expression ranges for the
unpacking diagnostics in CPython v3.15.0rc3 (`8a8eb0b90d40f0e03f8252e9f73cad156e1caca8`),
principally `invalid_if_expression`, `invalid_comprehension`,
`invalid_kvpair_unpacking`, and `invalid_starred_expression_unpacking_sequence`
in `Grammar/python.gram`. It distinguishes calls from class bases and from
later arguments, uses the existing parse pass for recovery, and lets dictionary
unpacking comprehensions use the reference grammar's full expression operand.
RustPython's compiler preserves these structured diagnostics before its older
source-based fallback mappings. The unchanged `Lib/test/test_unpack_ex.py`
covers the motivating cases. An external differential corpus records complete
messages and ranges against the pinned reference; no upstream parser or
RustPython test assertion is changed.

To reproduce the source, verify the archive checksum, extract the paths above,
and apply each upstream patch from the RustPython root with
`git apply --unidiff-zero -p3 --directory=vendor/rustpython-ruff_python_parser PATCH_PATH`.
Remove the normalized manifest's `fixtures` and `generate_inline_tests` target
declarations, and copy the original license from the recorded source revision.

## Registry publication

Cargo replaces path dependencies with their version requirements when publishing
a package to a registry. Before publishing RustPython crates, replace this local
copy with a compatible published parser containing both upstream fixes and the
local diagnostic changes; the declared `0.16.5` registry version alone does not
contain them.

## Removing the backport

Once a compatible published RustPython Ruff parser includes the upstream fixes
and equivalent structured unpacking diagnostics,
replace this path dependency with that version, remove this directory and its
workspace exclusion, and regenerate the lockfile. RustPython no longer
suppresses the parser's unary-plus syntax diagnostic; all patterns pass
through the parser normally. The VM retains its existing policy for which
feature-version diagnostics match the CPython reference.
