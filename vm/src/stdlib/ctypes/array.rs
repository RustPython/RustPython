use super::{
    pointer::PyCPointer,
    primitive::{new_simple_type, PySimpleType},
};
use crate::builtins::{
    self,
    memory::{try_buffer_from_object, Buffer},
    slice::PySlice,
    PyBytes, PyInt, PyList, PyStr, PyTypeRef,
};
use crate::common::borrow::BorrowedValueMut;
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject, TypeProtocol,
};
use crate::slots::BufferProtocol;
use crate::stdlib::ctypes::basics::{
    default_from_param, generic_get_buffer, get_size, BorrowValue as BorrowValueCData,
    BorrowValueMut, PyCData, PyCDataFunctions, PyCDataMethods, PyCDataSequenceMethods, RawBuffer,
};
use crate::utils::Either;
use crate::VirtualMachine;
use num_traits::Signed;
use std::convert::TryInto;
use std::fmt;
use widestring::WideCString;

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
        macro_rules! byte_match_type {
            (
                $(
                    $($type: literal)|+ => $body: ident
                )+
            ) => {
                match ty {
                    $(
                        $(
                            t if t == $type => {
                                let chunk: [u8; std::mem::size_of::<$body>()] = b.try_into().unwrap();
                                vm.new_pyobj($body::from_ne_bytes(chunk))
                            }
                        )+
                    )+
                    _ => unreachable!()
                }
            }
        }
        byte_match_type!(
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

pub fn make_array_with_length(
    cls: PyTypeRef,
    length: usize,
    vm: &VirtualMachine,
) -> PyResult<PyRef<PyCArray>> {
    let outer_type = cls.get_attr("_type_").ok_or_else(|| {
        vm.new_attribute_error("class must define a '_type_' attribute".to_string())
    })?;
    let length = length as usize;
    let _type_ = vm
        .get_attribute(outer_type.clone(), "_type_")
        .map_err(|_| vm.new_type_error("_type_ must have storage info".to_string()))?;
    let itemsize = get_size(_type_.downcast::<PyStr>().unwrap().to_string().as_str());
    let _ = length
        .checked_mul(itemsize)
        .ok_or_else(|| vm.new_overflow_error("Array size too big".to_string()))?;
    Ok(PyCArray {
        _type_: new_simple_type(Either::A(&outer_type), vm)?.into_ref(vm),
        _length_: length,
        _buffer: PyRwLock::new(RawBuffer {
            inner: Vec::with_capacity(length * itemsize).as_mut_ptr(),
            size: length * itemsize,
        }),
    }
    .into_ref_with_type(vm, cls)?)
}

fn set_array_value(
    zelf: &PyRef<PyCArray>,
    dst_buffer: &mut BorrowedValueMut<[u8]>,
    idx: usize,
    offset: usize,
    obj: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if !obj.class().issubclass(PyCData::static_type()) {
        let value = PyCDataMethods::from_param(zelf._type_.clone(), obj, vm)?;

        let v_buffer = try_buffer_from_object(vm, &value)?;
        let v_buffer_bytes = v_buffer.obj_bytes_mut();

        dst_buffer[idx..idx + offset].copy_from_slice(&v_buffer_bytes[..]);
    } else if vm.isinstance(&obj, &zelf._type_.clone_class())? {
        let o_buffer = try_buffer_from_object(vm, &obj)?;
        let src_buffer = o_buffer.obj_bytes_mut();

        dst_buffer[idx..idx + offset].copy_from_slice(&src_buffer[idx..idx + offset]);
    } else if vm.isinstance(zelf._type_.as_object(), PyCPointer::static_type())?
        && vm.isinstance(&obj, PyCArray::static_type())?
    {
        //@TODO: Fill here once CPointer is done
    } else {
        return Err(vm.new_type_error(format!(
            "incompatible types, {} instance instead of {} instance",
            obj.class().name,
            zelf.class().name
        )));
    }
    Ok(())
}

fn array_get_slice_inner(
    slice: PyRef<PySlice>,
    vm: &VirtualMachine,
) -> PyResult<(isize, isize, isize)> {
    let step = slice
        .step
        .as_ref()
        .map(|o| isize::try_from_object(vm, o.clone()))
        .transpose()? // FIXME: unnessessary clone
        .unwrap_or(1);

    assert!(step != 0);
    assert!(step >= -isize::MAX);

    let start = slice
        .start
        .clone() // FIXME: unnessessary clone
        .map_or(Ok(0), |o| isize::try_from_object(vm, o))?;

    if vm.is_none(&slice.stop) {
        return Err(vm.new_value_error("slice stop is required".to_string()));
    }
    // FIXME: unnessessary clone
    let stop = isize::try_from_object(vm, slice.stop.clone())?;

    Ok((step, start, stop))
}

#[pyclass(module = "_ctypes", name = "Array", base = "PyCData")]
pub struct PyCArray {
    _type_: PyRef<PySimpleType>,
    _length_: usize,
    _buffer: PyRwLock<RawBuffer>,
}

impl fmt::Debug for PyCArray {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "PyCArray {{ {} {} }}",
            self._type_._type_.as_str(),
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

impl PyCDataMethods for PyCArray {
    fn from_param(
        zelf: PyRef<Self>,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        if vm.isinstance(&value, PyCArray::static_type())? {
            return Ok(value);
        }

        if vm.obj_len(&value)? > zelf._length_ {
            return Err(vm.new_value_error("value has size greater than the array".to_string()));
        }

        if zelf._type_._type_.as_str() == "c"
            && value.clone().downcast_exact::<PyBytes>(vm).is_err()
        {
            return Err(vm.new_value_error(format!("expected bytes, {} found", value.class().name)));
        }

        if zelf._type_._type_.as_str() == "u" && value.clone().downcast_exact::<PyStr>(vm).is_err()
        {
            return Err(vm.new_value_error(format!(
                "expected unicode string, {} found",
                value.class().name
            )));
        }

        if !vm.isinstance(&value, &vm.ctx.types.tuple_type)? {
            //@TODO: make sure what goes here
            return Err(vm.new_type_error("Invalid type".to_string()));
        }

        PyCArray::init(zelf.clone(), OptionalArg::Present(value), vm)?;

        default_from_param(zelf.clone_class(), zelf.as_object().clone(), vm)
    }
}

#[pyimpl(
    flags(BASETYPE),
    with(BufferProtocol, PyCDataFunctions, PyCDataMethods)
)]
impl PyCArray {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        let length_obj = vm
            .get_attribute(cls.as_object().to_owned(), "_length_")
            .map_err(|_| {
                vm.new_attribute_error("class must define a '_type_' _length_".to_string())
            })?;
        let length_int = length_obj.downcast_exact::<PyInt>(vm).map_err(|_| {
            vm.new_type_error("The '_length_' attribute must be an integer".to_string())
        })?;
        let length: usize = if length_int.as_bigint().is_negative() {
            Err(vm.new_value_error("The '_length_' attribute must not be negative".to_string()))
        } else {
            Ok(
                builtins::int::try_to_primitive(length_int.as_bigint(), vm).map_err(|_| {
                    vm.new_overflow_error("The '_length_' attribute is too large".to_owned())
                })?,
            )
        }?;

