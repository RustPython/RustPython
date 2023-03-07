//! Implementation of Printf-Style string formatting
//! as per the [Python Docs](https://docs.python.org/3/library/stdtypes.html#printf-style-string-formatting).

use crate::{
    builtins::{
        try_f64_to_bigint, tuple, PyBaseExceptionRef, PyByteArray, PyBytes, PyFloat, PyInt, PyStr,
    },
    common::cformat::*,
    function::ArgIntoFloat,
    protocol::PyBuffer,
    stdlib::builtins,
    AsObject, PyObjectRef, PyResult, TryFromBorrowedObject, TryFromObject, VirtualMachine,
};
use itertools::Itertools;
use num_traits::cast::ToPrimitive;
use std::str::FromStr;

fn spec_format_bytes(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: PyObjectRef,
) -> PyResult<Vec<u8>> {
    match &spec.format_type {
        CFormatType::String(conversion) => match conversion {
            // Unlike strings, %r and %a are identical for bytes: the behaviour corresponds to
            // %a for strings (not %r)
            CFormatConversion::Repr | CFormatConversion::Ascii => {
                let b = builtins::ascii(obj, vm)?.into();
                Ok(b)
            }
            CFormatConversion::Str | CFormatConversion::Bytes => {
                if let Ok(buffer) = PyBuffer::try_from_borrowed_object(vm, &obj) {
                    Ok(buffer.contiguous_or_collect(|bytes| spec.format_bytes(bytes)))
                } else {
                    let bytes = vm
                        .get_special_method(obj, identifier!(vm, __bytes__))?
                        .map_err(|obj| {
                            vm.new_type_error(format!(
                                "%b requires a bytes-like object, or an object that \
                                    implements __bytes__, not '{}'",
                                obj.class().name()
                            ))
                        })?
                        .invoke((), vm)?;
                    let bytes = PyBytes::try_from_borrowed_object(vm, &bytes)?;
                    Ok(spec.format_bytes(bytes.as_bytes()))
                }
            }
        },
        CFormatType::Number(number_type) => match number_type {
            CNumberType::Decimal => match_class!(match &obj {
                ref i @ PyInt => {
                    Ok(spec.format_number(i.as_bigint()).into_bytes())
                }
                ref f @ PyFloat => {
                    Ok(spec
                        .format_number(&try_f64_to_bigint(f.to_f64(), vm)?)
                        .into_bytes())
                }
                obj => {
                    if let Some(method) = vm.get_method(obj.clone(), identifier!(vm, __int__)) {
                        let result = method?.call((), vm)?;
                        if let Some(i) = result.payload::<PyInt>() {
                            return Ok(spec.format_number(i.as_bigint()).into_bytes());
                        }
                    }
                    Err(vm.new_type_error(format!(
                        "%{} format: a number is required, not {}",
                        spec.format_char,
                        obj.class().name()
                    )))
                }
            }),
            _ => {
                if let Some(i) = obj.payload::<PyInt>() {
                    Ok(spec.format_number(i.as_bigint()).into_bytes())
                } else {
                    Err(vm.new_type_error(format!(
                        "%{} format: an integer is required, not {}",
                        spec.format_char,
                        obj.class().name()
                    )))
                }
            }
        },
        CFormatType::Float(_) => {
            let type_name = obj.class().name().to_string();
            let value = ArgIntoFloat::try_from_object(vm, obj).map_err(|e| {
                if e.fast_isinstance(vm.ctx.exceptions.type_error) {
                    // formatfloat in bytesobject.c generates its own specific exception
                    // text in this case, mirror it here.
                    vm.new_type_error(format!("float argument required, not {type_name}"))
                } else {
                    e
                }
            })?;
            Ok(spec.format_float(value.into()).into_bytes())
        }
        CFormatType::Character => {
            if let Some(i) = obj.payload::<PyInt>() {
                let ch = i
                    .try_to_primitive::<u8>(vm)
                    .map_err(|_| vm.new_overflow_error("%c arg not in range(256)".to_owned()))?
                    as char;
                return Ok(spec.format_char(ch).into_bytes());
            }
            if let Some(b) = obj.payload::<PyBytes>() {
                if b.len() == 1 {
                    return Ok(spec.format_char(b.as_bytes()[0] as char).into_bytes());
                }
            } else if let Some(ba) = obj.payload::<PyByteArray>() {
                let buf = ba.borrow_buf();
                if buf.len() == 1 {
                    return Ok(spec.format_char(buf[0] as char).into_bytes());
                }
            }
            Err(vm
                .new_type_error("%c requires an integer in range(256) or a single byte".to_owned()))
        }
    }
}

