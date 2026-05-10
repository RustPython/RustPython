# Unimplemented C API functions

Mapping source: `pyo3-ffi/src/*.rs`, which mirrors the CPython header split used by the C API.

## `abstract.h`

RustPython C API target: `crates/capi/src/abstract_.rs`

- `PyIter_Check`
- `PyIter_NextItem`
- `PyIter_Send`
- `PyMapping_Items`
- `PyMapping_Keys`
- `PyMapping_Size`
- `PyMapping_Values`
- `PyNumber_Add`
- `PyNumber_Lshift`
- `PyNumber_Or`
- `PyNumber_Rshift`
- `PyNumber_Subtract`
- `PyObject_GetIter`
- `PyObject_IsInstance`
- `PyObject_IsSubclass`
- `PyObject_Size`
- `PySequence_Check`
- `PySequence_Concat`
- `PySequence_Count`
- `PySequence_DelItem`
- `PySequence_DelSlice`
- `PySequence_GetItem`
- `PySequence_GetSlice`
- `PySequence_InPlaceConcat`
- `PySequence_InPlaceRepeat`
- `PySequence_Index`
- `PySequence_List`
- `PySequence_Repeat`
- `PySequence_SetItem`
- `PySequence_SetSlice`
- `PySequence_Size`
- `PySequence_Tuple`

## `bytearrayobject.h`

RustPython C API target: `crates/capi/src/bytearrayobject.rs` (not present yet)

- `PyByteArray_AsString`
- `PyByteArray_Check`
- `PyByteArray_FromObject`
- `PyByteArray_FromStringAndSize`
- `PyByteArray_Resize`
- `PyByteArray_Size`

## `descrobject.h`

RustPython C API target: `crates/capi/src/descrobject.rs` (not present yet)

- `PyDictProxy_New`

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

## `moduleobject.h`

RustPython C API target: `crates/capi/src/moduleobject.rs`

- `PyModule_GetFilenameObject`
- `PyModule_NewObject`

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

## `pybuffer.h`

RustPython C API target: `crates/capi/src/pybuffer.rs` (not present yet)

- `PyBuffer_FromContiguous`
- `PyBuffer_GetPointer`
- `PyBuffer_IsContiguous`
- `PyBuffer_Release`
- `PyBuffer_ToContiguous`
- `PyObject_GetBuffer`

## `pycapsule.h`

RustPython C API target: `crates/capi/src/capsule.rs`

- `PyCapsule_GetContext`
- `PyCapsule_Import`
- `PyCapsule_SetContext`
- `PyCapsule_SetPointer`

## `pyerrors.h`

RustPython C API target: `crates/capi/src/pyerrors.rs`

- `PyException_GetCause`
- `PyException_GetContext`
- `PyException_SetContext`
- `PyUnicodeDecodeError_Create`

## `pystate.h`

RustPython C API target: `crates/capi/src/pystate.rs`

- `PyInterpreterState_Get`
- `PyInterpreterState_GetID`

## `setobject.h`

RustPython C API target: `crates/capi/src/setobject.rs` (not present yet)

- `PyFrozenSet_Check`
- `PyFrozenSet_New`
- `PySet_Add`
- `PySet_Check`
- `PySet_Clear`
- `PySet_Contains`
- `PySet_Discard`
- `PySet_New`
- `PySet_Pop`
- `PySet_Size`

## `sliceobject.h`

RustPython C API target: `crates/capi/src/sliceobject.rs` (not present yet)

- `PySlice_AdjustIndices`
- `PySlice_New`
- `PySlice_Unpack`

## `unicodeobject.h`

RustPython C API target: `crates/capi/src/unicodeobject.rs`

- `PyUnicode_AsUTF8String`
- `PyUnicode_DecodeFSDefaultAndSize`
- `PyUnicode_EncodeFSDefault`
- `PyUnicode_FromEncodedObject`

## `warnings.h`

RustPython C API target: `crates/capi/src/warnings.rs` (not present yet)

- `PyErr_WarnEx`
- `PyErr_WarnExplicit`

## `weakrefobject.h`

RustPython C API target: `crates/capi/src/weakrefobject.rs` (not present yet)

- `PyWeakref_CheckProxy`
- `PyWeakref_CheckRef`
- `PyWeakref_GetRef`
- `PyWeakref_NewProxy`
- `PyWeakref_NewRef`
