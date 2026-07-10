use crate::util::CStrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{c_char, c_int};
use core::ptr::NonNull;
use core::slice;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_Create(
    encoding: *const c_char,
    object: *const c_char,
    length: isize,
    start: isize,
    end: isize,
    reason: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { encoding.try_as_str(vm) }?;
        let reason = unsafe { reason.try_as_str(vm) }?;
        let length: usize = length
            .try_into()
            .map_err(|_| vm.new_system_error("length must be non-negative"))?;
        let start: usize = start
            .try_into()
            .map_err(|_| vm.new_system_error("start must be non-negative"))?;
        let end: usize = end
            .try_into()
            .map_err(|_| vm.new_system_error("end must be non-negative"))?;

        let bytes = if object.is_null() {
            if length != 0 {
                return Err(vm.new_system_error(
                    "PyUnicodeDecodeError_Create called with null object and non-zero length",
                ));
            }
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(object.cast::<u8>(), length) }.to_vec()
        };

        let exc = vm.new_unicode_decode_error_real(
            vm.ctx.new_str(encoding),
            vm.ctx.new_bytes(bytes),
            start,
            end,
            vm.ctx.new_str(reason),
        );
        Ok(exc)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetEncoding(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("encoding", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetObject(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("object", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetReason(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("reason", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetStart(
    exc: *mut PyObject,
    start: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let start =
            NonNull::new(start).ok_or_else(|| vm.new_system_error("start must not be null"))?;
        let value = unsafe { &*exc }.get_attr("start", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(0, object_len.saturating_sub(1) as isize)
        };
        unsafe { start.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_GetEnd(exc: *mut PyObject, end: *mut isize) -> c_int {
    with_vm(|vm| {
        let end = NonNull::new(end).ok_or_else(|| vm.new_system_error("end must not be null"))?;
        let value = unsafe { &*exc }.get_attr("end", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(1, object_len as isize)
        };
        unsafe { end.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetStart(exc: *mut PyObject, start: isize) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("start", vm.ctx.new_int(start), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetEnd(exc: *mut PyObject, end: isize) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("end", vm.ctx.new_int(end), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeDecodeError_SetReason(
    exc: *mut PyObject,
    reason: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let reason = unsafe { reason.try_as_str(vm)? };
        unsafe { &*exc }.set_attr("reason", vm.ctx.new_str(reason), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetEncoding(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("encoding", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetObject(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("object", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetReason(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("reason", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetStart(
    exc: *mut PyObject,
    start: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let start =
            NonNull::new(start).ok_or_else(|| vm.new_system_error("start must not be null"))?;
        let value = unsafe { &*exc }.get_attr("start", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(0, object_len.saturating_sub(1) as isize)
        };
        unsafe { start.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_GetEnd(exc: *mut PyObject, end: *mut isize) -> c_int {
    with_vm(|vm| {
        let end = NonNull::new(end).ok_or_else(|| vm.new_system_error("end must not be null"))?;
        let value = unsafe { &*exc }.get_attr("end", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(1, object_len as isize)
        };
        unsafe { end.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetStart(exc: *mut PyObject, start: isize) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("start", vm.ctx.new_int(start), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetEnd(exc: *mut PyObject, end: isize) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("end", vm.ctx.new_int(end), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeEncodeError_SetReason(
    exc: *mut PyObject,
    reason: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let reason = unsafe { reason.try_as_str(vm)? };
        unsafe { &*exc }.set_attr("reason", vm.ctx.new_str(reason), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetObject(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("object", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetReason(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*exc }.get_attr("reason", vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetStart(
    exc: *mut PyObject,
    start: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let start =
            NonNull::new(start).ok_or_else(|| vm.new_system_error("start must not be null"))?;
        let value = unsafe { &*exc }.get_attr("start", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(0, object_len.saturating_sub(1) as isize)
        };
        unsafe { start.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_GetEnd(
    exc: *mut PyObject,
    end: *mut isize,
) -> c_int {
    with_vm(|vm| {
        let end = NonNull::new(end).ok_or_else(|| vm.new_system_error("end must not be null"))?;
        let value = unsafe { &*exc }.get_attr("end", vm)?;
        let value = value.try_index(vm)?.try_to_primitive::<isize>(vm)?;
        let object_len = unsafe { &*exc }.get_attr("object", vm)?.length(vm)?;
        let value = if object_len == 0 {
            0
        } else {
            value.clamp(1, object_len as isize)
        };
        unsafe { end.write(value) };
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetStart(
    exc: *mut PyObject,
    start: isize,
) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("start", vm.ctx.new_int(start), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetEnd(exc: *mut PyObject, end: isize) -> c_int {
    with_vm(|vm| unsafe { &*exc }.set_attr("end", vm.ctx.new_int(end), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyUnicodeTranslateError_SetReason(
    exc: *mut PyObject,
    reason: *const c_char,
) -> c_int {
    with_vm(|vm| {
        let reason = unsafe { reason.try_as_str(vm)? };
        unsafe { &*exc }.set_attr("reason", vm.ctx.new_str(reason), vm)
    })
}
