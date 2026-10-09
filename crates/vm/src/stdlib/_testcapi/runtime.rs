use crate::{
    AsObject, Py, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyBytes, PyModule, PyStr, PyStrRef, PyTraceback, PyType},
    function::{ItemDoc, PosArgs, PyMethodDef, PyMethodFlags},
};

fn format_size(
    chars: &mut core::iter::Peekable<core::str::Chars<'_>>,
    vm: &VirtualMachine,
) -> PyResult<Option<usize>> {
    let mut value = None;
    while let Some(character) = chars.peek().copied().filter(char::is_ascii_digit) {
        chars.next();
        let size = value
            .unwrap_or(0usize)
            .checked_mul(10)
            .and_then(|value| value.checked_add((character as u8 - b'0') as usize))
            .filter(|value| isize::try_from(*value).is_ok())
            .ok_or_else(|| vm.new_value_error("format field too big"))?;
        value = Some(size);
    }
    Ok(value)
}

fn format_type_name(
    class: &Py<PyType>,
    alternate: bool,
    vm: &VirtualMachine,
) -> PyResult<PyStrRef> {
    let mut name = class.fully_qualified_name(vm)?;
    if alternate {
        let module = class.__module__(vm)?;
        if let Some(module) = module
            .downcast_ref::<PyStr>()
            .and_then(|module| module.to_str())
            && !matches!(module, "builtins" | "__main__")
            && name
                .strip_prefix(module)
                .is_some_and(|rest| rest.starts_with('.'))
        {
            name.replace_range(module.len()..=module.len(), ":");
        }
    }
    Ok(vm.ctx.new_str(name))
}

// The fixture supplies PyObject* varargs. Parse the C formatter's object
// conversions before using the ordinary string renderer for width/precision;
// notably a bare precision dot leaves precision unspecified in the C API.
fn format_unraisable_message(
    format: &str,
    objects: &[PyObjectRef],
    vm: &VirtualMachine,
) -> PyResult<PyStrRef> {
    if !format.is_ascii() {
        return Err(vm.new_value_error("PyUnicode_FromFormatV requires an ASCII format"));
    }
    let mut chars = format.chars().peekable();
    let mut translated = String::new();
    let mut converted = Vec::new();
    let mut objects = objects.iter();
    while let Some(character) = chars.next() {
        translated.push(character);
        if character != '%' {
            continue;
        }
        if chars.next_if_eq(&'%').is_some() {
            translated.push('%');
            continue;
        }
        let mut left_justify = false;
        let mut alternate = false;
        loop {
            match chars.peek() {
                Some('-') => left_justify = true,
                Some('#') => alternate = true,
                Some('0') => {}
                _ => break,
            }
            chars.next();
        }
        let width = format_size(&mut chars, vm)?;
        let precision = if chars.next_if_eq(&'.').is_some() {
            format_size(&mut chars, vm)?
        } else {
            None
        };
        let spec = chars
            .next()
            .ok_or_else(|| vm.new_value_error("incomplete format"))?;
        let object = objects
            .next()
            .ok_or_else(|| vm.new_type_error("not enough format arguments"))?;
        let value = match spec {
            'R' => object.repr(vm)?,
            'S' => object.str(vm)?,
            'A' => crate::stdlib::builtins::ascii(object.clone(), vm)?,
            'U' => PyStrRef::try_from_object(vm, object.clone())?,
            'T' => format_type_name(object.class(), alternate, vm)?,
            'N' => format_type_name(object.try_downcast_ref::<PyType>(vm)?, alternate, vm)?,
            'p' if width.is_none() && precision.is_none() => {
                vm.ctx.new_str(format!("0x{:x}", object.get_id()))
            }
            // Integer and character-pointer varargs cannot be represented by
            // this PyObject*-only fixture without inventing C ABI behavior.
            _ => return Err(vm.new_value_error("unsupported object format")),
        };
        if left_justify {
            translated.push('-');
        }
        if let Some(width) = width {
            translated.push_str(&width.to_string());
        }
        if let Some(precision) = precision {
            translated.push('.');
            translated.push_str(&precision.to_string());
        }
        translated.push('s');
        converted.push(vm.ctx.new_str(value.as_wtf8()).into());
    }
    let translated = vm.ctx.new_str(translated);
    let values = vm.ctx.new_tuple(converted);
    let message = crate::cformat::cformat_string(vm, translated.as_wtf8(), values.as_object())?;
    Ok(vm.ctx.new_str(message))
}

fn err_formatunraisable(args: PosArgs, vm: &VirtualMachine) -> PyResult<()> {
    let [exception, format, objects @ ..] = args.as_ref() else {
        return Err(vm.new_type_error("err_formatunraisable requires at least 2 arguments"));
    };
    if objects.len() > 10 {
        return Err(vm.new_type_error("err_formatunraisable takes at most 12 arguments"));
    }
    let exception = PyBaseExceptionRef::try_from_object(vm, exception.clone())?;
    let bytes = if vm.is_none(format) {
        None
    } else if let Some(format) = format.downcast_ref::<PyStr>() {
        let encoded = vm
            .state
            .codec_registry
            .encode_text(format.to_owned(), "utf-8", None, vm)?;
        Some(encoded.as_bytes().to_vec())
    } else if let Some(bytes) = format.downcast_ref::<PyBytes>() {
        Some(bytes.as_bytes().to_vec())
    } else {
        return Err(vm.new_type_error("format must be str, bytes or None"));
    };
    let message = bytes.as_ref().and_then(|bytes| {
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        core::str::from_utf8(&bytes[..end])
            .ok()
            .and_then(|format| format_unraisable_message(format, objects, vm).ok())
    });
    // PyErr_FormatUnraisable attaches the calling frame when the exception
    // did not already carry a traceback.
    if exception.traceback().is_none()
        && let Some(frame) = vm.current_frame()
    {
        let lasti = frame.lasti().saturating_sub(1);
        let lineno = frame.current_location().line;
        let traceback = PyTraceback::new(None, frame, (lasti * 2) as i32, lineno).into_ref(&vm.ctx);
        exception.set_traceback(Some(traceback));
    }
    vm.run_unraisable_with_message(exception, message, vm.ctx.none());
    Ok(())
}

pub(super) fn extend_module(vm: &VirtualMachine, module: &Py<PyModule>) -> PyResult<()> {
    const METHODS: &[PyMethodDef] = &[PyMethodDef::new_const(
        "err_formatunraisable",
        err_formatunraisable,
        PyMethodFlags::VARARGS,
        ItemDoc::NONE,
    )];
    for method in METHODS {
        let function = method.build_function(&vm.ctx);
        drop(
            function
                .module
                .store(Some(vm.ctx.new_str("_testcapi").into())),
        );
        module.set_attr(method.name, function, vm)?;
    }
    Ok(())
}
