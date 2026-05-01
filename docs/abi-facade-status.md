# ABI Facade Status

## Verified unchanged downstream packages

- `rpds`: package-owned Python tests green
- `jiter`: package-owned Python tests green, including multithreaded parsing
- `blake3`: package-owned Python tests green except `test_strided_array_fails` (blocked by missing usable `numpy`)

## Verification policy

Downstream verification helpers are local-only and are not part of the RustPython PR.

## Ranked ABI backlog

### Tier 1: broad pristine PyO3 compatibility
- `PyType_FromSpec` slot coverage in `crates/capi/src/object.rs`
- `PyType_GetSlot` coverage in `crates/capi/src/object.rs`
- richer buffer support in `crates/capi/src/pybuffer.rs`

### Tier 2: common ecosystem compatibility
- unicode/encoding APIs in `crates/capi/src/unicodeobject.rs`
- lifecycle/finalization semantics in `crates/capi/src/pylifecycle.rs`

### Tier 3: architectural follow-up
- exported builtin/type handle model in `crates/capi/src/handles.rs`
