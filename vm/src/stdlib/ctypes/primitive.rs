use crossbeam_utils::atomic::AtomicCell;
use rustpython_common::borrow::BorrowValue;
use std::fmt;

use crate::builtins::memory::try_buffer_from_object;
use crate::builtins::PyTypeRef;
use crate::builtins::{
    int::try_to_primitive, pybool::boolval, PyByteArray, PyBytes, PyFloat, PyInt, PyNone, PyStr,
};
use crate::function::OptionalArg;
use crate::pyobject::{
    Either, IdProtocol, PyObjectRef, PyRef, PyResult, PyValue, StaticType, TryFromObject,
    TypeProtocol,
};
use crate::VirtualMachine;

use crate::stdlib::ctypes::array::PyCArray;
use crate::stdlib::ctypes::basics::{
    get_size, BorrowValueMut, PyCData, PyCDataFunctions, PyCDataMethods, PyCDataSequenceMethods,
};
use crate::stdlib::ctypes::function::PyCFuncPtr;
use crate::stdlib::ctypes::pointer::PyCPointer;

const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfguzZPqQ?";

fn set_primitive(_type_: &str, value: &PyObjectRef, vm: &VirtualMachine) -> PyResult {
    match _type_ {
        "c" => {
            if value
                .clone()
                .downcast_exact::<PyBytes>(vm)
                .map_or(false, |v| v.len() == 1)
                || value
                    .clone()
                    .downcast_exact::<PyByteArray>(vm)
                    .map_or(false, |v| v.borrow_value().len() == 1)
                || value
                    .clone()
                    .downcast_exact::<PyInt>(vm)
                    .map_or(Ok(false), |v| {
                        let n: i64 = try_to_primitive(v.borrow_value(), vm)?;
                        Ok(0 <= n && n <= 255)
                    })?
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(
                    "one character bytes, bytearray or integer expected".to_string(),
                ))
            }
        }
        "u" => {
            if let Ok(b) = value
                .clone()
                .downcast_exact::<PyStr>(vm)
                .map(|v| v.as_ref().chars().count() == 1)
            {
                if b {
                    Ok(value.clone())
                } else {
                    Err(vm.new_type_error("one character unicode string expected".to_string()))
                }
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string expected instead of {} instance",
                    value.class().name
                )))
            }
        }
        "b" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "an integer is required (got type {})",
                    value.class().name
                )))
            }
        }
        "f" | "d" | "g" => {
            if value.clone().downcast_exact::<PyFloat>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!("must be real number, not {}", value.class().name)))
            }
        }
        "?" => Ok(vm.ctx.new_bool(boolval(vm, value.clone())?)),
        "B" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok() {
                Ok(vm.new_pyobj(u8::try_from_object(vm, value.clone())?))
            } else {
                Err(vm.new_type_error(format!("int expected instead of {}", value.class().name)))
            }
        }
        "z" => {
            if value.clone().downcast_exact::<PyInt>(vm).is_ok()
                || value.clone().downcast_exact::<PyBytes>(vm).is_ok()
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "bytes or integer address expected instead of {} instance",
                    value.class().name
                )))
            }
        }
        "Z" => {
            if value.clone().downcast_exact::<PyStr>(vm).is_ok() {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error(format!(
                    "unicode string or integer address expected instead of {} instance",
                    value.class().name
                )))
            }
        }
        _ => {
            // "P"
            if value.clone().downcast_exact::<PyInt>(vm).is_ok()
                || value.clone().downcast_exact::<PyNone>(vm).is_ok()
            {
                Ok(value.clone())
            } else {
                Err(vm.new_type_error("cannot be converted to pointer".to_string()))
            }
        }
    }
}

fn generic_xxx_p_from_param(
    cls: &PyTypeRef,
    value: &PyObjectRef,
    type_str: &str,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    if vm.is_none(value) {
        return Ok(vm.ctx.none());
    }

    if vm.isinstance(value, &vm.ctx.types.str_type)?
        || vm.isinstance(value, &vm.ctx.types.bytes_type)?
    {
        Ok(PySimpleType {
            _type_: type_str.to_string(),
            value: AtomicCell::new(value.clone()),
            __abstract__: cls.is(PySimpleType::static_type()),
        }
        .into_object(vm))
    } else if vm.isinstance(value, PySimpleType::static_type())?
        && (type_str == "z" || type_str == "Z" || type_str == "P")
    {
        Ok(value.clone())
    } else {
        // @TODO: better message
        Err(vm.new_type_error("wrong type".to_string()))
    }
}

