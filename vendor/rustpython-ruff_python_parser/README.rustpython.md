# RustPython CPython 3.15rc3 parser fixes

This source-only copy of `rustpython-ruff_python_parser` 0.16.10 carries exactly
six local RustPython patches for CPython 3.15.0rc3: structured unpacking
diagnostics, full-expression operands in starred generator expressions, parameter
wording, parser-owned statement diagnostics, and f/t-string error context and
precedence, and multiline concatenation hint ranges. The latter four use the optional
`python315-diagnostics` feature. The published
0.16.10 parser already includes the official unary-plus pattern backports;
those patches are not reapplied or retained here.

## Source and integrity

- Package: <https://crates.io/crates/rustpython-ruff_python_parser/0.16.10>
- Archive: <https://static.crates.io/crates/rustpython-ruff_python_parser/rustpython-ruff_python_parser-0.16.10.crate>
- Archive SHA-256: `7c9729e35cac02839c7a67aaccfea3eba32032f7aeacca2e82ad79e8d7b7fd95`
- Source revision: <https://github.com/RustPython/ruff/commit/8b59b8534415d79b28376a9808fc1b425d04b3ec>
- Official fork tag: `0.16.10-rustpython`
- Source path: `crates/ruff_python_parser`
- License: `LICENSE`, copied from that source revision; upstream Git blob
  `655a0c76fc5539b2f8e63b673741a5fd0e0c5799`

The archive checksum matches RustPython main's `Cargo.lock` at
`a40334179123949fbeef1cdc7557539c06cd187b`; the archive's `.cargo_vcs_info.json`
identifies the source revision above. `UPSTREAM-SHA256.json` records the original
archive bytes for every included upstream file. `LICENSE` is verified separately
because it is absent from the published crate archive.

The copy retains `src/`, `README.md`, both manifests and the upstream revision
metadata. All original unit-test source and snapshots are retained. The separate
`tests/` and `resources/` trees are omitted, as in the preceding source-only
vendor; their `fixtures` and `generate_inline_tests` target declarations are
omitted from the normalized `Cargo.toml`. All dependency versions and features
remain those in the published 0.16.10 manifest.

## Local source changes

Apply these patches in order, from the RustPython root, with `git apply`:

1. `patches/rustpython-rc3-unpack-diagnostics.patch` adds six structured
   `ParseErrorType` variants and precise token/expression ranges for the
   unpacking invalid productions in CPython 3.15.0rc3. It preserves the newer
   parser's missing-comma, dictionary-colon/value, and statement-after-else
   handling. It accommodates `InvalidAssignmentTarget { .. }` and existing
   0.16.10 AST constructors without changing the AST crate.
2. `patches/rustpython-rc3-starred-generator-operands.patch` allows the first
   parenthesized starred element to use full expression precedence when a
   generator clause follows. Tuple/group and sequence-display restrictions
   remain. Display diagnostics are deferred in encounter order, restored only
   after another parse error, and rolled back with parser checkpoints. The
   newer tokenizer-error prioritization remains in effect.
3. `patches/rustpython-rc3-parameter-diagnostics.patch` declares the default-off
   `python315-diagnostics` Cargo feature in both manifests. When enabled, it uses
   CPython 3.15 parameter terminology for exactly six diagnostic families:
   parameters following `**kwargs`, bare `*`, defaults for `*args` and `**kwargs`,
   repeated `*`, and a leading `/`. It preserves structured error variants,
   ranges, error ordering, recovery, tokens, and the AST. Grammar target version
   remains independent: CPython 3.15 uses the same diagnostic wording even when
   `ast.parse(feature_version=...)` requests an older grammar. With the feature
   disabled, the original upstream messages and fixtures remain unchanged.

4. `patches/rustpython-rc3-statement-diagnostics.patch` extends the same feature
   with structured standalone-case, raise/from, trailing-with-comma,
   mapping-rest, assert-assignment/named-expression, misplaced-lazy-import, and
   forbidden lazy-future-import diagnostics. Parser checkpoints validate the
   relevant grammar using existing productions, then retain the original
   recovered AST and token stream. Unparenthesized from-import trailing commas
   are diagnosed at their terminating newline. The lazy-future grammar action
   runs after targets complete and before later statements; a missing optional
   alias after `as` can roll back to the valid unparenthesized name prefix.
   Missing names, parentheses, and trailing-comma errors keep precedence.
   Missing import aliases after `as` use the generic syntax diagnostic at the
   existing token range, preserving relative-lazy warning collection.

