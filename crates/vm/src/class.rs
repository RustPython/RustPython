//! Utilities to define a new Python class

use crate::{
    AsObject, PyPayload,
    builtins::{PyBaseObject, PyType, PyTypeRef, descriptor::PyWrapper},
    function::{ItemDoc, PyMethodDef, plain_doc},
    object::Py,
    types::{PyTypeFlags, PyTypeSlots, SLOT_DEFS, fn_addr, hash_not_implemented},
    vm::{Context, VirtualMachine},
};
use rustpython_common::static_cell;

/// Immutable recipes captured before slot inheritance. Replaying a native
/// namespace must not install wrappers for slots inherited later.
pub struct NativeOperators {
    wrappers: Vec<NativeOperator>,
    new: bool,
    unhashable: bool,
}

struct NativeOperator {
    name: &'static str,
    wrapped: crate::builtins::descriptor::SlotFunc,
    doc: &'static str,
    plain_off: u32,
    plain_len: u32,
}

impl NativeOperators {
    fn capture<T: PyClassDef>(class: &Py<PyType>, ctx: &Context) -> Self {
        let unhashable =
            class.slots().hash.load().is_some_and(|h| {
                fn_addr(h) == fn_addr(hash_not_implemented as crate::types::HashFunc)
            });
        let wrappers = SLOT_DEFS
            .iter()
            .filter_map(|def| {
                // __getattr__ is not an alias for the __getattribute__ wrapper.
                if matches!(def.name, "__new__" | "__getattr__")
                    || (def.name == "__hash__" && unhashable)
                {
                    return None;
                }
                let wrapped = def.accessor.get_slot_func_with_op(class.slots(), def.op)?;
                #[cfg(feature = "doc")]
                let (plain_off, plain_len) = match attr_doc(T::ATTR_DOCS, def.name) {
                    Some((offset, len)) if len != 0 => (offset, len),
                    _ => (0, 0),
                };
                #[cfg(not(feature = "doc"))]
                let (plain_off, plain_len) =
                    (0, u32::from(attr_name_present(T::ATTR_DOCS, def.name)));
                Some(NativeOperator {
                    name: def.name,
                    wrapped,
                    doc: def.doc,
                    plain_off,
                    plain_len,
                })
            })
            .collect();
        let new = class.slots().native_new.is_some_and(|slot_new| {
            let object_new = ctx.types.object_type.slots().new.load();
            core::ptr::eq(class, ctx.types.object_type)
                || object_new.is_none_or(|f| fn_addr(slot_new) != fn_addr(f))
        });
        Self {
            wrappers,
            new,
            unhashable,
        }
    }

    fn populate(&self, class: &Py<PyType>, ctx: &Context) {
        if self.new {
            let bound_new =
                ctx.slot_new_wrapper
                    .build_bound_method(ctx, class.to_owned().into(), class);
            class.set_attr(ctx.names.__new__, bound_new.into());
        }
        if self.unhashable {
            class.set_attr(ctx.names.__hash__, ctx.none());
        }
        for def in &self.wrappers {
            let name = ctx.intern_str(def.name);
            if class.attributes().contains(name) {
                continue;
            }
            let wrapper = PyWrapper {
                typ: class.to_owned(),
                name,
                wrapped: def.wrapped,
                doc: Some(def.doc),
                plain_off: def.plain_off,
                plain_len: def.plain_len,
            };
            class.set_attr(name, wrapper.into_ref(ctx).into());
        }
    }
}

pub trait StaticType {
    // Ideally, saving PyType is better than PyTypeRef
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn static_cell() -> &'static static_cell::StaticCell<PyTypeRef>;

