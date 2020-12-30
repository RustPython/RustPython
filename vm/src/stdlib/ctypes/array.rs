use std::convert::TryInto;
use std::{fmt, mem, os::raw::*};

use num_bigint::Sign;
use widestring::{WideCString, WideChar};

use crate::builtins::memory::{try_buffer_from_object, Buffer};
use crate::builtins::slice::PySliceRef;
use crate::builtins::{PyBytes, PyInt, PyList, PyStr, PyTypeRef};
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::FuncArgs;
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
    TryFromObject, TypeProtocol,
};
use crate::slots::BufferProtocol;
use crate::VirtualMachine;

use crate::stdlib::ctypes::basics::{
    generic_get_buffer, BorrowValue as BorrowValueCData, BorrowValueMut, PyCData, PyCDataMethods,
    RawBuffer,
};
use crate::stdlib::ctypes::pointer::PyCPointer;
use crate::stdlib::ctypes::primitive::PySimpleType;

macro_rules! os_match_type {
    (
        $kind: expr,

        $(
            $($type: literal)|+ => $body: ident
        )+
    ) => {
        match $kind {
            $(
                $(
                    t if t == $type => { mem::size_of::<$body>() }
                )+
            )+
            _ => unreachable!()
        }
    }
}

macro_rules! byte_match_type {
    (
        $kind: expr,
        $byte: expr,
        $vm: expr,
        $(
            $($type: literal)|+ => $body: ident
        )+
    ) => {
        match $kind {
            $(
                $(
                    t if t == $type => {
                        let chunk: [u8; mem::size_of::<$body>()] = $byte.try_into().unwrap();
                        $vm.new_pyobj($body::from_ne_bytes(chunk))
                    }
                )+
            )+
            _ => unreachable!()
        }
    }
}

fn get_size(ty: &str) -> usize {
    os_match_type!(
        ty,
        "u" => WideChar
        "c" | "b" => c_schar
        "h" => c_short
        "H" => c_ushort
        "i" => c_int
        "I" => c_uint
        "l" => c_long
        "q" => c_longlong
        "L" => c_ulong
        "Q" => c_ulonglong
        "f" => c_float
        "d" | "g" => c_double
        "?" | "B" => c_uchar
        "P" | "z" | "Z" => c_void
    )
}

fn byte_to_pyobj(ty: &str, b: &[u8], vm: &VirtualMachine) -> PyObjectRef {
    if ty == "u" {
        vm.new_pyobj(if cfg!(windows) {
            let chunk: [u8; 2] = b.try_into().unwrap();
            u16::from_ne_bytes(chunk) as u32
        } else {
            let chunk: [u8; 4] = b.try_into().unwrap();
            u32::from_ne_bytes(chunk)
        })
    } else {
        byte_match_type!(
            ty,
            b,
            vm,
            "c" | "b" => i8
            "h" => i16
            "H" => u16
            "i" => i32
            "I" => u32
            "l" | "q" => i64
            "L" | "Q" => u64
            "f" => f32
            "d" | "g" => f64
            "?" | "B" => u8
            "P" | "z" | "Z" => usize
        )
    }
}

fn slice_adjust_size(length: isize, start: &mut isize, stop: &mut isize, step: isize) -> isize {
    if *start < 0 {
        *start += length;
        if *start < 0 {
            *start = if step < 0 { -1 } else { 0 };
        }
    } else if *start >= length {
        *start = if step < 0 { length - 1 } else { length };
    }

    if *stop < 0 {
        *stop += length;
        if *stop < 0 {
            *stop = if step < 0 { -1 } else { 0 };
        }
    } else if *stop >= length {
        *stop = if step < 0 { length - 1 } else { length };
    }

    if step < 0 {
        if *stop < *start {
            return (*start - *stop - 1) / (-step) + 1;
        }
    } else if *start < *stop {
        return (*stop - *start - 1) / step + 1;
    }

    0
}

pub fn make_array_with_lenght(
    cls: PyTypeRef,
    length: usize,
    vm: &VirtualMachine,
) -> PyResult<PyRef<PyCArray>> {
    if let Ok(ref outer_type) = vm.get_attribute(cls.as_object().to_owned(), "_type_") {
        match vm.get_attribute(outer_type.clone(), "_type_") {
            Ok(inner_type)
                if vm.issubclass(&inner_type.clone_class(), &PyCPointer::static_type())?
                    || vm
                        .issubclass(&inner_type.clone_class(), &PySimpleType::static_type())? =>
            {
                let subletter = vm
                    .get_attribute(outer_type.clone(), "_type_")?
                    .downcast_exact::<PyStr>(vm)
                    .unwrap()
                    .to_string();

                let itemsize = get_size(subletter.as_str());

                Ok(PyCArray {
                    _type_: subletter,
                    _length_: length,
                    _buffer: PyRwLock::new(RawBuffer {
                        inner: Vec::with_capacity(length * itemsize).as_mut_ptr(),
                        size: length * itemsize,
                    }),
                }
                .into_ref_with_type(vm, cls)?)
            }
            _ => Err(vm.new_type_error("_type_ must have storage info".to_string())),
        }
    } else {
        Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string()))
    }
}