5. `patches/rustpython-rc3-fstring-diagnostics.patch` preserves multiline
   replacement-field opening-line diagnostics and their priority over expression
   hints within the same field. It uses the existing lexer bracket/interpolation
   state and token ordering, including nested format fields and continuation
   boundaries. Grammar acceptance, recovered ASTs and tokens are unchanged.
   Behavior is gated by `python315-diagnostics`; its structured
   `UnclosedLbraceOnLine` error variant is present unconditionally.

6. `patches/rustpython-rc3-concat-comma-diagnostics.patch` places a missing-comma
   hint for a multiline concatenated literal on its final line, retaining the
   original starting column. The structured error retains the expression span
   for error precedence and a separate diagnostic start for empty or reversed
   CPython ranges. RustPython's compiler maps those offsets without scanning
   source or rewriting the message. Behavior is gated by the same feature.

All six are local RustPython patches, not upstream Ruff commits. The first two
port the local parser patches previously carried on RustPython `70e069972`.
The third through sixth preserve diagnostics formerly handled by that revision's
compiler and VM normalization, without restoring source scanners or VM string
rewriting. All use CPython
`v3.15.0rc3` (`8a8eb0b90d40f0e03f8252e9f73cad156e1caca8`) as the reference.
No canonical assertions, upstream snapshots, or skip/xfail markers are changed.

Reproduce the vendor by verifying and extracting the archive, copying its
retained paths, removing the two omitted test target declarations, copying the
license from the exact source revision, and applying the six patches above.
The provenance manifest always records unmodified archive bytes, including the
original normalized manifest before removing the test targets.

## Workspace use and publication

Keep the parser outside the workspace member list and use a workspace dependency
with `path = "vendor/rustpython-ruff_python_parser"`, `version = "0.16.10"`,
and `features = ["python315-diagnostics"]`.
The path also applies when RustPython is another workspace's Git/path dependency.
A root `[patch.crates-io]` override alone does not have that property.

Cargo replaces path dependencies with their version requirements when publishing
to a registry. Before publishing RustPython crates, switch to a published parser
containing all six fixes; the unmodified 0.16.10 registry crate lacks them
and does not declare the local diagnostic feature. The published dependency
must retain that feature or provide equivalent wording unconditionally and
remove the now-unnecessary feature selection.

When a compatible published parser contains equivalent fixes, remove this vendor
and its workspace exclusion, switch to that registry version, and regenerate the
lockfile. Keep the newer AST interfaces and diagnostic plumbing.

## Diagnostic feature compatibility

`python315-diagnostics` is a build-time choice for the fixed-3.15 RustPython
runtime, not a language-version flag. Cargo feature unification applies to all
consumers of this package in the same dependency graph. Consumers that require
legacy diagnostics must use a graph where the feature is disabled. No new
parser entrypoint or option is added. The fourth patch adds meaningful
structured error variants and a standalone-case block-clause distinction.
The fifth adds an interpolation-error variant carrying the opening line;
the sixth adds a concatenation-hint variant with separate display offsets.
Downstream exhaustive Rust matches must account for these enum additions;
feature-off compatibility describes runtime output, not enum shape.

The immutable upstream snapshots retain their original diagnostics. Run their
default configuration with this feature off. Feature on intentionally changes
the preserved rc3 messages/ranges and rejects forbidden lazy future imports at
parse time, where CPython's grammar rejects them. The runtime already rejected
these imports; AST and tokens stay unchanged. No upstream snapshot or canonical
assertion was edited. Exact CPython rc3 records are the feature-on reference.

Native exception conversion still owns concatenation display-offset mapping,
newline-end normalization, zero-width
empty import names, standalone-case IndentationError end_offset=-1, and
unclosed-bracket end_offset=0. Barry preprocessing and single-input boundary
precedence remain caller responsibilities. Callers must honor structured
LazyFutureImport before later errors, but a preceding single-input boundary
still takes precedence.
