---
name: rustpython-capi-expansion
description: Implement missing RustPython C-API functions in crates/capi using the pyo3-ffi header split mapping (`pyo3-ffi/src/*.rs`, mirroring CPython C API headers). Use this whenever the user asks to add or port C-API functions (for example from setobject.h, dictobject.h, unicodeobject.h) or add capi tests.
---

# RustPython C-API Expansion

Use this workflow for adding missing C-API functions to RustPython.

## Source of truth for target files

- Use this mapping source: `pyo3-ffi/src/*.rs`, which mirrors the CPython header split used by the C API.
- Map requested header APIs to `crates/capi/src/<header_basename>.rs` using that split. Examples:
  - `setobject.h` -> `crates/capi/src/setobject.rs`
  - `dictobject.h` -> `crates/capi/src/dictobject.rs`
  - `unicodeobject.h` -> `crates/capi/src/unicodeobject.rs`
- Do not invent alternate target modules when the header split implies a direct target.
- If the target file is not present yet, create it and wire it in `crates/capi/src/lib.rs`.

## Implementation workflow

1. Identify requested missing APIs from the user request and their originating C API header.
2. Open nearby capi modules in `crates/capi/src/` and follow existing style and patterns.
3. Implement only the requested functions in the mapped target file.
4. Keep behavior aligned with CPython C-API contracts.
5. Prefer using existing `rustpython-vm` functionality as much as possible instead of re-implementing behavior in capi.
6. If a needed `rustpython-vm` helper exists but is private, make it public with a minimal, focused visibility change.
7. Prefer direct contract assumptions over defensive null checks unless required by the established local style.
8. Add basic tests only; do not overfit with very specific edge-case clutter.
9. In tests, use `pyo3` only as a safe wrapper over the API. Avoid raw pointer-heavy direct FFI-style tests.
10. Run tests from `crates/capi`.

## Testing rules

- Run test commands with working directory set to `crates/capi`.
- Prefer targeted tests first (module/function filter), then broader capi tests if needed.
- Keep test names concise (no required `test_` prefix).

## Style rules

- Follow existing RustPython capi coding style in neighboring files.
- Reuse `rustpython-vm` methods and types first; avoid duplicating VM logic in capi wrappers.
- When exposing previously private VM helpers, keep the API surface minimal and avoid unrelated refactors.
- Only expose and implement ABI-stable C-API surface needed for `abi3` / `abi3t`.
- Add comments only when they explain non-obvious behavior.
- Keep edits minimal and focused on requested API expansion.

## Completion checklist

- [ ] All requested functions implemented in mapped target file.
- [ ] New module exported in `crates/capi/src/lib.rs` when applicable.
- [ ] Basic safe-wrapper `pyo3` tests added/updated.
- [ ] Tests executed from `crates/capi` and passing for changed area.
- [ ] Final response includes changed file paths and test command summary.

## Example prompts this skill should handle

- "Implement these missing functions from `dictobject.h`."
- "Add `setobject.h` C-API functions in RustPython and include basic tests."
- "Port the listed `unicodeobject.h` APIs in capi, follow existing style, and run tests from `crates/capi`."
