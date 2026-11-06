use super::PyMemoryView;
use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    buffer::{Endianness, FormatSpec, FormatType, pack_float16, unpack_float32},
    builtins::{PyBaseExceptionRef, PyBytes, PyInt},
    common::rc::PyRc,
    function::{ArgIntoBool, ArgIntoComplex, ArgIntoFloat},
};
use malachite_bigint::{BigInt, Sign};
use num_traits::PrimInt;
use std::os::raw;

/// A scalar format accepted by memoryview, rather than the full struct grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NativeFormat(FormatType);

impl NativeFormat {
    pub(super) fn parse(format: &str) -> Option<Self> {
        let format = format.strip_prefix('@').unwrap_or(format);
        let code = match format.as_bytes() {
            [b'Z', b'f'] => FormatType::FloatComplex,
            [b'Z', b'd'] => FormatType::DoubleComplex,
            [code] if b"cbBhHiIlLqQnNP?fde".contains(code) => FormatType::try_from(*code).ok()?,
            _ => return None,
        };
        Some(Self(code))
    }

    pub(super) fn fmtchar(self) -> u8 {
        match self.0 {
            FormatType::FloatComplex | FormatType::DoubleComplex => b'Z',
            code => code as u8,
        }
    }

    fn name(self) -> &'static str {
        match self.0 {
            FormatType::SByte => "b",
            FormatType::UByte => "B",
            FormatType::Char => "c",
            FormatType::Short => "h",
            FormatType::UShort => "H",
            FormatType::Int => "i",
            FormatType::UInt => "I",
            FormatType::Long => "l",
            FormatType::ULong => "L",
            FormatType::LongLong => "q",
            FormatType::ULongLong => "Q",
            FormatType::SSizeT => "n",
            FormatType::SizeT => "N",
            FormatType::Float => "f",
            FormatType::Double => "d",
            FormatType::Half => "e",
            FormatType::Bool => "?",
            FormatType::VoidP => "P",
            FormatType::FloatComplex => "Zf",
            FormatType::DoubleComplex => "Zd",
            _ => unreachable!("not a native memoryview scalar format"),
        }
    }

    pub(super) fn size(self) -> usize {
        self.0.info(Endianness::Native).size
    }

    pub(super) fn unpack(self, bytes: &[u8], vm: &VirtualMachine) -> PyObjectRef {
        // memoryview uses PyFloat_Unpack4 for complex components, unlike the
        // native struct path's C float-to-double cast, which quiets sNaNs.
        if self.0 == FormatType::FloatComplex {
            let re = unpack_float32(u32::from_ne_bytes(bytes[..4].try_into().unwrap()));
            let im = unpack_float32(u32::from_ne_bytes(bytes[4..8].try_into().unwrap()));
            return vm
                .ctx
                .new_complex(num_complex::Complex64::new(re, im))
                .into();
        }
        let unpack = self.0.info(Endianness::Native).unpack.unwrap();
        unpack(vm, &bytes[..self.size()])
    }

    // unpack_cmp: identical native formats need neither Python objects nor
    // struct parsing. Floating values and booleans cannot use byte equality.
    pub(super) fn equals(self, left: &[u8], right: &[u8]) -> bool {
        match self.0 {
            FormatType::Bool => (left[0] != 0) == (right[0] != 0),
            FormatType::Half => {
                let left = half::f16::from_bits(u16::from_ne_bytes(left.try_into().unwrap()));
                let right = half::f16::from_bits(u16::from_ne_bytes(right.try_into().unwrap()));
                left == right
            }
            FormatType::Float | FormatType::FloatComplex => left
                .as_chunks::<4>()
                .0
                .iter()
                .zip(right.as_chunks::<4>().0)
                .all(|(a, b)| f32::from_ne_bytes(*a) == f32::from_ne_bytes(*b)),
            FormatType::Double | FormatType::DoubleComplex => left
                .as_chunks::<8>()
                .0
                .iter()
                .zip(right.as_chunks::<8>().0)
                .all(|(a, b)| f64::from_ne_bytes(*a) == f64::from_ne_bytes(*b)),
            _ => left == right,
        }
    }

    fn type_error(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_type_error(format!(
            "memoryview: invalid type for format '{}'",
            self.name()
        ))
    }

    fn value_error(self, vm: &VirtualMachine) -> PyBaseExceptionRef {
        vm.new_value_error(format!(
            "memoryview: invalid value for format '{}'",
            self.name()
        ))
    }

    // fix_error_int: conversion errors use the unprefixed native format, but
    // exceptions outside these three classes belong to the callback unchanged.
    fn fix_error(self, error: PyBaseExceptionRef, vm: &VirtualMachine) -> PyBaseExceptionRef {
        if error.fast_isinstance(vm.ctx.exceptions.type_error) {
            self.type_error(vm)
        } else if error.fast_isinstance(vm.ctx.exceptions.overflow_error)
            || error.fast_isinstance(vm.ctx.exceptions.value_error)
        {
            self.value_error(vm)
        } else {
            error
        }
    }

    fn integer<T>(self, value: &PyObject, vm: &VirtualMachine) -> PyResult<T>
    where
        T: PrimInt + for<'a> TryFrom<&'a BigInt>,
    {
        value
            .try_index(vm)
            .and_then(|value| value.try_to_primitive_in_range(vm))
            .map_err(|error| self.fix_error(error, vm))
    }

    fn narrow_integer<T, U>(
        self,
        view: &PyMemoryView,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<U>
    where
        T: PrimInt + for<'a> TryFrom<&'a BigInt>,
        U: TryFrom<T>,
    {
        let value: T = self.integer(value, vm)?;
        view.try_not_released(vm)?;
        U::try_from(value).map_err(|_| self.value_error(vm))
    }

    pub(super) fn pack(
        self,
        view: &PyMemoryView,
        value: PyObjectRef,
        vm: &VirtualMachine,
        write: impl FnOnce(&[u8]),
    ) -> PyResult<()> {
        // Conversion can call Python and release the view. Check immediately
        // after conversion, before checking a narrower destination's range.
        macro_rules! integer {
            ($convert:ty, $store:ty) => {{
                let value = self.narrow_integer::<$convert, $store>(view, &value, vm)?;
                write(&value.to_ne_bytes())
            }};
        }

        match self.0 {
            FormatType::SByte => integer!(raw::c_long, i8),
            FormatType::UByte => integer!(raw::c_ulong, u8),
            FormatType::Short => integer!(raw::c_long, raw::c_short),
            FormatType::UShort => integer!(raw::c_ulong, raw::c_ushort),
            FormatType::Int => integer!(raw::c_long, raw::c_int),
            FormatType::UInt => integer!(raw::c_ulong, raw::c_uint),
            FormatType::Long => integer!(raw::c_long, raw::c_long),
            FormatType::ULong => integer!(raw::c_ulong, raw::c_ulong),
            FormatType::LongLong => integer!(raw::c_longlong, raw::c_longlong),
            FormatType::ULongLong => integer!(raw::c_ulonglong, raw::c_ulonglong),
            FormatType::SSizeT => integer!(isize, isize),
            FormatType::SizeT => integer!(usize, usize),
            FormatType::Float | FormatType::Double | FormatType::Half => {
                let value = value
                    .try_to_value::<ArgIntoFloat>(vm)
                    .map_err(|error| self.fix_error(error, vm))?
                    .into_float();
                view.try_not_released(vm)?;
                match self.0 {
                    // Unlike struct.pack('f'), a native memoryview assignment
                    // uses a C cast and permits a finite value to become infinity.
                    FormatType::Float => write(&(value as f32).to_ne_bytes()),
                    FormatType::Double => write(&value.to_ne_bytes()),
                    _ => {
                        let bits = pack_float16(value).ok_or_else(|| self.value_error(vm))?;
                        write(&bits.to_ne_bytes())
                    }
                }
            }
            FormatType::FloatComplex | FormatType::DoubleComplex => {
                let value = value
                    .try_to_value::<ArgIntoComplex>(vm)
                    .map_err(|error| self.fix_error(error, vm))?
                    .into_complex();
                view.try_not_released(vm)?;
                if self.0 == FormatType::FloatComplex {
                    write(
                        [
                            (value.re as f32).to_ne_bytes(),
                            (value.im as f32).to_ne_bytes(),
                        ]
                        .as_flattened(),
                    );
                } else {
                    write([value.re.to_ne_bytes(), value.im.to_ne_bytes()].as_flattened());
                }
            }
            FormatType::Bool => {
                // Truth testing preserves even TypeError and ValueError.
                let value = value.try_to_value::<ArgIntoBool>(vm)?.into_bool();
                view.try_not_released(vm)?;
                write(&[u8::from(value)])
            }
            FormatType::Char => {
                let value = value
                    .downcast_ref::<PyBytes>()
                    .ok_or_else(|| self.type_error(vm))?;
                match value.as_bytes() {
                    [value] => {
                        // An index callback can release even a character view.
                        // Never write through its stale storage in that case.
                        view.try_not_released(vm)?;
                        write(&[*value]);
                    }
                    _ => return Err(self.value_error(vm)),
                }
            }
            FormatType::VoidP => {
                // PyLong_AsVoidPtr accepts actual ints, not __index__, and
                // allows signed negative values as well as unsigned positives.
                let value = value
                    .downcast_ref::<PyInt>()
                    .ok_or_else(|| self.type_error(vm))?;
                let value = if value.as_bigint().sign() == Sign::Minus {
                    value
                        .try_to_primitive_in_range::<isize>(vm)
                        .map(|value| value as usize)
                } else {
                    value.try_to_primitive_in_range::<usize>(vm)
                }
                .map_err(|error| self.fix_error(error, vm))?;
                view.try_not_released(vm)?;
                write(&value.to_ne_bytes())
            }
            _ => unreachable!("not a native memoryview scalar format"),
        };
        Ok(())
    }
}