fn spec_format_string(
    vm: &VirtualMachine,
    spec: &CFormatSpec,
    obj: PyObjectRef,
    idx: &usize,
) -> PyResult<String> {
    match &spec.format_type {
        CFormatType::String(conversion) => {
            let result = match conversion {
                CFormatConversion::Ascii => builtins::ascii(obj, vm)?.into(),
                CFormatConversion::Str => obj.str(vm)?.as_str().to_owned(),
                CFormatConversion::Repr => obj.repr(vm)?.as_str().to_owned(),
                CFormatConversion::Bytes => {
                    // idx is the position of the %, we want the position of the b
                    return Err(vm.new_value_error(format!(
                        "unsupported format character 'b' (0x62) at index {}",
                        idx + 1
                    )));
                }
            };
            Ok(spec.format_string(result))
        }
        CFormatType::Number(number_type) => match number_type {
            CNumberType::Decimal => match_class!(match &obj {
                ref i @ PyInt => {
                    Ok(spec.format_number(i.as_bigint()))
                }
                ref f @ PyFloat => {
                    Ok(spec.format_number(&try_f64_to_bigint(f.to_f64(), vm)?))
                }
                obj => {
                    if let Some(method) = vm.get_method(obj.clone(), identifier!(vm, __int__)) {
                        let result = method?.call((), vm)?;
                        if let Some(i) = result.payload::<PyInt>() {
                            return Ok(spec.format_number(i.as_bigint()));
                        }
                    }
                    Err(vm.new_type_error(format!(
                        "%{} format: a number is required, not {}",
                        spec.format_char,
                        obj.class().name()
                    )))
                }
            }),
            _ => {
                if let Some(i) = obj.payload::<PyInt>() {
                    Ok(spec.format_number(i.as_bigint()))
                } else {
                    Err(vm.new_type_error(format!(
                        "%{} format: an integer is required, not {}",
                        spec.format_char,
                        obj.class().name()
                    )))
                }
            }
        },
        CFormatType::Float(_) => {
            let value = ArgIntoFloat::try_from_object(vm, obj)?;
            Ok(spec.format_float(value.into()))
        }
        CFormatType::Character => {
            if let Some(i) = obj.payload::<PyInt>() {
                let ch = i
                    .as_bigint()
                    .to_u32()
                    .and_then(std::char::from_u32)
                    .ok_or_else(|| {
                        vm.new_overflow_error("%c arg not in range(0x110000)".to_owned())
                    })?;
                return Ok(spec.format_char(ch));
            }
            if let Some(s) = obj.payload::<PyStr>() {
                if let Ok(ch) = s.as_str().chars().exactly_one() {
                    return Ok(spec.format_char(ch));
                }
            }
            Err(vm.new_type_error("%c requires int or char".to_owned()))
        }
    }
}

fn try_update_quantity_from_element(
    vm: &VirtualMachine,
    element: Option<&PyObjectRef>,
) -> PyResult<CFormatQuantity> {
    match element {
        Some(width_obj) => {
            if let Some(i) = width_obj.payload::<PyInt>() {
                let i = i.try_to_primitive::<i32>(vm)?.unsigned_abs();
                Ok(CFormatQuantity::Amount(i as usize))
            } else {
                Err(vm.new_type_error("* wants int".to_owned()))
            }
        }
        None => Err(vm.new_type_error("not enough arguments for format string".to_owned())),
    }
}

fn try_update_quantity_from_tuple<'a, I: Iterator<Item = &'a PyObjectRef>>(
    vm: &VirtualMachine,
    elements: &mut I,
    q: &mut Option<CFormatQuantity>,
) -> PyResult<()> {
    let Some(CFormatQuantity::FromValuesTuple) = q else {
        return Ok(());
    };
    let quantity = try_update_quantity_from_element(vm, elements.next())?;
    *q = Some(quantity);
    Ok(())
}

fn try_update_precision_from_tuple<'a, I: Iterator<Item = &'a PyObjectRef>>(
    vm: &VirtualMachine,
    elements: &mut I,
    p: &mut Option<CFormatPrecision>,
) -> PyResult<()> {
    let Some(CFormatPrecision::Quantity(CFormatQuantity::FromValuesTuple)) = p else {
        return Ok(());
    };
    let quantity = try_update_quantity_from_element(vm, elements.next())?;
    *p = Some(CFormatPrecision::Quantity(quantity));
    Ok(())
}

fn specifier_error(vm: &VirtualMachine) -> PyBaseExceptionRef {
    vm.new_type_error("format requires a mapping".to_owned())
}