    #[inline]
    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn static_metaclass() -> &'static Py<PyType> {
        unsafe { PyType::static_type() }
    }

    #[inline]
    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn static_baseclass() -> &'static Py<PyType> {
        unsafe { PyBaseObject::static_type() }
    }

    fn baseclass(_ctx: &Context) -> PyTypeRef {
        unsafe { Self::static_baseclass() }.to_owned()
    }

    #[inline]
    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn static_type() -> &'static Py<PyType> {
        #[cold]
        fn fail() -> ! {
            panic!(
                "static type has not been initialized. e.g. the native types defined in different module may be used before importing library."
            );
        }
        unsafe { Self::static_cell() }
            .get()
            .unwrap_or_else(|| fail())
    }

    #[must_use]
    fn init_manually(typ: PyTypeRef) -> &'static Py<PyType> {
        let cell = unsafe { Self::static_cell() };
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization from init_manually"));
        let typ = cell.get().unwrap();
        typ.as_object().make_immortal();
        typ
    }

    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn init_builtin_type() -> &'static Py<PyType>
    where
        Self: PyClassImpl,
    {
        let typ = unsafe { Self::create_static_type() };
        let cell = unsafe { Self::static_cell() };
        cell.set(typ)
            .unwrap_or_else(|_| panic!("double initialization of {}", Self::NAME));
        let typ = cell.get().unwrap();
        typ.as_object().make_immortal();
        typ
    }

    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn create_static_type() -> PyTypeRef
    where
        Self: PyClassImpl,
    {
        // inherit_special COPYVAL(tp_itemsize): the direct base, and only when
        // this type left the slot at 0. The base type object already exists.
        let mut slots = Self::make_slots();
        if slots.itemsize == 0 {
            slots.itemsize = unsafe { Self::static_baseclass() }.slots.itemsize;
        }
        PyType::new_static(
            unsafe { Self::static_baseclass() }.to_owned(),
            Default::default(),
            slots,
            unsafe { Self::static_metaclass() }.to_owned(),
        )
        .unwrap()
    }
}

pub trait PyClassDef: 'static {
    const NAME: &'static str;
    const MODULE_NAME: Option<&'static str>;
    const TP_NAME: &'static str;
    const DOC: ItemDoc = ItemDoc::NONE;
    /// Attribute name → database span, sorted by name.
    /// `(u32::MAX, 0)` is an explicit empty doc.
    #[cfg(feature = "doc")]
    const ATTR_DOCS: &'static [(&'static str, u32, u32)] = &[];
    /// Names that have a database doc, sorted. The text is not in this build.
    #[cfg(not(feature = "doc"))]
    const ATTR_DOCS: &'static [&'static str] = &[];
    const BASICSIZE: usize;
    const ITEMSIZE: usize = 0;
    const INTERPRETER_LOCAL: bool = false;
    const NATIVE_LAYOUT_ID: core::any::TypeId = core::any::TypeId::of::<Self>();
    const UNHASHABLE: bool = false;
    const MEMBERS: &'static [crate::builtins::descriptor::PyMemberSpec] = &[];

    fn assert_member_layout() {}

    // due to restriction of rust trait system, object.__base__ is None
    // but PyBaseObject::Base will be PyBaseObject.
    type Base: PyClassDef;
}

const fn cmp_str(left: &str, right: &str) -> i8 {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let n = if left.len() < right.len() {
        left.len()
    } else {
        right.len()
    };
    let mut i = 0;
    while i < n {
        if left[i] != right[i] {
            return if left[i] < right[i] { -1 } else { 1 };
        }
        i += 1;
    }
    if left.len() == right.len() {
        0
    } else if left.len() < right.len() {
        -1
    } else {
        1
    }
}

#[must_use]
pub const fn attr_name_present(table: &[&str], name: &str) -> bool {
    let mut lo = 0;
    let mut hi = table.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let ord = cmp_str(table[mid], name);
        if ord == 0 {
            return true;
        } else if ord < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    false
}

