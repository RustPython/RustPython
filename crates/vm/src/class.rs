//! Utilities to define a new Python class

use crate::{
    PyPayload,
    builtins::{
        PyBaseObject, PyType, PyTypeRef,
        descriptor::{PyWrapper, SlotFunc},
    },
    function::PyMethodDef,
    object::Py,
    types::{PyComparisonOp, PyTypeFlags, PyTypeSlots, hash_not_implemented},
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

        // Don't add __new__ attribute if slot_new is inherited from object
        // (Python doesn't add __new__ to __dict__ for inherited slots)
        // Exception: object itself should have __new__ in its dict
        if let Some(slot_new) = class.slots.new.load() {
            let object_new = ctx.types.object_type.slots.new.load();
            let is_object_itself = std::ptr::eq(class, ctx.types.object_type);
            let is_inherited_from_object = !is_object_itself
                && object_new.is_some_and(|obj_new| slot_new as usize == obj_new as usize);

            if !is_inherited_from_object {
                let bound_new = Context::genesis().slot_new_wrapper.build_bound_method(
                    ctx,
                    class.to_owned().into(),
                    class,
                );
                class.set_attr(identifier!(ctx, __new__), bound_new.into());
            }
        }

        // Add slot wrappers for slots that exist and are not already in dict
        // This mirrors CPython's add_operators() in typeobject.c
        macro_rules! add_slot_wrapper {
            ($slot:ident, $name:ident, $variant:ident, $doc:expr) => {
                if let Some(func) = class.slots.$slot.load() {
                    let attr_name = identifier!(ctx, $name);
                    if !class.attributes.read().contains_key(attr_name) {
                        let wrapper = PyWrapper {
                            typ: class,
                            name: ctx.intern_str(stringify!($name)),
                            wrapped: SlotFunc::$variant(func),
                            doc: Some($doc),
                        };
                        class.set_attr(attr_name, wrapper.into_ref(ctx).into());
                    }
                }
            };
        }

        add_slot_wrapper!(
            init,
            __init__,
            Init,
            "Initialize self.  See help(type(self)) for accurate signature."
        );
        add_slot_wrapper!(repr, __repr__, Repr, "Return repr(self).");
        add_slot_wrapper!(str, __str__, Str, "Return str(self).");
        add_slot_wrapper!(iter, __iter__, Iter, "Implement iter(self).");
        add_slot_wrapper!(iternext, __next__, IterNext, "Implement next(self).");

        // __hash__ needs special handling: hash_not_implemented sets __hash__ = None
        if class.slots.hash.load().map_or(0, |h| h as usize) == hash_not_implemented as usize {
            class.set_attr(ctx.names.__hash__, ctx.none.clone().into());
        } else {
            add_slot_wrapper!(hash, __hash__, Hash, "Return hash(self).");
        }

        add_slot_wrapper!(call, __call__, Call, "Call self as a function.");
        add_slot_wrapper!(
            del,
            __del__,
            Del,
            "Called when the instance is about to be destroyed."
        );

        // Attribute access slots
        add_slot_wrapper!(
            getattro,
            __getattribute__,
            GetAttro,
            "Return getattr(self, name)."
        );
        // setattro is shared by __setattr__ and __delattr__
        if let Some(func) = class.slots.setattro.load() {
            let attr_name = identifier!(ctx, __setattr__);
            if !class.attributes.read().contains_key(attr_name) {
                let wrapper = PyWrapper {
                    typ: class,
                    name: ctx.intern_str("__setattr__"),
                    wrapped: SlotFunc::SetAttro(func),
                    doc: Some("Implement setattr(self, name, value)."),
                };
                class.set_attr(attr_name, wrapper.into_ref(ctx).into());
            }
            let attr_name = identifier!(ctx, __delattr__);
            if !class.attributes.read().contains_key(attr_name) {
                let wrapper = PyWrapper {
                    typ: class,
                    name: ctx.intern_str("__delattr__"),
                    wrapped: SlotFunc::DelAttro(func),
                    doc: Some("Implement delattr(self, name)."),
                };
                class.set_attr(attr_name, wrapper.into_ref(ctx).into());
            }
        }

        // Rich comparison slots
        macro_rules! add_richcompare_wrapper {
            ($name:ident, $op:expr, $doc:expr) => {
                if let Some(func) = class.slots.richcompare.load() {
                    let attr_name = identifier!(ctx, $name);
                    if !class.attributes.read().contains_key(attr_name) {
                        let wrapper = PyWrapper {
                            typ: class,
                            name: ctx.intern_str(stringify!($name)),
                            wrapped: SlotFunc::RichCompare(func, $op),
                            doc: Some($doc),
                        };
                        class.set_attr(attr_name, wrapper.into_ref(ctx).into());
                    }
                }
            };
        }
        add_richcompare_wrapper!(__eq__, PyComparisonOp::Eq, "Return self==value.");
        add_richcompare_wrapper!(__ne__, PyComparisonOp::Ne, "Return self!=value.");
        add_richcompare_wrapper!(__lt__, PyComparisonOp::Lt, "Return self<value.");
        add_richcompare_wrapper!(__le__, PyComparisonOp::Le, "Return self<=value.");
        add_richcompare_wrapper!(__gt__, PyComparisonOp::Gt, "Return self>value.");
        add_richcompare_wrapper!(__ge__, PyComparisonOp::Ge, "Return self>=value.");

        // Descriptor slots
        add_slot_wrapper!(
            descr_get,
            __get__,
            DescrGet,
            "Return an attribute of instance, which is of type owner."
        );
        // descr_set is shared by __set__ and __delete__
        if let Some(func) = class.slots.descr_set.load() {
            let attr_name = identifier!(ctx, __set__);
            if !class.attributes.read().contains_key(attr_name) {
                let wrapper = PyWrapper {
                    typ: class,
                    name: ctx.intern_str("__set__"),
                    wrapped: SlotFunc::DescrSet(func),
                    doc: Some("Set an attribute of instance to value."),
                };
                class.set_attr(attr_name, wrapper.into_ref(ctx).into());
            }
            let attr_name = identifier!(ctx, __delete__);
            if !class.attributes.read().contains_key(attr_name) {
                let wrapper = PyWrapper {
                    typ: class,
                    name: ctx.intern_str("__delete__"),
                    wrapped: SlotFunc::DescrDel(func),
                    doc: Some("Delete an attribute of instance."),
                };
                class.set_attr(attr_name, wrapper.into_ref(ctx).into());
            }
        }

        // Note: __new__ is handled specially at the beginning of extend_class

        // Inherit slots from base types after slots are fully initialized
        for base in class.bases.read().iter() {
            class.inherit_slots(base);
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
/// For subclasses with `#[repr(transparent)]`
/// which enables ownership transfer via `into_base()`.
pub trait PySubclass: crate::PyPayload {
    type Base: crate::PyPayload;

    /// Returns a reference to the base type's payload.
    fn as_base(&self) -> &Self::Base;
}
