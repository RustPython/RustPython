# xml 1.3.0 local accounting changes

This directory contains the official `xml` 1.3.0 crate from crates.io, with its
MIT license preserved in `LICENSE`. Upstream repository:
https://github.com/kornelski/xml-rs

- Upstream commit recorded by the package: `73c24eac0639a1e085269cf6c7b81b41cf1b9ad3`
- Published archive: https://static.crates.io/crates/xml/xml-1.3.0.crate
- Archive SHA-256: `636f85e5ca6488e96401b61eb7de54f4e44755c988af0f52cf90230c312a1a89`
- Included upstream files: `src/**`, `Cargo.toml`, `Cargo.toml.orig`, `README.md`,
  `LICENSE`, and `.cargo_vcs_info.json`
- Original-file hashes: `UPSTREAM-SHA256SUMS`
- Exact source/manifest changes: `patches/amplification.patch`, applicable with
  `patch -p1` from a fresh unpacking of the published archive

The root workspace dependency uses this directory with a version requirement,
so the accounting changes also apply when RustPython is a path or Git dependency
of a separate workspace. Its lockfile keeps version 1.3.0 without a registry
source/checksum. The local Cargo manifest has an empty workspace table so this
dependency can be checked separately, without building RustPython.

Cargo replaces path dependencies with their version requirements when publishing
a package to a registry. Before publishing RustPython crates, use a published
dependency that includes these accounting APIs and semantics; the declared `1.3`
registry requirement alone does not provide these local changes.

## Supported accounting

`ParserConfig::amplification_limits` installs immutable cumulative byte limits.
Direct bytes are measured at the decoder, including UTF-16 widths and the BOM.
Reparsed entity characters, parameter-entity substitutions, attribute entities,
and configured extra entities contribute indirect bytes. Numeric character
references do not contribute an additional replacement byte; the five named
predefined entities do, with the deferred checks used by Expat. Arithmetic
overflow is fatal even when the factor is infinity.

Attribute expansion and namespace installation wait for the complete opening
tag, then charge replacements before appending them. Attribute whitespace is
normalized with the distinction between physical newlines and character
references preserved. Existing depth and queue limits remain independent. Queue
boundaries now count actual nesting, rather than queued siblings, and the queue
cap is checked before reserving replacement storage.

RustPython enables a factor of 100 and activation threshold of 8,388,608 bytes
by default. Its two configuration methods take effect before the first Parse or
ParseFile call. Otherwise-valid later changes raise NotImplementedError,
including changes from handlers and after successful completion. Expat permits
live changes; supporting them requires callback synchronization and a replay
frontier for RustPython's synchronous fallback. External child parser Parse is
still explicitly unsupported, so shared accounting across external entities is
not claimed.

This is genuine cumulative expansion protection, not complete Expat tokenizer
parity. xml-rs may group character data and incomplete feeds differently, and
error positions remain xml-rs positions. In particular, exact timing at every
possible text-token/feed boundary is not promised. The fixed limits can reject
documents Expat accepts. No allocation-tracker API is supplied by these changes.

## Local checks

Run `cargo test --manifest-path vendor/xml/Cargo.toml --offline` from the repository
root. This runs the original crate tests and small accounting regressions, with
no network, external entities, or large generated documents.

For the current Rust toolchain, unchanged upstream code has three clippy lints:
`new_without_default`, `unbuffered_bytes`, and `enum_variant_names`. The local
changes pass with those existing lints permitted on the command line:

```
cargo clippy --manifest-path vendor/xml/Cargo.toml --offline --all-targets -- \
  -D warnings -A clippy::new_without_default -A clippy::unbuffered_bytes \
  -A clippy::enum_variant_names
```

Canonical Python tests are unchanged by this patch. The integration owner must
build RustPython and run the pinned pyexpat and XML consumer tests before removing
any stale expected-failure markers or claiming interpreter validation.