#[pyclass(module = "_ctypes", name = "Array", base = "PyCData")]
pub struct PyCArray {
    _type_: String,
    _length_: usize,
    _buffer: PyRwLock<RawBuffer>,
}

impl fmt::Debug for PyCArray {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "PyCArray {{ {} {} }}",
            self._type_.as_str(),
            self._length_
        )
    }
}

impl PyValue for PyCArray {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl<'a> BorrowValueCData<'a> for PyCArray {
    fn borrow_value(&'a self) -> PyRwLockReadGuard<'a, RawBuffer> {
        self._buffer.read()
    }
}

impl<'a> BorrowValueMut<'a> for PyCArray {
    fn borrow_value_mut(&'a self) -> PyRwLockWriteGuard<'a, RawBuffer> {
        self._buffer.write()
    }
}

impl BufferProtocol for PyCArray {
    fn get_buffer(zelf: &PyRef<Self>, vm: &VirtualMachine) -> PyResult<Box<dyn Buffer>> {
        generic_get_buffer::<Self>(zelf, vm)
    }
}

// impl PyCDataMethods for PyCArray {
//     fn from_param(cls: PyTypeRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {

//     }
// }

#[pyimpl(flags(BASETYPE), with(BufferProtocol))]
impl PyCArray {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let length = match vm.get_attribute(cls.as_object().to_owned(), "_length_") {
            Ok(ref length_obj) => {
                if let Ok(length_int) = length_obj.clone().downcast_exact::<PyInt>(vm) {
                    if length_int.borrow_value().sign() == Sign::Minus {
                        Err(vm.new_value_error(
                            "The '_length_' attribute must not be negative".to_string(),
                        ))
                    } else {
                        Ok(usize::try_from_object(vm, length_obj.clone()).map_err(|_| {
                            vm.new_overflow_error(
                                "The '_length_' attribute is too large".to_string(),
                            )
                        })?)
                    }
                } else {
                    Err(vm
                        .new_type_error("The '_length_' attribute must be an integer".to_string()))
                }
            }
            Err(_) => {
                Err(vm.new_attribute_error("class must define a '_type_' _length_".to_string()))
            }
        }?;

        make_array_with_lenght(cls, length, vm)
    }

    #[pymethod(magic)]
    pub fn init(&self, value: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        // @TODO
        Ok(())
    }

    #[pyproperty(name = "value")]
    pub fn value(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;

        let res = if zelf._type_ == "u" {
            vm.new_pyobj(
                unsafe {
                    if cfg!(windows) {
                        WideCString::from_vec_with_nul_unchecked(
                            buffer
                                .obj_bytes()
                                .chunks_exact(2)
                                .map(|c| {
                                    let chunk: [u8; 2] = c.try_into().unwrap();
                                    u16::from_ne_bytes(chunk) as u32
                                })
                                .collect::<Vec<u32>>(),
                        )
                    } else {
                        WideCString::from_vec_with_nul_unchecked(
                            buffer
                                .obj_bytes()
                                .chunks(4)
                                .map(|c| {
                                    let chunk: [u8; 4] = c.try_into().unwrap();
                                    u32::from_ne_bytes(chunk)
                                })
                                .collect::<Vec<u32>>(),
                        )
                    }
                }
                .to_string()
                .map_err(|e| vm.new_runtime_error(e.to_string()))?,
            )
        } else {
            // self._type_ == "c"
            let bytes = buffer.obj_bytes();

            let bytes_inner = if let Some((last, elements)) = bytes.split_last() {
                if *last == 0 {
                    elements.to_vec()
                } else {
                    bytes.to_vec()
                }
            } else {
                vec![0; 0]
            };

            PyBytes::from(bytes_inner).into_object(vm)
        };

        Ok(res)
    }

    #[pyproperty(name = "value", setter)]
    fn set_value(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;
        let my_size = buffer.get_options().len;
        let mut bytes = buffer.obj_bytes_mut();

        if zelf._type_ == "c" {
            // bytes
            if let Ok(value) = value.clone().downcast_exact::<PyBytes>(vm) {
                let wide_bytes = value.to_vec();

                if wide_bytes.len() > my_size {
                    Err(vm.new_value_error("byte string too long".to_string()))
                } else {
                    bytes[0..wide_bytes.len()].copy_from_slice(wide_bytes.as_slice());
                    if wide_bytes.len() < my_size {
                        bytes[my_size] = 0;
                    }
                    Ok(())
                }
            } else {
                Err(vm.new_value_error(format!(
                    "bytes expected instead of {} instance",
                    value.class().name
                )))
            }
        } else {
            // unicode string zelf._type_ == "u"
            if let Ok(value) = value.clone().downcast_exact::<PyStr>(vm) {
                let wide_str =
                    unsafe { WideCString::from_str_with_nul_unchecked(value.to_string()) };

                let wide_str_len = wide_str.len();

                if wide_str.len() > my_size {
                    Err(vm.new_value_error("string too long".to_string()))
                } else {
                    let res = if cfg!(windows) {
                        wide_str
                            .into_vec()
                            .iter_mut()
                            .map(|i| u16::to_ne_bytes(*i as u16).to_vec())
                            .flatten()
                            .collect::<Vec<u8>>()
                    } else {
                        wide_str
                            .into_vec()
                            .iter_mut()
                            .map(|i| u32::to_ne_bytes(*i).to_vec())
                            .flatten()
                            .collect::<Vec<u8>>()
                    };

                    bytes[0..wide_str_len].copy_from_slice(res.as_slice());

                    Ok(())
                }
            } else {
                Err(vm.new_value_error(format!(
                    "unicode string expected instead of {} instance",
                    value.class().name
                )))
            }
        }
    }

