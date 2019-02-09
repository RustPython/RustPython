use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objfloat;
use super::objtype;
use num_complex::Complex64;

pub fn init(context: &PyContext) {
    let complex_type = &context.complex_type;

    let complex_doc =
        "Create a complex number from a real part and an optional imaginary part.\n\n\
         This is equivalent to (real + imag*1j) where imag defaults to 0.";

    context.set_attr(&complex_type, "__abs__", context.new_rustfunc(complex_abs));
    context.set_attr(&complex_type, "__add__", context.new_rustfunc(complex_add));
    context.set_attr(&complex_type, "__new__", context.new_rustfunc(complex_new));
    context.set_attr(
        &complex_type,
        "__doc__",
        context.new_str(complex_doc.to_string()),
    );
    context.set_attr(
        &complex_type,
        "__repr__",
        context.new_rustfunc(complex_repr),
    );
    context.set_attr(
        &complex_type,
        "conjugate",
        context.new_rustfunc(complex_conjugate),
    );
}

pub fn get_value(obj: &PyObjectRef) -> Complex64 {
    if let PyObjectPayload::Complex { value } = &obj.borrow().payload {
        *value
    } else {
        panic!("Inner error getting complex");
    }
}

fn complex_new(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(cls, None)],
        optional = [(real, None), (imag, None)]
    );

    if !objtype::issubclass(cls, &vm.ctx.complex_type()) {
        return Err(vm.new_type_error(format!("{:?} is not a subtype of complex", cls)));
    }

    let real = match real {
        None => 0.0,
        Some(value) => objfloat::make_float(vm, value)?,
    };

    let imag = match imag {
        None => 0.0,
        Some(value) => objfloat::make_float(vm, value)?,
    };

    let value = Complex64::new(real, imag);

    Ok(PyObject::new(
        PyObjectPayload::Complex { value },
        cls.clone(),
    ))
}

fn complex_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);

    let Complex64 { re, im } = get_value(zelf);
    Ok(vm.ctx.new_float(re.hypot(im)))
}

fn complex_add(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(i, Some(vm.ctx.complex_type())), (i2, None)]
    );

    let v1 = get_value(i);
    if objtype::isinstance(i2, &vm.ctx.complex_type()) {
        Ok(vm.ctx.new_complex(v1 + get_value(i2)))
    } else {
        Err(vm.new_type_error(format!("Cannot add {} and {}", i.borrow(), i2.borrow())))
    }
}

fn complex_conjugate(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.complex_type()))]);

    let v1 = get_value(i);
    Ok(vm.ctx.new_complex(v1.conj()))
}

fn complex_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, Some(vm.ctx.complex_type()))]);
    let v = get_value(obj);
    let repr = if v.re == 0. {
        format!("{}j", v.im)
    } else {
        format!("({}+{}j)", v.re, v.im)
    };
    Ok(vm.new_str(repr))
}
