use super::{
    pointer::PyCPointer,
    primitive::{new_simple_type, PyCSimple},
};
use crate::builtins::{
    self,
    memory::{try_buffer_from_object, Buffer},
    slice::PySlice,
    PyBytes, PyInt, PyList, PyRange, PyStr, PyType, PyTypeRef,
};
use crate::common::lock::{PyRwLock, PyRwLockReadGuard, PyRwLockWriteGuard};
use crate::function::OptionalArg;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyIterable, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
    TypeProtocol,
};
use crate::sliceable::SequenceIndex;
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

// TODO: make sure that this is correct wrt windows and unix wstr
fn slice_to_obj(ty: &str, b: &[u8], vm: &VirtualMachine) -> PyResult {
    if ty == "u" {
        Ok(vm.new_pyobj(if cfg!(windows) {
            u16::from_ne_bytes(
                b.try_into().map_err(|_| {
                    vm.new_value_error("buffer does not fit widestring".to_string())
                })?,
            ) as u32
        } else {
            u32::from_ne_bytes(
                b.try_into().map_err(|_| {
                    vm.new_value_error("buffer does not fit widestring".to_string())
                })?,
            )
        }))
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
                                Ok(vm.new_pyobj($body::from_ne_bytes(b.try_into().map_err(|_| vm.new_value_error(format!("buffer does not fit type '{}'",ty)))?)))
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
    let capacity = length
        .checked_mul(itemsize)
        .ok_or_else(|| vm.new_overflow_error("array too large".to_string()))?;
    // FIXME change this initialization
    Ok(PyCArray {
        _type_: new_simple_type(Either::A(&outer_type), vm)?.into_ref(vm),
        _length_: length,
        _buffer: PyRwLock::new(RawBuffer {
            inner: Vec::with_capacity(capacity).as_mut_ptr(),
            size: capacity,
        }),
    }
        .into_ref_with_type(vm, cls)?)
}
// TODO: finish implementation
fn set_array_value(
    zelf: &PyObjectRef,
    dst_buffer: &mut [u8],
    idx: usize,
    size: usize,
    obj: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let self_cls = zelf.clone_class();

    if !self_cls.issubclass(PyCData::static_type()) {
        return Err(vm.new_type_error("not a ctype instance".to_string()));
    }

    let obj_cls = obj.clone_class();

    if !obj_cls.issubclass(PyCData::static_type()) {
        // TODO: Fill here
    }

    if vm.isinstance(&obj, &self_cls)? {
        let o_buffer = try_buffer_from_object(vm, &obj)?;
        let src_buffer = o_buffer.obj_bytes();

        assert!(dst_buffer.len() == size && src_buffer.len() >= size);

        dst_buffer.copy_from_slice(&src_buffer[..size]);
    }

    if self_cls.is(PyCPointer::static_type()) && obj_cls.is(PyCArray::static_type()) {
        //TODO: Fill here
    } else {
        return Err(vm.new_type_error(format!(
            "incompatible types, {} instance instead of {} instance",
            obj_cls.name, self_cls.name
        )));
    }

    Ok(())
}

fn array_get_slice_params(
    slice: PyRef<PySlice>,
    length: &Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<(isize, isize, isize)> {
    if let Some(ref len) = length {
        let indices = vm.get_method(slice.as_object().clone(), "indices").unwrap().unwrap();
        let tuple = vm.invoke(&indices, (len.clone(),))?;

        let (start, stop, step) = (
            tuple.get_item(0, vm)?,
            tuple.get_item(1, vm)?,
            tuple.get_item(2, vm)?,
        );

        Ok((
            isize::try_from_object(vm, step)?,
            isize::try_from_object(vm, start)?,
            isize::try_from_object(vm, stop)?,
        ))
    } else {
        let step = slice.step.as_ref()
            .map_or(Ok(1), |o| isize::try_from_object(vm, o.clone()))?;

        let start = slice.start.as_ref()
            .map_or_else(|| {
                if step > 0 {
                    Ok(0)
                } else {
                    Err(vm.new_value_error("slice start is required for step < 0".to_string()))
                }
            },
                         |o| isize::try_from_object(vm, o.clone()),
            )?;

        let stop = isize::try_from_object(vm, slice.stop.clone())?;

        Ok((step, start, stop))
    }
}

fn array_slice_getitem<'a>(
    zelf: PyObjectRef,
    buffer_bytes: &'a [u8],
    slice: PyRef<PySlice>,
    size: usize,
    vm: &'a VirtualMachine,
) -> PyResult {
    let length = vm
        .get_attribute(zelf.clone(), "_length_")
        .map(|c_l| usize::try_from_object(vm, c_l))??;

    let tp = vm.get_attribute(zelf, "_type_")?.downcast::<PyStr>().unwrap().to_string();
    let _type_ = tp.as_str();
    let (step, start, stop) = array_get_slice_params(slice, &Some(vm.ctx.new_int(length)), vm)?;

    let _range = PyRange {
        start: PyInt::from(start).into_ref(vm),
        stop: PyInt::from(stop).into_ref(vm),
        step: PyInt::from(step).into_ref(vm),
    };

    let mut obj_vec = Vec::new();
    let mut offset;

    for curr in PyIterable::try_from_object(vm,_range.into_object(vm))?.iter(vm)? {
        let idx = fix_index(isize::try_from_object(vm, curr?)?, length, vm)? as usize;
        offset = idx * size;

        obj_vec.push(slice_to_obj(
            _type_,
            buffer_bytes[offset..offset + size].as_ref(),
            vm,
        )?);
    }

    Ok(vm.new_pyobj(PyList::from(obj_vec)))
}

fn array_slice_setitem(
    zelf: PyObjectRef,
    slice: PyRef<PySlice>,
    buffer_bytes: &mut [u8],
    obj: PyObjectRef,
    length: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    let (step, start, stop) = array_get_slice_params(slice, &length, vm)?;

    let slice_length = if (step < 0 && stop >= start) || (step > 0 && start >= stop) {
        0
    } else if step < 0 {
        (stop - start + 1) / step + 1
    } else {
        (stop - start - 1) / step + 1
    };

    if slice_length != vm.obj_len(&obj)? as isize {
        return Err(vm.new_value_error("Can only assign sequence of same size".to_string()));
    }

    let _range = PyRange {
        start: PyInt::from(start).into_ref(vm),
        stop: PyInt::from(stop).into_ref(vm),
        step: PyInt::from(step).into_ref(vm),
    };

    //FIXME: this function should be called for pointer too (length should be None),
    //thus, needs to make sure the size.
    //Right now I'm setting one
    let size = length.map_or(Ok(1), |v| usize::try_from_object(vm, v))?;

    for (i, curr) in PyIterable::try_from_object(vm,_range.into_object(vm))?.iter(vm)?.enumerate() {
        let idx = fix_index(isize::try_from_object(vm, curr?)?, size, vm)? as usize;
        let offset = idx * size;
        let item = obj.get_item(i, vm)?;
        let buffer_slice = &mut buffer_bytes[offset..offset + size];

        set_array_value(&zelf, buffer_slice, idx, size, item, vm)?;
    }

    Ok(())
}

#[inline(always)]
fn fix_index(index: isize, length: usize, vm: &VirtualMachine) -> PyResult<isize> {
    let index = if index < 0 {
        index + length as isize
    } else {
        index
    };

    if 0 <= index && index <= length as isize {
        Ok(index)
    } else {
        Err(vm.new_index_error("invalid index".to_string()))
    }
}

#[pyclass(module = "_ctypes", name = "PyCArrayType", base = "PyType")]
pub struct PyCArrayMeta {}

#[pyclass(
    module = "_ctypes",
    name = "Array",
    base = "PyCData",
    metaclass = "PyCArrayMeta"
)]
pub struct PyCArray {
    _type_: PyRef<PyCSimple>,
    _length_: usize,
    _buffer: PyRwLock<RawBuffer>,
}

