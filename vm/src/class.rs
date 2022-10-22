//! Utilities to define a new Python class

use crate::{
    builtins::{PyBaseObject, PyBoundMethod, PyType, PyTypeRef},
    identifier,
    object::{Py, PyObjectPayload, PyObjectRef, PyRef},
    types::{PyTypeFlags, PyTypeSlots},
    vm::Context,
};
use rustpython_common::{lock::PyRwLock, static_cell};

pub trait StaticType {
    // Ideally, saving PyType is better than PyTypeRef
    fn static_cell() -> &'static static_cell::StaticCell<PyTypeRef>;
    fn static_metaclass() -> &'static Py<PyType> {
        PyType::static_type()
    }
    fn static_baseclass() -> &'static Py<PyType> {
        PyBaseObject::static_type()
    }
    fn static_type() -> &'static Py<PyType> {
        Self::static_cell()
            .get()
            .expect("static type has not been initialized")
    }
    fn init_manually(typ: PyTypeRef) -> &'static Py<PyType> {
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization from init_manually"));
        cell.get().unwrap()
    }
    fn init_bare_type() -> &'static Py<PyType>
    where
        Self: PyClassImpl,
    {
        let typ = Self::create_bare_type();
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization of {}", Self::NAME));
        cell.get().unwrap()
    }
    fn create_bare_type() -> PyTypeRef
    where
        Self: PyClassImpl,
    {
        PyType::new_bare_ref(
            Self::NAME,
            Self::static_baseclass().to_owned(),
            Default::default(),
            Self::make_slots(),
            Self::static_metaclass().to_owned(),
        )
        .unwrap()
    }
}

pub trait PyClassDef {
    const NAME: &'static str;
    const MODULE_NAME: Option<&'static str>;
    const TP_NAME: &'static str;
    const DOC: Option<&'static str> = None;
    const BASICSIZE: usize;
}

impl<T> PyClassDef for PyRef<T>
where
    T: PyObjectPayload + PyClassDef,
{
    const NAME: &'static str = T::NAME;
    const MODULE_NAME: Option<&'static str> = T::MODULE_NAME;
    const TP_NAME: &'static str = T::TP_NAME;
    const DOC: Option<&'static str> = T::DOC;
    const BASICSIZE: usize = T::BASICSIZE;
}

pub trait PyClassImpl: PyClassDef {
    const TP_FLAGS: PyTypeFlags = PyTypeFlags::DEFAULT;

    fn impl_extend_class(ctx: &Context, class: &'static Py<PyType>);

    fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
        #[cfg(debug_assertions)]
        {
            assert!(class.slots.flags.is_created_with_flags());
        }
        if Self::TP_FLAGS.has_feature(PyTypeFlags::HAS_DICT) {
            let __dict__ = identifier!(ctx, __dict__);
            class.set_attr(
                __dict__,
                ctx.new_getset(
                    "__dict__",
                    class,
                    crate::builtins::object::object_get_dict,
                    crate::builtins::object::object_set_dict,
                )
                .into(),
            );
        }
        Self::impl_extend_class(ctx, class);
        if let Some(doc) = Self::DOC {
            class.set_attr(identifier!(ctx, __doc__), ctx.new_str(doc).into());
        }
        if let Some(module_name) = Self::MODULE_NAME {
            class.set_attr(
                identifier!(ctx, __module__),
                ctx.new_str(module_name).into(),
            );
        }
        if class.slots.new.load().is_some() {
            let bound: PyObjectRef =
                PyBoundMethod::new_ref(class.to_owned().into(), ctx.slot_new_wrapper.clone(), ctx)
                    .into();
            class.set_attr(identifier!(ctx, __new__), bound);
        }
    }

    fn make_class(ctx: &Context) -> PyTypeRef
    where
        Self: StaticType,
    {
        (*Self::static_cell().get_or_init(|| {
            let typ = Self::create_bare_type();
            Self::extend_class(ctx, unsafe {
                // typ will be saved in static_cell
                let r: &Py<PyType> = &typ;
                &*(r as *const _)
            });
            typ
        }))
        .to_owned()
    }

    fn extend_slots(slots: &mut PyTypeSlots);

    fn make_slots() -> PyTypeSlots {
        let mut slots = PyTypeSlots {
            flags: Self::TP_FLAGS,
            name: PyRwLock::new(Some(Self::TP_NAME.to_owned())),
            basicsize: Self::BASICSIZE,
            doc: PyRwLock::new(Self::DOC),
            ..Default::default()
        };
        Self::extend_slots(&mut slots);
        slots
    }
}
