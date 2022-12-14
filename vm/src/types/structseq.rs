use crate::{
    builtins::{PyTuple, PyTupleRef, PyType},
    class::{PyClassImpl, StaticType},
    vm::Context,
    AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
};

#[pyclass]
pub trait PyStructSequence: StaticType + PyClassImpl + Sized + 'static {
    const FIELD_NAMES: &'static [&'static str];

    fn into_tuple(self, vm: &VirtualMachine) -> PyTuple;

    fn into_struct_sequence(self, vm: &VirtualMachine) -> PyTupleRef {
        self.into_tuple(vm)
            .into_ref_with_type(vm, Self::static_type().to_owned())
            .unwrap()
    }

    fn try_elements_from<const FIELD_LEN: usize>(
        obj: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<[PyObjectRef; FIELD_LEN]> {
        let typ = Self::static_type();
        // if !obj.fast_isinstance(typ) {
        //     return Err(vm.new_type_error(format!(
        //         "{} is not a subclass of {}",
        //         obj.class().name(),
        //         typ.name(),
        //     )));
        // }
        let seq: Vec<PyObjectRef> = obj.try_into_value(vm)?;
        let seq: [PyObjectRef; FIELD_LEN] = seq.try_into().map_err(|_| {
            vm.new_type_error(format!(
                "{} takes a sequence of length {}",
                typ.name(),
                FIELD_LEN
            ))
        })?;
        Ok(seq)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyResult<String> {
        let format_field = |(value, name): (&PyObjectRef, _)| {
            let s = value.repr(vm)?;
            Ok(format!("{name}={s}"))
        };
        let (body, suffix) = if let Some(_guard) =
            rustpython_vm::recursion::ReprGuard::enter(vm, zelf.as_object())
        {
            if Self::FIELD_NAMES.len() == 1 {
                let value = zelf.first().unwrap();
                let formatted = format_field((value, Self::FIELD_NAMES[0]))?;
                (formatted, ",")
            } else {
                let fields: PyResult<Vec<_>> = zelf
                    .iter()
                    .zip(Self::FIELD_NAMES.iter().copied())
                    .map(format_field)
                    .collect();
                (fields?.join(", "), "")
            }
        } else {
            (String::new(), "...")
        };
        Ok(format!("{}({}{})", Self::TP_NAME, body, suffix))
    }

    #[pymethod(magic)]
    fn reduce(zelf: PyRef<PyTuple>, vm: &VirtualMachine) -> PyTupleRef {
        vm.new_tuple((zelf.class().to_owned(), (vm.ctx.new_tuple(zelf.to_vec()),)))
    }

    #[extend_class]
    fn extend_pyclass(ctx: &Context, class: &'static Py<PyType>) {
        for (i, &name) in Self::FIELD_NAMES.iter().enumerate() {
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
                Self::FIELD_NAMES
                    .iter()
                    .map(|&name| ctx.new_str(name).into())
                    .collect::<Vec<_>>(),
            )
            .into(),
        );
    }
}
