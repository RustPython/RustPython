use crate::util::CStrExt;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use core::ffi::{c_char, c_int};
use rustpython_vm::{AsObject, VirtualMachine};

fn call_codec_error_handler(
    vm: &VirtualMachine,
    handler_name: &str,
    exc: *mut PyObject,
) -> rustpython_vm::PyResult {
    vm.state
        .codec_registry
        .lookup_error(handler_name, vm)?
        .call((unsafe { exc.assume_borrowed() }.to_owned(),), vm)
}

fn codec_stream(
    vm: &VirtualMachine,
    encoding: *const c_char,
    stream: *mut PyObject,
    errors: *const c_char,
    method: &str,
) -> rustpython_vm::PyResult {
    let encoding = unsafe { encoding.try_as_str_opt(vm) }?.unwrap_or("utf-8");
    let errors = unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_str(errors));
    let stream = unsafe { stream.assume_borrowed() }.to_owned();
    let codec = vm.state.codec_registry.lookup(encoding, vm)?;
    let args = match errors {
        Some(errors) => vec![stream, errors.into()],
        None => vec![stream],
    };
    vm.call_method(codec.as_tuple().as_object(), method, args)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Register(search_function: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let search_function = unsafe { search_function.assume_borrowed() }.to_owned();
        vm.state.codec_registry.register(search_function, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Unregister(search_function: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let search_function = unsafe { search_function.assume_borrowed() }.to_owned();
        vm.state.codec_registry.unregister(search_function);
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_KnownEncoding(encoding: *const c_char) -> c_int {
    with_vm(|vm| {
        let encoding = unsafe { encoding.try_as_str(vm) }?;
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
        let object = unsafe { object.assume_borrowed() }.to_owned();
        let encoding = unsafe { encoding.try_as_str_opt(vm) }?.unwrap_or("utf-8");
        let errors =
            unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_utf8_str(errors));
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
        let object = unsafe { object.assume_borrowed() }.to_owned();
        let encoding = unsafe { encoding.try_as_str_opt(vm) }?.unwrap_or("utf-8");
        let errors =
            unsafe { errors.try_as_str_opt(vm) }?.map(|errors| vm.ctx.new_utf8_str(errors));
        vm.state.codec_registry.decode(object, encoding, errors, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Encoder(encoding: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { encoding.try_as_str(vm) }?;
        vm.state
            .codec_registry
            .lookup(encoding, vm)
            .map(|codec| codec.get_encode_func().to_owned())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_Decoder(encoding: *const c_char) -> *mut PyObject {
    with_vm(|vm| {
        let encoding = unsafe { encoding.try_as_str(vm) }?;
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
        let encoding = unsafe { encoding.try_as_str(vm) }?;
        let errors = unsafe { errors.try_as_str_opt(vm) }?.map(|s| vm.ctx.new_str(s));
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
        let encoding = unsafe { encoding.try_as_str(vm) }?;
        let errors = unsafe { errors.try_as_str_opt(vm) }?.map(|s| vm.ctx.new_str(s));
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
    with_vm(|vm| codec_stream(vm, encoding, stream, errors, "streamreader"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StreamWriter(
    encoding: *const c_char,
    stream: *mut PyObject,
    errors: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| codec_stream(vm, encoding, stream, errors, "streamwriter"))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_RegisterError(name: *const c_char, error: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let name = unsafe { name.try_as_str(vm) }?;
        let error = unsafe { error.assume_borrowed() }.to_owned();
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
        let name = unsafe { name.try_as_str(vm) }?;
        vm.state.codec_registry.lookup_error(name, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_StrictErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "strict", exc))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_IgnoreErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "ignore", exc))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_ReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "replace", exc))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_XMLCharRefReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "xmlcharrefreplace", exc))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_BackslashReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "backslashreplace", exc))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyCodec_NameReplaceErrors(exc: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| call_codec_error_handler(vm, "namereplace", exc))
}
