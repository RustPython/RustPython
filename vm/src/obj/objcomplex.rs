use super::super::pyobject::{
    PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef, PyResult, TypeProtocol,
};
use super::super::vm::VirtualMachine;
use super::objfloat;
use super::objint;
use super::objtype;
use num_complex::Complex64;
use num_traits::ToPrimitive;

pub fn init(context: &PyContext) {
    let complex_type = &context.complex_type;

    let complex_doc =
        "Create a complex number from a real part and an optional imaginary part.\n\n\
         This is equivalent to (real + imag*1j) where imag defaults to 0.";

    context.set_attr(&complex_type, "__abs__", context.new_rustfunc(complex_abs));
    context.set_attr(&complex_type, "__add__", context.new_rustfunc(complex_add));
    context.set_attr(&complex_type, "__eq__", context.new_rustfunc(complex_eq));
    context.set_attr(&complex_type, "__neg__", context.new_rustfunc(complex_neg));
    context.set_attr(&complex_type, "__new__", context.new_rustfunc(complex_new));
    context.set_attr(&complex_type, "real", context.new_property(complex_real));
    context.set_attr(&complex_type, "imag", context.new_property(complex_imag));
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

fn complex_real(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
    let Complex64 { re, im: _ } = get_value(zelf);
    Ok(vm.ctx.new_float(re))
}

fn complex_imag(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
    let Complex64 { re: _, im } = get_value(zelf);
    Ok(vm.ctx.new_float(im))
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

fn complex_eq(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, Some(vm.ctx.complex_type())), (other, None)]
    );

    let z = get_value(zelf);

    let result = if objtype::isinstance(other, &vm.ctx.complex_type()) {
        z == get_value(other)
    } else if objtype::isinstance(other, &vm.ctx.int_type()) {
        match objint::get_value(other).to_f64() {
            Some(f) => z.im == 0.0f64 && z.re == f,
            None => false,
        }
    } else if objtype::isinstance(other, &vm.ctx.float_type()) {
        z.im == 0.0 && z.re == objfloat::get_value(other)
    } else {
        return Ok(vm.ctx.not_implemented());
    };

    Ok(vm.ctx.new_bool(result))
}

fn complex_neg(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, Some(vm.ctx.complex_type()))]);
    Ok(vm.ctx.new_complex(-get_value(zelf)))
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
