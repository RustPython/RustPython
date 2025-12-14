//! Utilities to define a new Python class

use crate::{
    builtins::{PyBaseObject, PyType, PyTypeRef},
    function::PyMethodDef,
    object::Py,
    types::{PyTypeFlags, PyTypeSlots, hash_not_implemented},
    vm::Context,
};
use rustpython_common::static_cell;

pub trait StaticType {
    // Ideally, saving PyType is better than PyTypeRef
    fn static_cell() -> &'static static_cell::StaticCell<PyTypeRef>;
    #[inline]
    fn static_metaclass() -> &'static Py<PyType> {
        PyType::static_type()
    }
    #[inline]
    fn static_baseclass() -> &'static Py<PyType> {
        PyBaseObject::static_type()
    }
    #[inline]
    fn static_type() -> &'static Py<PyType> {
        #[cold]
        fn fail() -> ! {
            panic!(
                "static type has not been initialized. e.g. the native types defined in different module may be used before importing library."
            );
        }
        Self::static_cell().get().unwrap_or_else(|| fail())
    }
    fn init_manually(typ: PyTypeRef) -> &'static Py<PyType> {
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization from init_manually"));
        cell.get().unwrap()
    }
    fn init_builtin_type() -> &'static Py<PyType>
    where
        Self: PyClassImpl,
    {
        let typ = Self::create_static_type();
        let cell = Self::static_cell();
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization of {}", Self::NAME));
        cell.get().unwrap()
    }
    fn create_static_type() -> PyTypeRef
    where
        Self: PyClassImpl,
    {
        PyType::new_static(
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
    const UNHASHABLE: bool = false;

    // due to restriction of rust trait system, object.__base__ is None
    // but PyBaseObject::Base will be PyBaseObject.
    type Base: PyClassDef;
}

pub trait PyClassImpl: PyClassDef {
    const TP_FLAGS: PyTypeFlags = PyTypeFlags::DEFAULT;

    fn extend_class(ctx: &Context, class: &'static Py<PyType>)
    where
        Self: Sized,
    {
        #[cfg(debug_assertions)]
        {
            assert!(class.slots.flags.is_created_with_flags());
        }

        let _ = ctx.intern_str(Self::NAME); // intern type name

        if Self::TP_FLAGS.has_feature(PyTypeFlags::HAS_DICT) {
            let __dict__ = identifier!(ctx, __dict__);
            class.set_attr(
                __dict__,
                ctx.new_static_getset(
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
            // Only set __doc__ if it doesn't already exist (e.g., as a member descriptor)
            // This matches CPython's behavior in type_dict_set_doc
            let doc_attr_name = identifier!(ctx, __doc__);
            if class.attributes.read().get(doc_attr_name).is_none() {
                class.set_attr(doc_attr_name, ctx.new_str(doc).into());
            }
        }
        if let Some(module_name) = Self::MODULE_NAME {
            class.set_attr(
                identifier!(ctx, __module__),
                ctx.new_str(module_name).into(),
            );
        }

        if class.slots.new.load().is_some() {
            let bound_new = Context::genesis().slot_new_wrapper.build_bound_method(
                ctx,
                class.to_owned().into(),
                class,
            );
            class.set_attr(identifier!(ctx, __new__), bound_new.into());
        }

        if class.slots.hash.load().map_or(0, |h| h as usize) == hash_not_implemented as usize {
            class.set_attr(ctx.names.__hash__, ctx.none.clone().into());
        }

        class.extend_methods(class.slots.methods, ctx);
    }

    fn make_class(ctx: &Context) -> PyTypeRef
    where
        Self: StaticType + Sized,
    {
        (*Self::static_cell().get_or_init(|| {
            let typ = Self::create_static_type();
            Self::extend_class(ctx, unsafe {
                // typ will be saved in static_cell
                let r: &Py<PyType> = &typ;
                let r: &'static Py<PyType> = std::mem::transmute(r);
                r
            });
            typ
        }))
        .to_owned()
    }

    fn impl_extend_class(ctx: &Context, class: &'static Py<PyType>);
    const METHOD_DEFS: &'static [PyMethodDef];
    fn extend_slots(slots: &mut PyTypeSlots);

    fn make_slots() -> PyTypeSlots {
        let mut slots = PyTypeSlots {
            flags: Self::TP_FLAGS,
            name: Self::TP_NAME,
            basicsize: Self::BASICSIZE,
            doc: Self::DOC,
            methods: Self::METHOD_DEFS,
            ..Default::default()
        };

        if Self::UNHASHABLE {
            slots.hash.store(Some(hash_not_implemented));
        }

        Self::extend_slots(&mut slots);
        slots
    }
}

/// Trait for Python subclasses that can provide a reference to their base type.
///
/// This trait is automatically implemented by the `#[pyclass]` macro when
/// `base = SomeType` is specified. It provides safe reference access to the
/// base type's payload.
///
/// For subclasses with `#[repr(transparent)]`, see also [`PySubclassTransparent`]
/// which enables ownership transfer via `into_base()`.
pub trait PySubclass: crate::PyPayload {
    type Base: crate::PyPayload;

    /// Returns a reference to the base type's payload.
    fn as_base(&self) -> &Self::Base;
}

/// Marker trait for `#[repr(transparent)]` subclasses.
///
/// This trait enables ownership transfer from `PyRef<Self>` to `PyRef<Self::Base>`
/// via the `into_base_ref()` method. Only types with identical memory layout to their
/// base type (i.e., `#[repr(transparent)]` newtypes) should implement this trait.
///
/// # Safety
///
/// Implementors must ensure:
/// - The type uses `#[repr(transparent)]` with the Base type as the only field
/// - Memory layout is identical to the Base type
pub trait PySubclassTransparent: PySubclass {}
