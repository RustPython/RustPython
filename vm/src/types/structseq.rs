use crate::{
    AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyBaseExceptionRef, PyTuple, PyTupleRef, PyType},
    class::{PyClassImpl, StaticType},
    vm::Context,
};

#[pyclass]
pub trait PyStructSequence: StaticType + PyClassImpl + Sized + 'static {
    const REQUIRED_FIELD_NAMES: &'static [&'static str];
    const OPTIONAL_FIELD_NAMES: &'static [&'static str];

    fn into_tuple(self, vm: &VirtualMachine) -> PyTuple;

    fn into_struct_sequence(self, vm: &VirtualMachine) -> PyTupleRef {
        self.into_tuple(vm)
            .into_ref_with_type(vm, Self::static_type().to_owned())
            .unwrap()
    }

    fn try_elements_from(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<Vec<PyObjectRef>> {
        #[cold]
        fn sequence_length_error(
            name: &str,
            len: usize,
            vm: &VirtualMachine,
        ) -> PyBaseExceptionRef {
            vm.new_type_error(format!("{name} takes a sequence of length {len}"))
        }

        let typ = Self::static_type();
        // if !obj.fast_isinstance(typ) {
        //     return Err(vm.new_type_error(format!(
        //         "{} is not a subclass of {}",
        //         obj.class().name(),
        //         typ.name(),
        //     )));
        // }
        let seq: Vec<PyObjectRef> = obj.try_into_value(vm)?;
        if seq.len() < Self::REQUIRED_FIELD_NAMES.len() {
            return Err(sequence_length_error(
                &typ.name(),
                Self::REQUIRED_FIELD_NAMES.len(),
                vm,
            ));
        }
        Ok(seq)
    }

    #[pymethod]
    fn __repr__(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyResult<String> {
        let format_field = |(value, name): (&PyObjectRef, _)| {
            let s = value.repr(vm)?;
            Ok(format!("{name}={s}"))
        };
        let (body, suffix) = if let Some(_guard) =
            rustpython_vm::recursion::ReprGuard::enter(vm, zelf.as_object())
        {
            if Self::REQUIRED_FIELD_NAMES.len() == 1 {
                let value = zelf.first().unwrap();
                let formatted = format_field((value, Self::REQUIRED_FIELD_NAMES[0]))?;
                (formatted, ",")
            } else {
                let fields: PyResult<Vec<_>> = zelf
                    .iter()
                    .zip(Self::REQUIRED_FIELD_NAMES.iter().copied())
                    .map(format_field)
                    .collect();
                (fields?.join(", "), "")
            }
        } else {
            (String::new(), "...")
        };
        Ok(format!("{}({}{})", Self::TP_NAME, body, suffix))
    }

    #[pymethod]
    fn __reduce__(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((zelf.class().to_owned(), (vm.ctx.new_tuple(zelf.to_vec()),)))
    }

    #[extend_class]
    fn extend_pyclass(ctx: &Context, class: &'static Py<PyType>) {
        for (i, &name) in Self::REQUIRED_FIELD_NAMES.iter().enumerate() {
            // cast i to a u8 so there's less to store in the getter closure.
            // Hopefully there's not struct sequences with >=256 elements :P
            let i = i as u8;
            class.set_attr(
                ctx.intern_str(name),
                ctx.new_readonly_getset(name, class, move |zelf: &PyTuple| {
                    zelf.fast_getitem(i.into())
                })
                .into(),
            );
        }

        class.set_attr(
            identifier!(ctx, __match_args__),
            ctx.new_tuple(
                Self::REQUIRED_FIELD_NAMES
                    .iter()
                    .map(|&name| ctx.new_str(name).into())
                    .collect::<Vec<_>>(),
            )
            .into(),
        );
    }
}