fn from_param_char_p(
    cls: &PyTypeRef,
    value: &PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    let _type_ = vm
        .get_attribute(value.clone(), "_type_")?
        .downcast_exact::<PyStr>(vm)
        .unwrap();
    let type_str = _type_.as_ref();

    let res = generic_xxx_p_from_param(cls, value, type_str, vm)?;

    if !vm.is_none(&res) {
        Ok(res)
    } else if (vm.isinstance(value, PyCArray::static_type())?
        || vm.isinstance(value, PyCPointer::static_type())?)
        && (type_str == "z" || type_str == "Z" || type_str == "P")
    {
        Ok(value.clone())
    } else {
        // @TODO: Make sure of what goes here
        Err(vm.new_type_error("some error".to_string()))
    }
}

fn from_param_void_p(
    cls: &PyTypeRef,
    value: &PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<PyObjectRef> {
    let _type_ = vm
        .get_attribute(value.clone(), "_type_")?
        .downcast_exact::<PyStr>(vm)
        .unwrap();
    let type_str = _type_.as_ref();

    let res = generic_xxx_p_from_param(cls, value, type_str, vm)?;

    if !vm.is_none(&res) {
        Ok(res)
    } else if vm.isinstance(value, PyCArray::static_type())? {
        Ok(value.clone())
    } else if vm.isinstance(value, PyCFuncPtr::static_type())?
        || vm.isinstance(value, PyCPointer::static_type())?
    {
        // @TODO: Is there a better way of doing this?
        if let Some(from_address) = vm.get_method(cls.as_object().clone(), "from_address") {
            if let Ok(cdata) = value.clone().downcast::<PyCData>() {
                let buffer_guard = cdata.borrow_value_mut();
                let addr = buffer_guard.inner as usize;

                Ok(vm.invoke(&from_address?, (cls.clone_class(), addr))?)
            } else {
                // @TODO: Make sure of what goes here
                Err(vm.new_type_error("value should be an instance of _CData".to_string()))
            }
        } else {
            // @TODO: Make sure of what goes here
            Err(vm.new_attribute_error("class has no from_address method".to_string()))
        }
    } else if vm.isinstance(value, &vm.ctx.types.int_type)? {
        Ok(PySimpleType {
            _type_: type_str.to_string(),
            value: AtomicCell::new(value.clone()),
            __abstract__: cls.is(PySimpleType::static_type()),
        }
        .into_object(vm))
    } else {
        // @TODO: Make sure of what goes here
        Err(vm.new_type_error("some error".to_string()))
    }
}

pub fn new_simple_type(
    cls: Either<&PyObjectRef, &PyTypeRef>,
    vm: &VirtualMachine,
) -> PyResult<PySimpleType> {
    let cls = match cls {
        Either::A(obj) => obj,
        Either::B(typ) => typ.as_object(),
    };

    let is_abstract = cls.is(PySimpleType::static_type());

    if is_abstract {
        return Err(vm.new_type_error("abstract class".to_string()));
    }
    match vm.get_attribute(cls.clone(), "_type_") {
        Ok(_type_) if vm.isinstance(&_type_, &vm.ctx.types.str_type)? => {
            let tp_str = _type_.downcast_exact::<PyStr>(vm).unwrap().to_string();

            if tp_str.len() != 1 {
                Err(vm.new_value_error(
                    "class must define a '_type_' attribute which must be a string of length 1"
                        .to_string(),
                ))
            } else if !SIMPLE_TYPE_CHARS.contains(tp_str.as_str()) {
                Err(vm.new_attribute_error(format!("class must define a '_type_' attribute which must be a single character string containing one of {}.",SIMPLE_TYPE_CHARS)))
            } else {
                Ok(PySimpleType {
                    _type_: tp_str,
                    value: AtomicCell::new(vm.ctx.none()),
                    __abstract__: is_abstract,
                })
            }
        }
        Ok(_) => {
            Err(vm.new_type_error("class must define a '_type_' string attribute".to_string()))
        }
        _ => Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string())),
    }
}

#[pyclass(module = "_ctypes", name = "_SimpleCData", base = "PyCData")]
pub struct PySimpleType {
    pub _type_: String,
    value: AtomicCell<PyObjectRef>,
    __abstract__: bool,
}

impl fmt::Debug for PySimpleType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let value = unsafe { (*self.value.as_ptr()).to_string() };

        write!(
            f,
            "PySimpleType {{
            _type_: {},
            value: {},
        }}",
            self._type_.as_str(),
            value
        )
    }
}