pub(crate) fn cformat_bytes(
    vm: &VirtualMachine,
    format_string: &[u8],
    values_obj: PyObjectRef,
) -> PyResult<Vec<u8>> {
    let mut format = CFormatBytes::parse_from_bytes(format_string)
        .map_err(|err| vm.new_value_error(err.to_string()))?;
    let (num_specifiers, mapping_required) = format
        .check_specifiers()
        .ok_or_else(|| specifier_error(vm))?;

    let mut result = vec![];

    let is_mapping = values_obj.class().has_attr(identifier!(vm, __getitem__))
        && !values_obj.fast_isinstance(vm.ctx.types.tuple_type)
        && !values_obj.fast_isinstance(vm.ctx.types.bytes_type)
        && !values_obj.fast_isinstance(vm.ctx.types.bytearray_type);

    if num_specifiers == 0 {
        // literal only
        return if is_mapping
            || values_obj
                .payload::<tuple::PyTuple>()
                .map_or(false, |e| e.is_empty())
        {
            for (_, part) in format.iter_mut() {
                match part {
                    CFormatPart::Literal(literal) => result.append(literal),
                    CFormatPart::Spec(_) => unreachable!(),
                }
            }
            Ok(result)
        } else {
            Err(vm.new_type_error("not all arguments converted during bytes formatting".to_owned()))
        };
    }

    if mapping_required {
        // dict
        return if is_mapping {
            for (_, part) in format.iter_mut() {
                match part {
                    CFormatPart::Literal(literal) => result.append(literal),
                    CFormatPart::Spec(spec) => {
                        let value = match &spec.mapping_key {
                            Some(key) => values_obj.get_item(key.as_str(), vm)?,
                            None => unreachable!(),
                        };
                        let mut part_result = spec_format_bytes(vm, spec, value)?;
                        result.append(&mut part_result);
                    }
                }
            }
            Ok(result)
        } else {
            Err(vm.new_type_error("format requires a mapping".to_owned()))
        };
    }

    // tuple
    let values = if let Some(tup) = values_obj.payload_if_subclass::<tuple::PyTuple>(vm) {
        tup.as_slice()
    } else {
        std::slice::from_ref(&values_obj)
    };
    let mut value_iter = values.iter();

    for (_, part) in format.iter_mut() {
        match part {
            CFormatPart::Literal(literal) => result.append(literal),
            CFormatPart::Spec(spec) => {
                try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.min_field_width)?;
                try_update_precision_from_tuple(vm, &mut value_iter, &mut spec.precision)?;

                let value = match value_iter.next() {
                    Some(obj) => Ok(obj.clone()),
                    None => {
                        Err(vm.new_type_error("not enough arguments for format string".to_owned()))
                    }
                }?;
                let mut part_result = spec_format_bytes(vm, spec, value)?;
                result.append(&mut part_result);
            }
        }
    }

    // check that all arguments were converted
    if value_iter.next().is_some() && !is_mapping {
        Err(vm.new_type_error("not all arguments converted during bytes formatting".to_owned()))
    } else {
        Ok(result)
    }
}

pub(crate) fn cformat_string(
    vm: &VirtualMachine,
    format_string: &str,
    values_obj: PyObjectRef,
) -> PyResult<String> {
    let mut format = CFormatString::from_str(format_string)
        .map_err(|err| vm.new_value_error(err.to_string()))?;
    let (num_specifiers, mapping_required) = format
        .check_specifiers()
        .ok_or_else(|| specifier_error(vm))?;

    let mut result = String::new();

    let is_mapping = values_obj.class().has_attr(identifier!(vm, __getitem__))
        && !values_obj.fast_isinstance(vm.ctx.types.tuple_type)
        && !values_obj.fast_isinstance(vm.ctx.types.str_type);

    if num_specifiers == 0 {
        // literal only
        return if is_mapping
            || values_obj
                .payload::<tuple::PyTuple>()
                .map_or(false, |e| e.is_empty())
        {
            for (_, part) in format.iter() {
                match part {
                    CFormatPart::Literal(literal) => result.push_str(literal),
                    CFormatPart::Spec(_) => unreachable!(),
                }
            }
            Ok(result)
        } else {
            Err(vm
                .new_type_error("not all arguments converted during string formatting".to_owned()))
        };
    }

    if mapping_required {
        // dict
        return if is_mapping {
            for (idx, part) in format.iter() {
                match part {
                    CFormatPart::Literal(literal) => result.push_str(literal),
                    CFormatPart::Spec(spec) => {
                        let value = match &spec.mapping_key {
                            Some(key) => values_obj.get_item(key.as_str(), vm)?,
                            None => unreachable!(),
                        };
                        let part_result = spec_format_string(vm, spec, value, idx)?;
                        result.push_str(&part_result);
                    }
                }
            }
            Ok(result)
        } else {
            Err(vm.new_type_error("format requires a mapping".to_owned()))
        };
    }

    // tuple
    let values = if let Some(tup) = values_obj.payload_if_subclass::<tuple::PyTuple>(vm) {
        tup.as_slice()
    } else {
        std::slice::from_ref(&values_obj)
    };
    let mut value_iter = values.iter();

    for (idx, part) in format.iter_mut() {
        match part {
            CFormatPart::Literal(literal) => result.push_str(literal),
            CFormatPart::Spec(spec) => {
                try_update_quantity_from_tuple(vm, &mut value_iter, &mut spec.min_field_width)?;
                try_update_precision_from_tuple(vm, &mut value_iter, &mut spec.precision)?;

                let value = match value_iter.next() {
                    Some(obj) => Ok(obj.clone()),
                    None => {
                        Err(vm.new_type_error("not enough arguments for format string".to_owned()))
                    }
                }?;
                let part_result = spec_format_string(vm, spec, value, idx)?;
                result.push_str(&part_result);
            }
        }
    }

    // check that all arguments were converted
    if value_iter.next().is_some() && !is_mapping {
        Err(vm.new_type_error("not all arguments converted during string formatting".to_owned()))
    } else {
        Ok(result)
    }
}
