# Unimplemented C API functions

Mapping source: `pyo3-ffi/src/*.rs`, which mirrors the CPython header split used by the C API.

## `dictobject.h`

RustPython C API target: `crates/capi/src/dictobject.rs`

- `PyDict_Contains`
- `PyDict_Copy`
- `PyDict_DelItem`
- `PyDict_Items`
- `PyDict_Keys`
- `PyDict_Merge`
- `PyDict_MergeFromSeq2`
- `PyDict_Update`
- `PyDict_Values`

## `genericaliasobject.h`

RustPython C API target: `crates/capi/src/genericaliasobject.rs` (not present yet)

- `Py_GenericAlias`

## `import.h`

RustPython C API target: `crates/capi/src/import.rs`

- `PyImport_ExecCodeModuleEx`

## `listobject.h`

RustPython C API target: `crates/capi/src/listobject.rs`

- `PyList_AsTuple`
- `PyList_GetSlice`
- `PyList_SetSlice`
- `PyList_Sort`

## `longobject.h`

RustPython C API target: `crates/capi/src/longobject.rs`

- `PyLong_AsUnsignedLongLongMask`

## `modsupport.h`

RustPython C API target: `crates/capi/src/modsupport.rs` (not present yet)

- `PyModule_ExecDef`
- `PyModule_FromDefAndSpec2`

## `object.h`

RustPython C API target: `crates/capi/src/object.rs`

- `PyCallable_Check`
- `PyObject_ClearWeakRefs`
- `PyObject_Dir`
- `PyObject_GenericGetAttr`
- `PyObject_GetOptionalAttr`
- `PyObject_RichCompare`
- `PyType_GetModuleName`

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

## `unicodeobject.h`

RustPython C API target: `crates/capi/src/unicodeobject.rs`

- `PyUnicode_AsUTF8String`
- `PyUnicode_DecodeFSDefaultAndSize`
- `PyUnicode_EncodeFSDefault`
- `PyUnicode_FromEncodedObject`
