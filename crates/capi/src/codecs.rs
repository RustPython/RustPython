use crate::{PyObject, pystate::with_vm};
use core::ffi::{CStr, c_char, c_int};
use rustpython_vm::AsObject;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Register(search_function: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let search_function = unsafe { &*search_function }.to_owned();
        vm.state.codec_registry.register(search_function, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Unregister(search_function: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let search_function = unsafe { &*search_function }.to_owned();
        vm.state.codec_registry.unregister(search_function);
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_KnownEncoding(encoding: *const c_char) -> c_int {
    with_vm(|vm| {
        let encoding = unsafe { CStr::from_ptr(encoding) }
            .to_str()
            .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?;
        match vm.state.codec_registry.lookup(encoding, vm) {
            Ok(_) => Ok(true),
            Err(err) if err.fast_isinstance(vm.ctx.exceptions.lookup_error) => Ok(false),
            Err(err) => Err(err),
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Encode(
    object: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let object = unsafe { &*object }.to_owned();
        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_utf8_str(errors))
        };
        vm.state.codec_registry.encode(object, encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Decode(
    object: *mut PyObject,
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let object = unsafe { &*object }.to_owned();
        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_utf8_str(errors))
        };
        vm.state.codec_registry.decode(object, encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Encoder(encoding: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { CStr::from_ptr(encoding) }
            .to_str()
            .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?;
        vm.state
            .codec_registry
            .lookup(encoding, vm)
            .map(|codec| codec.get_encode_func().to_owned())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Decoder(encoding: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { CStr::from_ptr(encoding) }
            .to_str()
            .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?;
        vm.state
            .codec_registry
            .lookup(encoding, vm)
            .map(|codec| codec.get_decode_func().to_owned())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IncrementalEncoder(
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { CStr::from_ptr(encoding) }
            .to_str()
            .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?;
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_str(errors))
        };
        let codec = vm.state.codec_registry.lookup(encoding, vm)?;
        codec.get_incremental_encoder(errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IncrementalDecoder(
    encoding: *const c_char,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { CStr::from_ptr(encoding) }
            .to_str()
            .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?;
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_str(errors))
        };
        let codec = vm.state.codec_registry.lookup(encoding, vm)?;
        codec.get_incremental_decoder(errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StreamReader(
    encoding: *const c_char,
    stream: *mut PyObject,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_str(errors))
        };
        let stream = unsafe { &*stream }.to_owned();
        let codec = vm.state.codec_registry.lookup(encoding, vm)?;
        let args = match errors {
            Some(errors) => vec![stream, errors.into()],
            None => vec![stream],
        };
        vm.call_method(codec.as_tuple().as_object(), "streamreader", args)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StreamWriter(
    encoding: *const c_char,
    stream: *mut PyObject,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = if encoding.is_null() {
            "utf-8"
        } else {
            unsafe { CStr::from_ptr(encoding) }
                .to_str()
                .map_err(|_| vm.new_system_error("encoding must be valid UTF-8"))?
        };
        let errors = if errors.is_null() {
            None
        } else {
            let errors = unsafe { CStr::from_ptr(errors) }
                .to_str()
                .map_err(|_| vm.new_system_error("errors must be valid UTF-8"))?;
            Some(vm.ctx.new_str(errors))
        };
        let stream = unsafe { &*stream }.to_owned();
        let codec = vm.state.codec_registry.lookup(encoding, vm)?;
        let args = match errors {
            Some(errors) => vec![stream, errors.into()],
            None => vec![stream],
        };
        vm.call_method(codec.as_tuple().as_object(), "streamwriter", args)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_RegisterError(name: *const c_char, error: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .map_err(|_| vm.new_system_error("name must be valid UTF-8"))?;
        let error = unsafe { &*error }.to_owned();
        if !error.is_callable() {
            return Err(vm.new_type_error("handler must be callable"));
        }
        vm.state
            .codec_registry
            .register_error(name.to_owned(), error);
        Ok(())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_LookupError(name: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(name) }
            .to_str()
            .map_err(|_| vm.new_system_error("name must be valid UTF-8"))?;
        vm.state.codec_registry.lookup_error(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StrictErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm.state.codec_registry.lookup_error("strict", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IgnoreErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm.state.codec_registry.lookup_error("ignore", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_ReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm.state.codec_registry.lookup_error("replace", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_XMLCharRefReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm
            .state
            .codec_registry
            .lookup_error("xmlcharrefreplace", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_BackslashReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm
            .state
            .codec_registry
            .lookup_error("backslashreplace", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_NameReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let err = vm.state.codec_registry.lookup_error("namereplace", vm)?;
        let exc = unsafe { &*exc }.to_owned();
        err.call((exc,), vm)
    })
}