impl fmt::Debug for PyCArrayMeta {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PyCArrayMeta",)
    }
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

impl PyValue for PyCArrayMeta {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
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

impl PyCDataMethods for PyCArrayMeta {
    fn from_param(
        zelf: PyRef<Self>,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let mut value = value;
        let cls = zelf.clone_class();

        if vm.isinstance(&value, &cls)? {
            return Ok(value);
        }

        let length = vm
            .get_attribute(zelf.as_object().clone(), "_length_")
            .map(|c_l| usize::try_from_object(vm, c_l))??;

        let value_len = vm.obj_len(&value)?;

        if let Ok(tp) = vm.get_attribute(zelf.as_object().clone(), "_type_") {
            let _type = tp.downcast::<PyCSimple>().unwrap();

            if _type._type_.as_str() == "c" {
                if vm.isinstance(&value, &vm.ctx.types.bytes_type).is_ok() {
                    if value_len > length {
                        return Err(vm.new_value_error("Invalid length".to_string()));
                    }
                    value = make_array_with_length(cls.clone(), length, vm)?.as_object().clone();
                } else if vm.isinstance(&value, &cls).is_err() {
                    return Err(
                        vm.new_type_error(format!("expected bytes, {} found", value.class().name))
                    );
                }
            } else if _type._type_.as_str() == "u" {
                if vm.isinstance(&value, &vm.ctx.types.str_type).is_ok() {
                    if value_len > length {
                        return Err(vm.new_value_error("Invalid length".to_string()));
                    }
                    value = make_array_with_length(cls.clone(), length, vm)?.as_object().clone();
                } else if vm.isinstance(&value, &cls).is_err() {
                    return Err(vm.new_type_error(format!(
                        "expected unicode string, {} found",
                        value.class().name
                    )));
                }
            }
        }

        if vm.isinstance(&value, &vm.ctx.types.tuple_type).is_ok() {
            if value_len > length {
                return Err(vm.new_runtime_error("Invalid length".to_string()));
            }
            value = make_array_with_length(cls, length, vm)?.as_object().clone();
        }

        default_from_param(zelf, value, vm)
    }
}

#[pyimpl(with(PyCDataMethods), flags(BASETYPE))]
impl PyCArrayMeta {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, vm: &VirtualMachine) -> PyResult {
        let length_obj = vm
            .get_attribute(cls.as_object().to_owned(), "_length_")
            .map_err(|_| {
                vm.new_attribute_error("class must define a '_length_' attribute".to_string())
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

        Ok(make_array_with_length(cls, length, vm)?.as_object().clone())
    }
}

#[pyimpl(flags(BASETYPE), with(BufferProtocol, PyCDataFunctions))]
impl PyCArray {
    #[pymethod(magic)]
    pub fn init(zelf: PyRef<Self>, value: OptionalArg, vm: &VirtualMachine) -> PyResult<()> {
        value.map_or(Ok(()), |value| {
            let value_length = vm.obj_len(&value)?;

            if value_length < zelf._length_ {
                let value_vec: Vec<PyObjectRef> = vm.extract_elements(&value)?;
                for (i, v) in value_vec.iter().enumerate() {
                    Self::setitem(zelf.clone(), SequenceIndex::Int(i as isize), v.clone(), vm)?
                }
                Ok(())
            } else if value_length == zelf._length_ {
                let py_slice = SequenceIndex::Slice(
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
        // TODO: make sure that this is correct
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
            // TODO: make sure that this is correct
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
    fn getitem(zelf: PyRef<Self>, k_or_idx: SequenceIndex, vm: &VirtualMachine) -> PyResult {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?;
        let buffer_size = buffer.get_options().len;
        let buffer_bytes = buffer.obj_bytes();
        let size = buffer_size / zelf._length_;

        match k_or_idx {
            SequenceIndex::Int(idx) => {
                let idx = fix_index(idx, zelf._length_, vm)? as usize;
                let offset = idx * size;
                let buffer_slice = buffer_bytes[offset..offset + size].as_ref();
                slice_to_obj(zelf._type_._type_.as_str(), buffer_slice, vm)
            }
            SequenceIndex::Slice(slice) => array_slice_getitem(
                zelf.as_object().clone(),
                &buffer_bytes[..],
                slice,
                size,
                vm,
            ),
        }
    }

    #[pymethod(magic)]
    fn setitem(
        zelf: PyRef<Self>,
        k_or_idx: SequenceIndex,
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?;
        let buffer_size = buffer.get_options().len;
        let mut buffer_bytes = buffer.obj_bytes_mut();

        let size = buffer_size / zelf._length_;

        match k_or_idx {
            SequenceIndex::Int(idx) => {
                let idx = fix_index(idx, zelf._length_, vm)? as usize;
                let offset = idx * size;
                let buffer_slice = &mut buffer_bytes[offset..offset + size];
                set_array_value(&zelf.as_object().clone(), buffer_slice, idx, size, obj, vm)
            }
            SequenceIndex::Slice(slice) => array_slice_setitem(
                zelf.as_object().clone(),
                slice,
                &mut buffer_bytes[..],
                obj,
                Some(vm.ctx.new_int(zelf._length_)),
                vm,
            ),
        }
    }
}

impl PyCDataFunctions for PyCArray {
    fn size_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<usize> {
        Ok(zelf._length_ * PyCDataFunctions::size_of_instances(zelf._type_.clone(), vm)?)
    }

    fn alignment_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<usize> {
        PyCDataFunctions::alignment_of_instances(zelf._type_.clone(), vm)
    }

    fn ref_to(zelf: PyRef<Self>, offset: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let offset = offset
            .into_option()
            .map_or(Ok(0), |o| usize::try_from_object(vm, o))?;

        if offset > zelf._length_ * get_size(zelf._type_._type_.as_str()) {
            Err(vm.new_index_error("offset out of bounds".to_string()))
        } else {
            let guard = zelf.borrow_value();
            let ref_at: *mut u8 = unsafe { guard.inner.add(offset) };

            Ok(vm.new_pyobj(ref_at as *mut _ as *mut usize as usize))
        }
    }

    fn address_of(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        Ok(vm
            .new_pyobj(unsafe { &*zelf.borrow_value().inner } as *const _ as *const usize as usize))
    }
}

impl PyCDataSequenceMethods for PyCArrayMeta {}
