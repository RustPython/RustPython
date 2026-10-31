# RustPython parser source reconciliation

This is a preservation-first forward-port for RustPython's Python 3.15 integration. It starts from the complete `crates/ruff_python_parser` package at RustPython/ruff revision `9290a8849542134d83f77e9360e7f0b93897fd93`, package version 0.17.1. It does not restore the old 0.16.10 dependency graph.

## Reproduction and identities

`UPSTREAM-SHA 256.json` records all 1,439 original package files. `Cargo.toml.orig` is the exact upstream package manifest. `LICENSE` comes from the same fork revision. The standalone manifest resolves upstream workspace inheritance explicitly and pins all Ruff internal dependencies to that exact fork revision, preserving package names, versions, features and AST/trivia/text-size identities.

The original fixture target and all 1,415 upstream tests/resources/snapshots are retained byte-for-byte. No upstream test expectation was changed. The source-only rebase has not run these tests.

Apply the ten patches listed in `patches/series` in order with `git apply -p 1` from the pristine package root. The resulting source and Cargo.toml have been reproduced byte-for-byte. `RUSTPYTHON-PROVENANCE.json` records the baseline and changes. Source reproduction is not compilation or behavioral validation.

## Preserved local behavior

The first six patches forward-port the original six diagnostic patches from RustPython mega commit `4a79418f9ab572e9b130b4b0aa9c4b25dc12ff95`: unpacking diagnostics, starred-generator expression operands, parameter terminology, statement diagnostics, f/t-string context and precedence, and concatenated-literal diagnostic offsets. The originals were based on published parser 0.16.10 at fork revision `8b59b8534415d79b28376a9808fc1b425d04b3ec`; they are not claimed to apply unchanged to 0.17.1.

Statement adaptation preserves newer upstream raise/from conditions and spans, trailing-comma locations, zero-width empty-import diagnostics, missing-alias and invalid-with classifications. Only the needed structured variants and remaining local behavior are added. Patch 7 adapts the manifest without downgrading internal source identities.

Patch 8 isolates recovery state around the added assert-assignment and misplaced-lazy-import probes, using the same three-field save/restore as the standalone-case probe. It does not change the shared checkpoint API.

Patch 9 preserves direct unpack errors and the first eligible deferred starred-generator error before standalone-case rewind. It retains encounter order, not source-offset sorting, and leaves earlier caller-owned deferred entries untouched. This correction is supported by source review and an eight-input compile-only observation against pinned CPython 3.15 final source `d220409a13b77de7a6c30abe16ea90db01528edc`. No Ruff or RustPython parser was executed for that observation.

Patch 10 adds parser-owned relative-lazy grammar-action and lexer-continuation provenance, without downgrading the pinned AST/trivia types. It records completed-import eligibility, actual lexer progress and hard stops, regular TYPE_COMMENT owners and the actual parameter-comma boundary. Compiler-side emission preserves earlier warning escalation and tokenizer-only recovery; true prompt remains a separate no-recovery path.

## Bounded validation and remaining gates

The isolated warning composition on main 5dcef1aa6d53c72315a44296146c768120841953 plus published warning basisf1ef9088f4f4165cac3528f935f46e6d4cb06135 passed parser/VM checks, strict VM Clippy and a default CLI build. All 1,056 fixed compile-only observations matched CPython warning lists/order/messages and exception selection;832 published controls remained byte-identical. No compiled payload was executed. Raw equality is 430, with 533 existing argument-representation differences and 93 existing location/text differences, recorded without normalization. This does not validate the complete rebased mega, true prompt, upstream parser suites, feature-off behavior or arbitrary malformed input.

The exact ten-patch source replay and all 1,415 upstream resource/test/snapshot files remain preserved. Full-mega dependency resolution, normal hooks, appropriate feature-on/off compilation and focused approved checks remain required. Private same-source/mode constructors retain all provenance checks. An internal invariant failure reports SystemError rather than suppressing a warning, falling back, or manufacturing a syntax diagnostic. Any supported-input hit blocks acceptance; finite zero-hit observations are not a universal parity claim. Held parser/fuzz/ecosystem and lifecycle probes remain outside this validation.