        make_array_with_length(cls, length, vm)
    }

    #[pymethod(magic)]
    pub fn init(zelf: PyRef<Self>, value: OptionalArg, vm: &VirtualMachine) -> PyResult<()> {
        value.map_or(Ok(()), |value| {
            let value_length = vm.obj_len(&value)?;

            if value_length < zelf._length_ {
                let value_vec: Vec<PyObjectRef> = vm.extract_elements(&value)?;
                for (i, v) in value_vec.iter().enumerate() {
                    Self::setitem(zelf.clone(), Either::A(i as isize), v.clone(), vm)?
                }
                Ok(())
            } else if value_length == zelf._length_ {
                let py_slice = Either::B(
                    PySlice {
                        start: Some(vm.new_pyobj(0)),
                        stop: vm.new_pyobj(zelf._length_),
                        step: None,
                    }
                    .into_ref(vm),
                );

                Self::setitem(zelf, py_slice, value, vm)
            } else {
                Err(vm.new_value_error("value has size greater than the array".to_string()))
            }
        })
    }

    #[pyproperty]
    pub fn value(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;

        let res = if zelf._type_._type_ == "u" {
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
                Vec::new()
            };

            PyBytes::from(bytes_inner).into_object(vm)
        };

        Ok(res)
    }

    #[pyproperty(setter)]
    fn set_value(zelf: PyRef<Self>, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;
        let my_size = buffer.get_options().len;
        let mut bytes = buffer.obj_bytes_mut();

        if zelf._type_._type_ == "c" {
            // bytes
            let value = value.downcast_exact::<PyBytes>(vm).map_err(|value| {
                vm.new_value_error(format!(
                    "bytes expected instead of {} instance",
                    value.class().name
                ))
            })?;
            let wide_bytes = value.to_vec();

            if wide_bytes.len() > my_size {
                return Err(vm.new_value_error("byte string too long".to_string()));
            }

            bytes[..wide_bytes.len()].copy_from_slice(wide_bytes.as_slice());
            if wide_bytes.len() < my_size {
                bytes[my_size] = 0;
            }
        } else {
            // unicode string zelf._type_ == "u"
            let value = value.downcast_exact::<PyStr>(vm).map_err(|value| {
                vm.new_value_error(format!(
                    "unicode string expected instead of {} instance",
                    value.class().name
                ))
            })?;
            let wide_str = unsafe { WideCString::from_str_with_nul_unchecked(value.to_string()) };

            let wide_str_len = wide_str.len();

            if wide_str.len() > my_size {
                return Err(vm.new_value_error("string too long".to_string()));
            }
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

            bytes[..wide_str_len].copy_from_slice(res.as_slice());
        }

        Ok(())
    }

    #[pyproperty]
    fn raw(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyBytes> {
        // zelf._type_ == "c"

        let obj = zelf.as_object();
        let buffer = try_buffer_from_object(vm, obj)?;
        let buffer_vec = buffer.obj_bytes().to_vec();

        Ok(PyBytes::from(buffer_vec))
    }

    #[pyproperty(setter)]
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
            borrowed_buffer[..new_size].copy_from_slice(&src);
            Ok(())
        }
    }

    #[pymethod(magic)]
    fn len(&self) -> usize {
        self._length_
    }

    #[pymethod(magic)]
    fn getitem(
        zelf: PyRef<Self>,
        k_or_idx: Either<isize, PyRef<PySlice>>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?;
        let buffer_size = buffer.get_options().len;
        let buffer_bytes = buffer.obj_bytes();
        let offset = buffer_size / zelf.len();

        let res = match k_or_idx {
            Either::A(idx) => {
                if idx < 0 {
                    Err(vm.new_index_error("invalid index".to_string()))
                } else if idx as usize > zelf._length_ {
                    Err(vm.new_index_error("index out of bounds".to_string()))
                } else {
                    let idx = idx as usize;
                    let buffer_slice = buffer_bytes[idx..idx + offset].as_ref();
                    Ok(byte_to_pyobj(zelf._type_._type_.as_str(), buffer_slice, vm))
                }?
            }
            Either::B(slice) => {
                let (step, mut start, mut stop) = array_get_slice_inner(slice, vm)?;

                let slice_length =
                    slice_adjust_size(zelf._length_ as isize, &mut start, &mut stop, step) as usize;

                let mut obj_vec = Vec::with_capacity(slice_length);

                for i in (start as usize..stop as usize).step_by(step as usize) {
                    obj_vec.push(byte_to_pyobj(
                        zelf._type_._type_.as_str(),
                        buffer_bytes[i..i + offset].as_ref(),
                        vm,
                    ));
                }

                PyList::from(obj_vec).into_object(vm)
            }
        };

        Ok(res)
    }

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        k_or_idx: Either<isize, PyRef<PySlice>>,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?;
        let buffer_size = buffer.get_options().len;
        let mut buffer_bytes = buffer.obj_bytes_mut();

        let offset = buffer_size / zelf.len();

        match k_or_idx {
            Either::A(idx) => {
                if idx < 0 {
                    Err(vm.new_index_error("invalid index".to_string()))
                } else if idx as usize > zelf._length_ {
                    Err(vm.new_index_error("index out of bounds".to_string()))
                } else {
                    set_array_value(&zelf, &mut buffer_bytes, idx as usize, offset, obj, vm)
                }
            }
            Either::B(slice) => {
                let (step, mut start, mut stop) = array_get_slice_inner(slice, vm)?;

                let slice_length =
                    slice_adjust_size(zelf._length_ as isize, &mut start, &mut stop, step) as usize;

                let values: Vec<PyObjectRef> = vm.extract_elements(&obj)?;

                if values.len() != slice_length {
                    Err(vm.new_value_error("can only assign sequence of same size".to_string()))
                } else {
                    let mut cur = start as usize;

                    for v in values {
                        set_array_value(&zelf, &mut buffer_bytes, cur, offset, v, vm)?;
                        cur += step as usize;
                    }
                    Ok(())
                }
            }
        }
    }
}

impl PyCDataFunctions for PyCArray {
    fn size_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(vm.new_pyobj(
            zelf._length_
                * usize::try_from_object(
                    vm,
                    PyCDataFunctions::size_of_instances(zelf._type_.clone(), vm)?,
                )?,
        ))
    }

    fn alignment_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        PyCDataFunctions::alignment_of_instances(zelf._type_.clone(), vm)
    }

    fn ref_to(zelf: PyRef<Self>, offset: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let off_set = offset
            .into_option()
            .map_or(Ok(0), |o| usize::try_from_object(vm, o))?;

        if off_set > zelf._length_ * get_size(zelf._type_._type_.as_str()) {
            Err(vm.new_index_error("offset out of bounds".to_string()))
        } else {
            let guard = zelf.borrow_value();
            let ref_at: *mut u8 = unsafe { guard.inner.add(off_set) };

            Ok(vm.new_pyobj(ref_at as *mut _ as *mut usize as usize))
        }
    }

    fn address_of(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .new_pyobj(unsafe { &*zelf.borrow_value().inner } as *const _ as *const usize as usize))
    }
}

impl PyCDataSequenceMethods for PyCArray {}