/// Database doc of attribute `name` of `T`.
#[must_use]
pub fn class_attr_item_doc<T: PyClassDef + ?Sized>(name: &str) -> ItemDoc {
    #[cfg(feature = "doc")]
    if let Some((offset, len)) = attr_doc(T::ATTR_DOCS, name) {
        if len != 0 {
            return ItemDoc {
                text: None,
                offset,
                len,
            };
        }
        if offset == u32::MAX {
            return ItemDoc::EMPTY;
        }
    }
    #[cfg(not(feature = "doc"))]
    let _ = name;
    ItemDoc::NONE
}

/// Set `doc` as `__doc__` of a native type that has none.
pub fn assign_missing_doc(vm: &VirtualMachine, class: &Py<PyType>, doc: ItemDoc) {
    let Some(text) = plain_doc(doc) else {
        return;
    };
    let doc_name = identifier!(vm, __doc__);
    let missing = class
        .attributes()
        .get(doc_name)
        .is_none_or(|value| value.is(&vm.ctx.none));
    if missing {
        class.set_attr(doc_name, vm.ctx.new_str(text).into());
    }
}

/// Doc for `name` in a sorted attribute-doc table.
#[must_use]
#[inline(never)]
pub const fn attr_doc(table: &[(&str, u32, u32)], name: &str) -> Option<(u32, u32)> {
    let mut lo = 0;
    let mut hi = table.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let ord = cmp_str(table[mid].0, name);
        if ord == 0 {
            return Some((table[mid].1, table[mid].2));
        } else if ord < 0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    None
}

pub trait PyClassImpl: PyClassDef {
    const TP_FLAGS: PyTypeFlags = PyTypeFlags::empty();

    /// Signature-bearing class doc. [`ItemDoc::NONE`] when the constructor has no signature.
    const INTERNAL_DOC: ItemDoc = ItemDoc::NONE;

    const METHOD_DEFS: &'static [PyMethodDef];

    fn impl_extend_class(ctx: &Context, class: &Py<PyType>);

    fn extend_slots(slots: &mut PyTypeSlots);

    fn extend_class(ctx: &Context, class: &Py<PyType>)
    where
        Self: Sized,
    {
        // Names must be available before a lazily populated namespace is read.
        let _ = ctx.intern_str(Self::NAME);
        let operators = NativeOperators::capture::<Self>(class, ctx);
        if class.heaptype_ext.is_none() {
            class
                .attributes()
                .define_native(crate::vm::native_types::NativeNamespaceDefinition {
                    class: class.to_owned(),
                    operators,
                    populate: Self::populate_namespace,
                });
        } else {
            Self::populate_namespace(ctx, class, &operators);
        }
        // Same walk as init_slots: a slot such as tp_init is copied only from
        // a base that defines it, so a static grandchild must see that base
        // in the MRO, not only its direct bases.
        let mro = {
            let guard = class.mro.read();
            guard[1..].to_vec()
        };
        for base in &mro {
            class.inherit_slots(base);
        }
    }

