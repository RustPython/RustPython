# Unimplemented C API functions

Mapping source: `pyo3-ffi/src/*.rs`, which mirrors the CPython header split used by the C API.

## `genericaliasobject.h`

RustPython C API target: `crates/capi/src/genericaliasobject.rs` (not present yet)

- `Py_GenericAlias`

## `import.h`

RustPython C API target: `crates/capi/src/import.rs`

- `PyImport_ExecCodeModuleEx`

## `longobject.h`

RustPython C API target: `crates/capi/src/longobject.rs`

- `PyLong_AsUnsignedLongLongMask`

## `modsupport.h`

RustPython C API target: `crates/capi/src/modsupport.rs` (not present yet)

- `PyModule_ExecDef`
- `PyModule_FromDefAndSpec2`

## `osmodule.h`

RustPython C API target: `crates/capi/src/osmodule.rs` (not present yet)

- `PyOS_FSPath`

## `pyerrors.h`

RustPython C API target: `crates/capi/src/pyerrors.rs`

- `PyException_SetContext`
- `PyUnicodeDecodeError_Create`

## `pystate.h`

RustPython C API target: `crates/capi/src/pystate.rs`

- `PyInterpreterState_Get`
- `PyInterpreterState_GetID`