impl PyValue for PySimpleType {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

impl PyCDataMethods for PySimpleType {
    // From PyCSimpleType_Type PyCSimpleType_methods
    fn from_param(
        zelf: PyRef<Self>,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let cls = zelf.clone_class();
        if cls.is(PySimpleType::static_type()) {
            Err(vm.new_type_error("abstract class".to_string()))
        } else if vm.isinstance(&value, &cls)? {
            Ok(value)
        } else {
            match vm.get_attribute(zelf.as_object().clone(), "_type_") {
                Ok(tp_obj) if vm.isinstance(&tp_obj, &vm.ctx.types.str_type)? => {
                    let _type_ = tp_obj.downcast_exact::<PyStr>(vm).unwrap();
                    let tp_str = _type_.as_ref();

                    match tp_str {
                        "z" | "Z" => from_param_char_p(&cls, &value, vm),
                        "P" => from_param_void_p(&cls, &value, vm),
                        _ => {
                            match new_simple_type(Either::B(&cls), vm) {
                                Ok(obj) => Ok(obj.into_object(vm)),
                                Err(e)
                                    if vm.isinstance(
                                        &e.clone().into_object(),
                                        &vm.ctx.exceptions.type_error,
                                    )? || vm.isinstance(
                                        &e.clone().into_object(),
                                        &vm.ctx.exceptions.value_error,
                                    )? =>
                                {
                                    if let Some(my_base) = cls.base.clone() {
                                        if let Some(from_param) =
                                            vm.get_method(my_base.as_object().clone(), "from_param")
                                        {
                                            Ok(vm.invoke(
                                                &from_param?,
                                                (my_base.clone_class(), value),
                                            )?)
                                        } else {
                                            // @TODO: Make sure of what goes here
                                            Err(vm.new_attribute_error(
                                                "class has no from_param method".to_string(),
                                            ))
                                        }
                                    } else {
                                        // @TODO: This should be unreachable
                                        Err(vm.new_type_error("class has no base".to_string()))
                                    }
                                }
                                Err(e) => Err(e),
                            }
                        }
                    }
                }
                // @TODO: Sanity check, this should be unreachable
                _ => {
                    Err(vm
                        .new_attribute_error("class must define a '_type_' attribute".to_string()))
                }
            }
        }
    }
}

#[pyimpl(with(PyCDataFunctions, PyCDataMethods), flags(BASETYPE))]
impl PySimpleType {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, _: OptionalArg, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        new_simple_type(Either::B(&cls), vm)?.into_ref_with_type(vm, cls)
    }

    #[pymethod(magic)]
    pub fn init(&self, value: OptionalArg, vm: &VirtualMachine) -> PyResult<()> {
        match value.into_option() {
            Some(ref v) => {
                let content = set_primitive(self._type_.as_str(), v, vm)?;
                self.value.store(content);
            }
            _ => {
                self.value.store(match self._type_.as_str() {
                    "c" | "u" => vm.ctx.new_bytes(vec![0]),
                    "b" | "B" | "h" | "H" | "i" | "I" | "l" | "q" | "L" | "Q" => vm.ctx.new_int(0),
                    "f" | "d" | "g" => vm.ctx.new_float(0.0),
                    "?" => vm.ctx.new_bool(false),
                    _ => vm.ctx.none(), // "z" | "Z" | "P"
                });
            }
        }
        Ok(())
    }

    #[pyproperty(name = "value")]
    pub fn value(&self) -> PyObjectRef {
        unsafe { (*self.value.as_ptr()).clone() }
    }

    #[pyproperty(name = "value", setter)]
    fn set_value(&self, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        let content = set_primitive(self._type_.as_str(), &value, vm)?;
        self.value.store(content);
        Ok(())
    }

    // From Simple_Type Simple_methods
    #[pymethod(magic)]
    pub fn ctypes_from_outparam(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        if let Some(base) = zelf.class().base.clone() {
            if vm.bool_eq(&base.as_object(), PySimpleType::static_type().as_object())? {
                return Ok(zelf.as_object().clone());
            }
        }
        Ok(zelf.value())
    }

    // Simple_repr
    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "{}({})",
            zelf.class().name,
            vm.to_repr(&zelf.value())?.to_string()
        ))
    }

    // Simple_as_number
    #[pymethod(magic)]
    fn bool(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let buffer = try_buffer_from_object(vm, zelf.as_object())?
            .obj_bytes()
            .to_vec();

        Ok(vm.new_pyobj(buffer != vec![0]))
    }
}

impl PyCDataFunctions for PySimpleType {
    fn size_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(vm.new_pyobj(get_size(zelf._type_.as_str())))
    }

    fn alignment_of_instances(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Self::size_of_instances(zelf, vm)
    }

    fn ref_to(
        zelf: PyRef<Self>,
        offset: OptionalArg,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        Ok(vm.new_pyobj(zelf.value.as_ptr() as *mut _ as *mut usize as usize))
    }

    fn address_of(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(vm.new_pyobj(unsafe { &*zelf.value.as_ptr() } as *const _ as *const usize as usize))
    }
}

impl PyCDataSequenceMethods for PySimpleType {}