    #[doc(hidden)]
    fn populate_namespace(ctx: &Context, class: &Py<PyType>, operators: &NativeOperators)
    where
        Self: Sized,
    {
        #[cfg(debug_assertions)]
        debug_assert!(
            class
                .slots()
                .flags
                .contains(&PyTypeFlags::_CREATED_WITH_FLAGS)
        );

        if Self::TP_FLAGS.contains(&PyTypeFlags::HAS_DICT)
            && !Self::MEMBERS.iter().any(|member| member.name == "__dict__")
        {
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

        Self::assert_member_layout();
        for member in Self::MEMBERS {
            class.set_str_attr(
                member.name,
                ctx.new_member(
                    member.name,
                    member.kind,
                    member.offset,
                    member.flags,
                    class,
                    member.doc,
                ),
                ctx,
            );
        }

        Self::impl_extend_class(ctx, class);

        // Only set __doc__ if it doesn't already exist (e.g., as a member descriptor)
        // This matches CPython's behavior in type_dict_set_doc
        let doc_attr_name = identifier!(ctx, __doc__);
        if class.attributes().get(doc_attr_name).is_none() {
            // Local heap types store the same rendered text that static types
            // expose through type.__doc__, including native signature handling.
            let text = if Self::INTERPRETER_LOCAL {
                crate::builtins::type_::rendered_item_doc(Self::NAME, class.slots.doc)
            } else {
                plain_doc(Self::DOC)
            };
            let doc = text.map_or_else(|| ctx.none(), |doc| ctx.new_str(doc).into());
            class.set_attr(doc_attr_name, doc);
        }

        if let Some(module_name) = Self::MODULE_NAME {
            let module_key = identifier!(ctx, __module__);
            // Don't overwrite a getset descriptor for __module__ (e.g. TypeAliasType
            // has an instance-level __module__ getset that should not be replaced)
            let has_getset = class
                .attributes()
                .get(module_key)
                .is_some_and(|v| v.downcastable::<crate::builtins::PyGetSet>());
            if !has_getset {
                class.set_attr(module_key, ctx.new_str(module_name).into());
            }
        }

        operators.populate(class, ctx);

        class.extend_methods(class.slots().methods, ctx);
    }

    #[must_use]
    fn make_class(ctx: &Context) -> PyTypeRef
    where
        Self: StaticType + Sized + 'static,
    {
        if !Self::INTERPRETER_LOCAL {
            return unsafe { Self::make_static_type() };
        }
        crate::vm::thread::with_current_vm(|vm| {
            vm.state.native_types.class::<Self>(
                vm,
                || {
                    let base = Self::baseclass(ctx);
                    let mut slots = Self::make_slots();
                    if slots.itemsize == 0 {
                        slots.itemsize = base.slots.itemsize;
                    }
                    PyType::new_native_heap::<Self>(
                        Self::NAME,
                        base,
                        slots,
                        unsafe { Self::static_metaclass() }.to_owned(),
                        ctx,
                    )
                    .expect("invalid native type definition")
                },
                |class| Self::extend_class(ctx, class),
            )
        })
    }

    #[must_use]
    /// # Safety
    /// Raw types must be used under the native ownership and attachment
    /// contract in [`crate::embedding`], including during bootstrap.
    unsafe fn make_static_type() -> PyTypeRef
    where
        Self: StaticType + Sized,
    {
        assert!(
            !Self::INTERPRETER_LOCAL,
            "interpreter-local type requires make_class"
        );
        let typ = unsafe { Self::static_cell() }.get_or_init(|| {
            let _owner = crate::gc_state::AllocationScope::shared();
            let typ = unsafe { Self::create_static_type() };
            Self::extend_class(Context::genesis(), &typ);
            typ
        });
        // A static type is held by its `static_cell` for the life of the
        // process, so nothing is kept alive that would have died: all this
        // buys is that every reference to a builtin type from here on is a
        // branch rather than an atomic read-modify-write.
        typ.as_object().make_immortal();
        (*typ).to_owned()
    }

    fn make_slots() -> PyTypeSlots {
        let mut slots = PyTypeSlots {
            flags: crate::types::AtomicPyTypeFlags::from_plain(Self::TP_FLAGS),
            name: Self::TP_NAME,
            basicsize: Self::BASICSIZE,
            native_layout_id: Some(Self::NATIVE_LAYOUT_ID),
            itemsize: Self::ITEMSIZE,
            doc: {
                let internal = Self::INTERNAL_DOC;
                if internal.text.is_some() || internal.len != 0 {
                    internal
                } else {
                    Self::DOC
                }
            },
            methods: Self::METHOD_DEFS,
            ..Default::default()
        };

        if Self::UNHASHABLE {
            slots.hash.store(Some(hash_not_implemented));
        }

        Self::extend_slots(&mut slots);
        slots.native_new = slots.new.load();
        if !Self::INTERPRETER_LOCAL {
            // Shared native layouts and slots cannot be changed by an interpreter.
            slots.flags.insert(PyTypeFlags::IMMUTABLETYPE);
        }
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