    #[pyproperty(name = "raw")]
    fn raw(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        // zelf._type_ == "c"

        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;
        let buffer_vec = buffer.obj_bytes().to_vec();

        Ok(PyBytes::from(buffer_vec).into_object(vm))
    }

    #[pyproperty(name = "raw", setter)]
    fn set_raw(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = zelf.as_object();
        let my_buffer = try_buffer_from_object(vm, obj)?;
        let my_size = my_buffer.get_options().len;

        let new_value = try_buffer_from_object(vm, &value)?;
        let new_size = new_value.get_options().len;

        // byte string zelf._type_ == "c"
        if new_size > my_size {
            Err(vm.new_value_error("byte string too long".to_string()))
        } else {
            let mut borrowed_buffer = my_buffer.obj_bytes_mut();
            let src = new_value.obj_bytes();
            borrowed_buffer[0..new_size].copy_from_slice(&src);
            Ok(())
        }
    }

    #[pymethod(name = "__len__")]
    fn len(&self) -> usize {
        self._length_
    }

    #[pymethod(magic)]
    fn getitem(
        zelf: PyRef<Self>,
        k_or_idx: Either<isize, PySliceRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?;
        let buffer_size = buffer.get_options().len;
        let buffer_bytes = buffer.obj_bytes();
        let offset = buffer_size / zelf.len();

        let res = match k_or_idx {
            Either::A(idx) => {
                if idx < 0 || idx as usize > zelf._length_ {
                    Err(vm.new_index_error("invalid index".to_string()))
                } else {
                    let idx = idx as usize;
                    let buffer_slice = buffer_bytes[idx..idx + offset].as_ref();
                    Ok(byte_to_pyobj(zelf._type_.as_str(), buffer_slice, vm))
                }?
            }
            Either::B(slice) => {
                let step = slice
                    .step
                    .clone()
                    .map_or(Ok(1), |o| isize::try_from_object(vm, o))?;

                assert!(step != 0);
                assert!(step >= -isize::MAX);

                let mut start = slice
                    .start
                    .clone()
                    .map_or(Ok(0), |o| isize::try_from_object(vm, o))?;

                let mut stop = if slice.stop.is(&vm.ctx.none()) {
                    Err(vm.new_value_error("slice stop is required".to_string()))
                } else {
                    Ok(isize::try_from_object(vm, slice.stop.clone())?)
                }?;

                let slice_length =
                    slice_adjust_size(zelf._length_ as isize, &mut start, &mut stop, step) as usize;

                let mut obj_vec = Vec::with_capacity(slice_length);

                for i in (start as usize..stop as usize).step_by(step as usize) {
                    obj_vec.push(byte_to_pyobj(
                        zelf._type_.as_str(),
                        buffer_bytes[i..i + offset].as_ref(),
                        vm,
                    ));
                }

                PyList::from(obj_vec).into_object(vm)
            }
        };

        Ok(res)
    }

    // #[pymethod(magic)]
    // fn setitem(
    //     zelf: PyRef<Self>,
    //     k_or_idx: Either<isize, PySliceRef>,
    //     obj: PyObjectRef,
    //     vm: &VirtualMachine,
    // ) -> PyResult<()> {
    //     let buffer = try_buffer_from_object(vm, zelf.as_object())?;
    //     let buffer_size = buffer.get_options().len;
    //     let buffer_bytes = buffer.obj_bytes_mut();
    //     let offset = buffer_size / zelf.len();

    //     match k_or_idx {
    //         Either::A(idx) => {
    //             if idx < 0 || idx as usize > zelf._length_ {
    //                 Err(vm.new_index_error("invalid index".to_string()))
    //             } else {
    //                 let idx = idx as usize;
    //                 let type_obj = get_obj(zelf._type_.as_str())
    //                 if let Some(from_param) = vm.get_method(type_obj.clone(), "from_param"){
    //                     let cobj = vm.invoke(
    //                         &from_param?,
    //                         (type_obj, obj),
    //                     )?;
    //                 } else {

    //                 }
    //                 // buffer_bytes[idx..idx + offset];
    //                 Ok(())
    //             }?
    //         },
    //         Either::B(slice) => {
    //             let slice_length = slice_adjust_size(zelf._length_ as isize, &mut start, &mut stop, step) as usize;
    //         }
    //     }
    // }
}
