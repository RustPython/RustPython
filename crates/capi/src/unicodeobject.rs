use crate::object::define_py_check;
use crate::util::CStrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char, c_int};
use core::ptr::NonNull;
use core::slice;
use core::str;
use rustpython_vm::builtins::{PyStr, PyStrRef};
use rustpython_vm::common::wtf8::{CodePoint, Wtf8Buf};
use rustpython_vm::convert::ToPyObject;
use rustpython_vm::{AsObject, PyObjectRef, PyResult, VirtualMachine};

define_py_check!(fn PyUnicode_Check, types.str_type);
define_py_check!(exact fn PyUnicode_CheckExact, types.str_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromStringAndSize(
    s: *const c_char,
    len: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let len: usize = len
            .try_into()
            .map_err(|_| vm.new_system_error("length must be non-negative"))?;

        let text = if s.is_null() {
            if len != 0 {
                return Err(vm.new_system_error(
                    "PyUnicode_FromStringAndSize called with null data and non-zero len",
                ));
            }
            ""
        } else {
            let bytes = unsafe { slice::from_raw_parts(s.cast::<u8>(), len) };
            str::from_utf8(bytes).expect("PyUnicode_FromStringAndSize got non-UTF8 data")
        };

        Ok(vm.ctx.new_str(text))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromString(s: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let s = unsafe { s.try_as_str(vm)? };
        Ok(vm.ctx.new_str(s))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromObject(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        Ok(unsafe { &*obj }
            .try_downcast_ref::<PyStr>(vm)?
            .as_object()
            .str(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromOrdinal(ordinal: c_int) -> *mut PyObject {
    with_vm(|vm| {
        let ordinal: u32 = ordinal
            .try_into()
            .map_err(|_| vm.new_value_error("ordinal not in range(0x110000)"))?;
        let code_point = CodePoint::from_u32(ordinal)
            .ok_or_else(|| vm.new_value_error("ordinal not in range(0x110000)"))?;
        Ok(vm.ctx.new_str(Wtf8Buf::from_iter([code_point])))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8AndSize(
    obj: *mut PyObject,
    size: *mut isize,
) -> *const c_char {
    with_vm(|vm| {
        let unicode = unsafe { &*obj }.try_downcast_ref::<PyStr>(vm)?;

        let str = unicode.to_str().ok_or_else(|| {
            vm.new_system_error("PyUnicode_AsUTF8AndSize only supports UTF-8 or ASCII strings")
        })?;

        if size.is_null() {
            // We do not support null size arguments because the returned string is not NULL terminated.
            return Err(
                vm.new_system_error("size argument to PyUnicode_AsUTF8AndSize cannot be null")
            );
        }

        unsafe { *size = str.len() as isize };
        Ok(str.as_ptr())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsASCIIString(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "ascii", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsLatin1String(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "latin-1", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsRawUnicodeEscapeString(
    unicode: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "raw-unicode-escape", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF16String(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "utf-16", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF32String(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "utf-32", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUnicodeEscapeString(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "unicode-escape", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsEncodedString(
    unicode: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        let encoding = unsafe { encoding.try_as_str_opt(vm) }?.unwrap_or("utf-8");
        let errors =
            unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_utf8_str(errors));
        vm.state
            .codec_registry
            .encode_text(unicode, encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_AsUTF8String(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state
            .codec_registry
            .encode_text(unicode, "utf-8", None, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Decode(
    s: *const c_char,
    size: isize,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let size: usize = size
            .try_into()
            .map_err(|_| vm.new_system_error("size must be non-negative"))?;

        let bytes = if s.is_null() {
            if size != 0 {
                return Err(vm.new_system_error("decode called with null data and non-zero size"));
            }
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(s.cast::<u8>(), size) }.to_vec()
        };

        let encoding = unsafe { encoding.try_as_str_opt(vm)?.unwrap_or("utf-8") };
        let errors =
            unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_utf8_str(errors));

        vm.state
            .codec_registry
            .decode_text(vm.ctx.new_bytes(bytes).into(), encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeASCII(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"ascii".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeLatin1(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"latin-1".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeRawUnicodeEscape(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"raw-unicode-escape".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF7(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"utf-7".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUTF8(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"utf-8".as_ptr(), errors) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeUnicodeEscape(
    s: *const c_char,
    size: isize,
    errors: *const c_char,
) -> *mut PyObject {
    unsafe { PyUnicode_Decode(s, size, c"unicode-escape".as_ptr(), errors) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefaultAndSize(
    s: *const c_char,
    size: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let size: usize = size
            .try_into()
            .map_err(|_| vm.new_system_error("size must be non-negative"))?;

        decode_fsdefault_and_size(vm, s, size)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Concat(
    left: *mut PyObject,
    right: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let left = unsafe { &*left }.try_downcast_ref::<PyStr>(vm)?;
        let right = unsafe { &*right }.try_downcast_ref::<PyStr>(vm)?;
        vm._add(left.as_object(), right.as_object())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetLength(unicode: *mut PyObject) -> isize {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }.try_downcast_ref::<PyStr>(vm)?;
        Ok(unicode.char_len())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_GetDefaultEncoding() -> *const c_char {
    c"utf-8".as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternFromString(s: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let s = unsafe { s.try_as_str(vm)? };
        Ok(vm.ctx.intern_str(s).to_owned())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Compare(left: *mut PyObject, right: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let left = unsafe { &*left }.try_downcast_ref::<PyStr>(vm)?;
        let right = unsafe { &*right }.try_downcast_ref::<PyStr>(vm)?;
        Ok(match left.as_wtf8().cmp(right.as_wtf8()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_CompareWithASCIIString(
    left: *mut PyObject,
    right: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let left = unsafe { &*left }.try_downcast_ref::<PyStr>(vm)?;
        let right = unsafe { right.try_as_str(vm)? };
        Ok(match left.as_wtf8().cmp(right.into()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Equal(left: *mut PyObject, right: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let left = unsafe { &*left }.try_downcast_ref::<PyStr>(vm)?;
        let right = unsafe { &*right }.try_downcast_ref::<PyStr>(vm)?;
        Ok(left.as_wtf8() == right.as_wtf8())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8(
    unicode: *mut PyObject,
    string: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }.try_downcast_ref::<PyStr>(vm)?;
        let other = unsafe { string.try_as_str(vm)? };
        Ok(unicode.to_str().is_some_and(|s| s == other))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_DecodeFSDefault(s: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let size = unsafe { CStr::from_ptr(s) }.to_bytes().len();
        decode_fsdefault_and_size(vm, s, size)
    })
}

pub(crate) fn decode_fsdefault_and_size(
    vm: &VirtualMachine,
    s: *const c_char,
    size: usize,
) -> PyResult<PyStrRef> {
    let bytes = if s.is_null() {
        if size != 0 {
            return Err(vm.new_system_error(
                "PyUnicode_DecodeFSDefaultAndSize called with null data and non-zero size",
            ));
        }
        &[][..]
    } else {
        unsafe { slice::from_raw_parts(s.cast::<u8>(), size) }
    };

    vm.state.codec_registry.decode_text(
        vm.ctx.new_bytes(bytes.to_vec()).into(),
        vm.fs_encoding().as_str(),
        Some(vm.fs_encode_errors().to_owned()),
        vm,
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EncodeFSDefault(unicode: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let unicode = unsafe { &*unicode }
            .try_downcast_ref::<PyStr>(vm)?
            .to_owned();
        vm.state.codec_registry.encode_text(
            unicode,
            vm.fs_encoding().as_str(),
            Some(vm.fs_encode_errors().to_owned()),
            vm,
        )
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_FromEncodedObject(
    obj: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };

        if obj.downcast_ref::<PyStr>().is_some() {
            return Err(vm.new_type_error("decoding str is not supported"));
        }

        let encoding = unsafe { encoding.try_as_str_opt(vm) }?.unwrap_or("utf-8");
        let errors =
            unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_utf8_str(errors));

        obj.try_bytes_like(vm, |b| {
            vm.state.codec_registry.decode_text(
                vm.ctx.new_bytes(b.to_vec()).into(),
                encoding,
                errors,
                vm,
            )
        })?
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Contains(
    container: *mut PyObject,
    element: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let container = unsafe { &*container }.try_downcast_ref::<PyStr>(vm)?;
        let element = unsafe { &*element }.try_downcast_ref::<PyStr>(vm)?;
        Ok(container.as_wtf8().contains(element.as_wtf8()))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Format(
    format: *mut PyObject,
    args: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let format = unsafe { &*format }.try_downcast_ref::<PyStr>(vm)?;
        let result = format.__mod__(unsafe { &*args }.to_owned(), vm)?;
        Ok(result.to_pyobject(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_IsIdentifier(s: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let s = unsafe { &*s }.try_downcast_ref::<PyStr>(vm)?;
        Ok(s.isidentifier())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Partition(
    s: *mut PyObject,
    sep: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let s = unsafe { &*s }.try_downcast_ref::<PyStr>(vm)?;
        let sep = unsafe { &*sep }.try_downcast_ref::<PyStr>(vm)?;
        s.partition(sep.to_owned(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_RPartition(
    s: *mut PyObject,
    sep: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let s = unsafe { &*s }.try_downcast_ref::<PyStr>(vm)?;
        let sep = unsafe { &*sep }.try_downcast_ref::<PyStr>(vm)?;
        s.rpartition(sep.to_owned(), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_Translate(
    str_obj: *mut PyObject,
    table: *mut PyObject,
    _errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let str_obj = unsafe { &*str_obj }.try_downcast_ref::<PyStr>(vm)?;
        Ok(str_obj
            .translate(unsafe { &*table }.to_owned(), vm)?
            .to_pyobject(vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_InternInPlace(string: *mut *mut PyObject) {
    with_vm(|vm| {
        let old_str = unsafe { PyObjectRef::from_raw(NonNull::new_unchecked(*string)) }
            .downcast_exact::<PyStr>(vm)
            .expect("PyUnicode_InternInPlace called with non-string object");

        let interned: PyObjectRef = vm.ctx.intern_str(old_str).to_owned().into();

        unsafe { *string = interned.into_raw().as_ptr() }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicode_EqualToUTF8AndSize(
    unicode: *mut PyObject,
    string: *const c_char,
    size: isize,
) -> c_int {
    with_vm(|vm| {
        let size = size.try_into().map_err(|_| {
            vm.new_system_error("Negative size passed to PyUnicode_EqualToUTF8AndSize")
        })?;

        let unicode = unsafe { &*unicode }.try_downcast_ref::<PyStr>(vm)?;
        let result = unsafe {
            let slice = slice::from_raw_parts(string as _, size);
            str::from_utf8(slice)
        }
        .ok()
        .and_then(|other| Some(unicode.to_str()? == other))
        .unwrap_or(false);

        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};

    use pyo3::intern;
    use pyo3::prelude::*;
    use pyo3::types::{PyBytes, PyString, PyStringMethods};

    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn unicode() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(string.is_instance_of::<PyString>());
            assert_eq!(string.to_str().unwrap(), "Hello, World!");
            assert_eq!(string, "Hello, World!");
        })
    }

    #[test]
    fn intern_str() {
        Python::attach(|py| {
            let _string = intern!(py, "Hello, World!");
        })
    }

    #[test]
    fn encode_utf8_via_wrapper() {
        Python::attach(|py| {
            let s = PyString::new(py, "h\u{00E9}llo");
            let encoded = s.encode_utf8().unwrap();
            assert_eq!(encoded.as_bytes(), "h\u{00E9}llo".as_bytes());
        })
    }

    #[test]
    fn from_encoded_object_bytes() {
        Python::attach(|py| {
            let src = PyBytes::new(py, b"h\xC3\xA9llo");
            let s = PyString::from_encoded_object(src.as_any(), None, None).unwrap();
            assert_eq!(s.to_str().unwrap(), "h\u{00E9}llo");
        })
    }

    #[cfg(unix)]
    #[test]
    fn fs_default_roundtrip_non_utf8_unix() {
        Python::attach(|py| {
            let original = OsStr::from_bytes(&[b'f', b'o', 0x80]);
            let py_str = original.into_pyobject(py).unwrap();
            let roundtrip: OsString = py_str.extract().unwrap();
            assert_eq!(roundtrip.as_os_str().as_bytes(), original.as_bytes());
        })
    }

    #[test]
    fn fs_default_roundtrip_utf8() {
        Python::attach(|py| {
            let original = OsStr::new("hello.txt");
            let py_str = original.into_pyobject(py).unwrap();
            let roundtrip: OsString = py_str.extract().unwrap();
            assert_eq!(roundtrip, original);
        })
    }
}