/// struct_get_unpacker / fix_struct_error_int. Equality uses the same strict
/// parser and bounded interpreter-local cache as the struct module.
pub(super) fn comparison_format(
    format: &str,
    vm: &VirtualMachine,
) -> PyResult<Option<PyRc<FormatSpec>>> {
    match vm
        .state
        .struct_format_cache
        .get_or_parse(format.as_bytes(), vm)
    {
        Ok(format) => Ok(Some(format)),
        Err(error)
            if error.fast_isinstance(vm.ctx.exceptions.memory_error)
                || error.fast_isinstance(vm.ctx.exceptions.import_error) =>
        {
            Err(error)
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_comparison_uses_values_not_float_or_bool_bytes() {
        let boolean = NativeFormat::parse("?").unwrap();
        assert!(boolean.equals(&[1], &[255]));
        assert!(!boolean.equals(&[0], &[255]));

        let half = NativeFormat::parse("e").unwrap();
        assert!(half.equals(&0_u16.to_ne_bytes(), &0x8000_u16.to_ne_bytes()));
        for bits in 0..=u16::MAX {
            let bytes = bits.to_ne_bytes();
            assert_eq!(half.equals(&bytes, &bytes), bits & 0x7fff <= 0x7c00);
        }

        for format in ["f", "@f", "Zf", "@Zf"] {
            let format = NativeFormat::parse(format).unwrap();
            let left = [0_f32.to_ne_bytes(), 1_f32.to_ne_bytes()];
            let right = [(-0_f32).to_ne_bytes(), 1_f32.to_ne_bytes()];
            assert!(format.equals(
                &left.as_flattened()[..format.size()],
                &right.as_flattened()[..format.size()]
            ));
            for bits in [0x7f80_0001_u32, 0x7fc0_0000, 0xff80_0001, 0xffc0_0000] {
                let bytes = [bits.to_ne_bytes(), 1_f32.to_ne_bytes()];
                let bytes = &bytes.as_flattened()[..format.size()];
                assert!(!format.equals(bytes, bytes));
            }
        }
        for format in ["d", "@d", "Zd", "@Zd"] {
            let format = NativeFormat::parse(format).unwrap();
            let left = [0_f64.to_ne_bytes(), 1_f64.to_ne_bytes()];
            let right = [(-0_f64).to_ne_bytes(), 1_f64.to_ne_bytes()];
            assert!(format.equals(
                &left.as_flattened()[..format.size()],
                &right.as_flattened()[..format.size()]
            ));
            for bits in [0x7ff0_0000_0000_0001_u64, 0x7ff8_0000_0000_0000] {
                let bytes = [bits.to_ne_bytes(), 1_f64.to_ne_bytes()];
                let bytes = &bytes.as_flattened()[..format.size()];
                assert!(!format.equals(bytes, bytes));
            }
        }
    }
}
