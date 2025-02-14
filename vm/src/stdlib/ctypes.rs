pub(crate) mod base;

use crate::builtins::PyModule;
use crate::{PyRef, VirtualMachine};

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _ctypes::make_module(vm);
    base::extend_module_nodes(vm, &module);
    module
}

#[pymodule]
pub(crate) mod _ctypes {
    use super::base::PyCSimple;
    use crate::builtins::PyTypeRef;
    use crate::class::StaticType;
    use crate::function::Either;
    use crate::{AsObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine};
    use crossbeam_utils::atomic::AtomicCell;
    use std::ffi::{
        c_double, c_float, c_int, c_long, c_longlong, c_schar, c_short, c_uchar, c_uint, c_ulong,
        c_ulonglong,
    };
    use std::mem;
    use widestring::WideChar;

    pub fn get_size(ty: &str) -> usize {
        match ty {
            "u" => mem::size_of::<WideChar>(),
            "c" | "b" => mem::size_of::<c_schar>(),
            "h" => mem::size_of::<c_short>(),
            "H" => mem::size_of::<c_short>(),
            "i" => mem::size_of::<c_int>(),
            "I" => mem::size_of::<c_uint>(),
            "l" => mem::size_of::<c_long>(),
            "q" => mem::size_of::<c_longlong>(),
            "L" => mem::size_of::<c_ulong>(),
            "Q" => mem::size_of::<c_ulonglong>(),
            "f" => mem::size_of::<c_float>(),
            "d" | "g" => mem::size_of::<c_double>(),
            "?" | "B" => mem::size_of::<c_uchar>(),
            "P" | "z" | "Z" => mem::size_of::<usize>(),
            _ => unreachable!(),
        }
    }

    const SIMPLE_TYPE_CHARS: &str = "cbBhHiIlLdfguzZPqQ?";

    pub fn new_simple_type(
        cls: Either<&PyObjectRef, &PyTypeRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyCSimple> {
        let cls = match cls {
            Either::A(obj) => obj,
            Either::B(typ) => typ.as_object(),
        };

        if let Ok(_type_) = cls.get_attr("_type_", vm) {
            if _type_.is_instance((&vm.ctx.types.str_type).as_ref(), vm)? {
                let tp_str = _type_.str(vm)?.to_string();

                if tp_str.len() != 1 {
                    Err(vm.new_value_error(
                        format!("class must define a '_type_' attribute which must be a string of length 1, str: {tp_str}"),
                    ))
                } else if !SIMPLE_TYPE_CHARS.contains(tp_str.as_str()) {
                    Err(vm.new_attribute_error(format!("class must define a '_type_' attribute which must be\n a single character string containing one of {SIMPLE_TYPE_CHARS}, currently it is {tp_str}.")))
                } else {
                    Ok(PyCSimple {
                        _type_: tp_str,
                        _value: AtomicCell::new(vm.ctx.none()),
                    })
                }
            } else {
                Err(vm.new_type_error("class must define a '_type_' string attribute".to_string()))
            }
        } else {
            Err(vm.new_attribute_error("class must define a '_type_' attribute".to_string()))
        }
    }

    #[pyfunction(name = "sizeof")]
    pub fn size_of(tp: Either<PyTypeRef, PyObjectRef>, vm: &VirtualMachine) -> PyResult<usize> {
        match tp {
            Either::A(type_) if type_.fast_issubclass(PyCSimple::static_type()) => {
                let zelf = new_simple_type(Either::B(&type_), vm)?;
                Ok(get_size(zelf._type_.as_str()))
            }
            Either::B(obj) if obj.has_attr("size_of_instances", vm)? => {
                let size_of_method = obj.get_attr("size_of_instances", vm)?;
                let size_of_return = size_of_method.call(vec![], vm)?;
                Ok(usize::try_from_object(vm, size_of_return)?)
            }
            _ => Err(vm.new_type_error("this type has no size".to_string())),
        }
    }

    #[pyfunction]
    fn get_errno() -> i32 {
        errno::errno().0
    }

    #[pyfunction]
    fn set_errno(value: i32) {
        errno::set_errno(errno::Errno(value));
    }
}
