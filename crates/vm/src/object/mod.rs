//! Essential types for object models
//!
//! +-------------------------+--------------+-----------------------+
//! |       Management        |       Typed      |      Untyped      |
//! +-------------------------+------------------+-------------------+
//! | Interpreter-independent | [`Py<T>`]        | [`PyObject`]      |
//! | Reference-counted       | [`PyRef<T>`]     | [`PyObjectRef`]   |
//! | Weak                    | [`PyWeakRef<T>`] | [`PyRef<PyWeak>`] |
//! +-------------------------+--------------+-----------------------+
//!
//! [`PyRef<PyWeak>`] may looking like to be called as PyObjectWeak by the rule,
//! but not to do to remember it is a PyRef object.
//!
//! # Reference ownership
//!
//! `PyObjectRef` is an alias for `PyRef<PyObject>`. Typed payload references,
//! erased references, union views, and typed tuple views share the same owning representation.
//!
//! ## Representation and invariants
//!
//! - `PyRef<T>` stores `NonNull<PyObject>` and a zero-sized
//!   `PhantomData<NonNull<Py<T>>>`. The latter preserves the previous typed pointer's
//!   covariance and auto-trait/`Unpin` shape. The transparent owner and its `Option`
//!   remain one pointer wide with pointer alignment.
//! - The sealed `object::PyRefTarget` selects `Deref::Target`: payload `T` maps to
//!   `Py<T>`, `PyObject` maps to itself, and immutable typed tuples map recursively
//!   to their existing typed `Py` view. Downstream safe code cannot register an
//!   unrelated layout. `PyObject` does not implement `PyPayload`.
//! - `PyAnySet` is an internal transparent `PyObject` view. Its owner is
//!   `PyRef<PyAnySet>`, whose construction validates set/frozenset (including
//!   subclasses). Payload access downcasts to the actual alternative; neither
//!   `Py<PyAnySet>` nor a shared set/frozenset payload layout is assumed.
//!   `ArgIterable` instead stores a cached iterator function beside its owner,
//!   so it remains a wrapper rather than an allocation view.
//! - [`PyTypedRefTarget`] admits non-erased element targets for typed tuple
//!   views, including union views and recursively typed tuples. It does not
//!   perform validation: `try_into_typed` checks every element before changing
//!   the view. Typed constructors instead receive already-typed owners.
//!   All tuple views retain erased element storage with transparent wrappers;
//!   typed slices reinterpret only the guaranteed transparent reference
//!   elements, never a generic `Vec` or `Box` representation. Private marshal
//!   initialization finishes before publication; published elements are immutable.
//! - The owner itself has no new generic bound. `Clone` and `Drop` share one
//!   implementation over the existing object header. Object allocation, vtable
//!   destruction, weakrefs, GC, and trashcan handling are unchanged.
//! - Typed erasure uses `T: PyRefTarget<Target = Py<T>>`. This excludes the erased
//!   type from the conversion blanket, avoiding overlap with Rust's identity
//!   `From<T> for T`, while retaining typed and nested tuple conversions.
//!   Union views provide their own checked conversions; `PyRef::into_object`
//!   erases any owning view without adjusting its strong count.
//! - Ownership transfers use `ManuallyDrop` or `forget`, without an extra incref or
//!   decrement. The existing raw APIs retain their ownership-transfer contracts.
//! - C-API typed output uses the same target equality. An erased owner supports
//!   `*mut PyObject`, but cannot produce the fictitious `*mut Py<PyObject>` output.
//!   Duplicate erased owner/member-layout/FFI implementations are removed.
//! - Feature-dependent `Send`/`Sync` behavior is retained. The non-threading erased
//!   owner is neither; the threading owner implements both.
//!
//! ## API compatibility considerations
//!
//! Existing payload `PyRef<T>` and `PyObjectRef` use remains supported, but aliasing
//! makes some inference less direct:
//!
//! - Distinct raw constructor signatures remain: erased
//!   `PyObjectRef::from_raw(NonNull<PyObject>)` and typed
//!   `PyRef::<T>::from_raw(*const Py<T>)`. Unqualified `PyRef::from_raw` can now be
//!   ambiguous and require the explicit type parameter.
//! - Associated-target dereferencing can require an explicit downcast type or a
//!   `PyRefTarget` equality bound in generic typed-view code. Arbitrary non-payload
//!   views no longer automatically gain typed dereferencing; supported internal
//!   views must be registered in the sealed selector.
//! - Rust `Debug` formatting now consistently uses the allocation's vtable rather
//!   than the statically selected payload's formatter. This is a Rust diagnostic
//!   difference, not a Python semantic assertion.

mod core;
mod ext;
mod payload;
pub(crate) mod qsbr;
mod traverse;
mod traverse_object;

pub use self::core::*;
pub use self::ext::*;
pub use self::payload::*;
pub use core::SIZEOF_PYOBJECT_HEAD;
pub(crate) use core::{GC_NO_OWNER, GC_PERMANENT, GC_REACHABLE, GC_UNTRACKED, GcLink, GcOwner};
pub use traverse::{MaybeTraverse, Traverse, TraverseFn};
