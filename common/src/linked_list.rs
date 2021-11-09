//! This module is modified from tokio::util::linked_list: https://github.com/tokio-rs/tokio/blob/master/tokio/src/util/linked_list.rs
//! Tokio is licensed under the MIT license:
//!
//! Copyright (c) 2021 Tokio Contributors
//!
//! Permission is hereby granted, free of charge, to any
//! person obtaining a copy of this software and associated
//! documentation files (the "Software"), to deal in the
//! Software without restriction, including without
//! limitation the rights to use, copy, modify, merge,
//! publish, distribute, sublicense, and/or sell copies of
//! the Software, and to permit persons to whom the Software
//! is furnished to do so, subject to the following
//! conditions:
//!
//! The above copyright notice and this permission notice
//! shall be included in all copies or substantial portions
//! of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
//! ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
//! TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
//! PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
//! SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
//! CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
//! OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
//! IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
//! DEALINGS IN THE SOFTWARE.
//!
//! Original header:
//!
//! An intrusive double linked list of data.
//!
//! The data structure supports tracking pinned nodes. Most of the data
//! structure's APIs are `unsafe` as they require the caller to ensure the
//! specified node is actually contained by the list.

#![allow(clippy::new_without_default, clippy::missing_safety_doc)]

use core::cell::UnsafeCell;
use core::fmt;
use core::marker::{PhantomData, PhantomPinned};
use core::mem::ManuallyDrop;
use core::ptr::{self, NonNull};

/// An intrusive linked list.
///
/// Currently, the list is not emptied on drop. It is the caller's
/// responsibility to ensure the list is empty before dropping it.
pub struct LinkedList<L, T> {
    /// Linked list head
    head: Option<NonNull<T>>,

    // /// Linked list tail
    // tail: Option<NonNull<T>>,
    /// Node type marker.
    _marker: PhantomData<*const L>,
}

unsafe impl<L: Link> Send for LinkedList<L, L::Target> where L::Target: Send {}
unsafe impl<L: Link> Sync for LinkedList<L, L::Target> where L::Target: Sync {}

/// Defines how a type is tracked within a linked list.
///
/// In order to support storing a single type within multiple lists, accessing
/// the list pointers is decoupled from the entry type.
///
/// # Safety
///
/// Implementations must guarantee that `Target` types are pinned in memory. In
/// other words, when a node is inserted, the value will not be moved as long as
/// it is stored in the list.
pub unsafe trait Link {
    /// Handle to the list entry.
    ///
    /// This is usually a pointer-ish type.
    type Handle;

    /// Node type.
    type Target;

    /// Convert the handle to a raw pointer without consuming the handle.
    #[allow(clippy::wrong_self_convention)]
    fn as_raw(handle: &Self::Handle) -> NonNull<Self::Target>;

    /// Convert the raw pointer to a handle
    unsafe fn from_raw(ptr: NonNull<Self::Target>) -> Self::Handle;

    /// Return the pointers for a node
    unsafe fn pointers(target: NonNull<Self::Target>) -> NonNull<Pointers<Self::Target>>;
}

/// Previous / next pointers.
pub struct Pointers<T> {
    inner: UnsafeCell<PointersInner<T>>,
}
/// We do not want the compiler to put the `noalias` attribute on mutable
/// references to this type, so the type has been made `!Unpin` with a
/// `PhantomPinned` field.
///
/// Additionally, we never access the `prev` or `next` fields directly, as any
/// such access would implicitly involve the creation of a reference to the
/// field, which we want to avoid since the fields are not `!Unpin`, and would
/// hence be given the `noalias` attribute if we were to do such an access.
/// As an alternative to accessing the fields directly, the `Pointers` type
/// provides getters and setters for the two fields, and those are implemented
/// using raw pointer casts and offsets, which is valid since the struct is
/// #[repr(C)].
///
/// See this link for more information:
/// <https://github.com/rust-lang/rust/pull/82834>
#[repr(C)]
struct PointersInner<T> {
    /// The previous node in the list. null if there is no previous node.
    ///
    /// This field is accessed through pointer manipulation, so it is not dead code.
    #[allow(dead_code)]
    prev: Option<NonNull<T>>,

    /// The next node in the list. null if there is no previous node.
    ///
    /// This field is accessed through pointer manipulation, so it is not dead code.
    #[allow(dead_code)]
    next: Option<NonNull<T>>,

    /// This type is !Unpin due to the heuristic from:
    /// <https://github.com/rust-lang/rust/pull/82834>
    _pin: PhantomPinned,
}

unsafe impl<T: Send> Send for Pointers<T> {}
unsafe impl<T: Sync> Sync for Pointers<T> {}

// ===== impl LinkedList =====

impl<L, T> LinkedList<L, T> {
    /// Creates an empty linked list.
    pub const fn new() -> LinkedList<L, T> {
        LinkedList {
            head: None,
            // tail: None,
            _marker: PhantomData,
        }
    }
}

impl<L: Link> LinkedList<L, L::Target> {
    /// Adds an element first in the list.
    pub fn push_front(&mut self, val: L::Handle) {
        // The value should not be dropped, it is being inserted into the list
        let val = ManuallyDrop::new(val);
        let ptr = L::as_raw(&*val);
        assert_ne!(self.head, Some(ptr));
        unsafe {
            L::pointers(ptr).as_mut().set_next(self.head);
            L::pointers(ptr).as_mut().set_prev(None);

            if let Some(head) = self.head {
                L::pointers(head).as_mut().set_prev(Some(ptr));
            }

            self.head = Some(ptr);

            // if self.tail.is_none() {
            //     self.tail = Some(ptr);
            // }
        }
    }

    // /// Removes the last element from a list and returns it, or None if it is
    // /// empty.
    // pub fn pop_back(&mut self) -> Option<L::Handle> {
    //     unsafe {
    //         let last = self.tail?;
    //         self.tail = L::pointers(last).as_ref().get_prev();

    //         if let Some(prev) = L::pointers(last).as_ref().get_prev() {
    //             L::pointers(prev).as_mut().set_next(None);
    //         } else {
    //             self.head = None
    //         }

    //         L::pointers(last).as_mut().set_prev(None);
    //         L::pointers(last).as_mut().set_next(None);

    //         Some(L::from_raw(last))
    //     }
    // }

    /// Returns whether the linked list does not contain any node
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
        // if self.head.is_some() {
        //     return false;
        // }

        // assert!(self.tail.is_none());
        // true
    }

    /// Removes the specified node from the list
    ///
    /// # Safety
    ///
    /// The caller **must** ensure that `node` is currently contained by
    /// `self` or not contained by any other list.
    pub unsafe fn remove(&mut self, node: NonNull<L::Target>) -> Option<L::Handle> {
        if let Some(prev) = L::pointers(node).as_ref().get_prev() {
            debug_assert_eq!(L::pointers(prev).as_ref().get_next(), Some(node));
            L::pointers(prev)
                .as_mut()
                .set_next(L::pointers(node).as_ref().get_next());
        } else {
            if self.head != Some(node) {
                return None;
            }

            self.head = L::pointers(node).as_ref().get_next();
        }

        if let Some(next) = L::pointers(node).as_ref().get_next() {
            debug_assert_eq!(L::pointers(next).as_ref().get_prev(), Some(node));
            L::pointers(next)
                .as_mut()
                .set_prev(L::pointers(node).as_ref().get_prev());
        } else {
            // // This might be the last item in the list
            // if self.tail != Some(node) {
            //     return None;
            // }

            // self.tail = L::pointers(node).as_ref().get_prev();
        }

        L::pointers(node).as_mut().set_next(None);
        L::pointers(node).as_mut().set_prev(None);

        Some(L::from_raw(node))
    }

    // pub fn last(&self) -> Option<&L::Target> {
    //     let tail = self.tail.as_ref()?;
    //     unsafe { Some(&*tail.as_ptr()) }
    // }

    // === rustpython additions ===

    pub fn iter(&self) -> impl Iterator<Item = &L::Target> {
        std::iter::successors(self.head, |node| unsafe {
            L::pointers(*node).as_ref().get_next()
        })
        .map(|ptr| unsafe { ptr.as_ref() })
    }
}

impl<L: Link> fmt::Debug for LinkedList<L, L::Target> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkedList")
            .field("head", &self.head)
            // .field("tail", &self.tail)
            .finish()
    }
}

impl<L: Link> Default for LinkedList<L, L::Target> {
    fn default() -> Self {
        Self::new()
    }
}

// ===== impl DrainFilter =====

pub struct DrainFilter<'a, T: Link, F> {
    list: &'a mut LinkedList<T, T::Target>,
    filter: F,
    curr: Option<NonNull<T::Target>>,
}

impl<T: Link> LinkedList<T, T::Target> {
    pub fn drain_filter<F>(&mut self, filter: F) -> DrainFilter<'_, T, F>
    where
        F: FnMut(&mut T::Target) -> bool,
    {
        let curr = self.head;
        DrainFilter {
            curr,
            filter,
            list: self,
        }
    }
}

impl<'a, T, F> Iterator for DrainFilter<'a, T, F>
where
    T: Link,
    F: FnMut(&mut T::Target) -> bool,
{
    type Item = T::Handle;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(curr) = self.curr {
            // safety: the pointer references data contained by the list
            self.curr = unsafe { T::pointers(curr).as_ref() }.get_next();

            // safety: the value is still owned by the linked list.
            if (self.filter)(unsafe { &mut *curr.as_ptr() }) {
                return unsafe { self.list.remove(curr) };
            }
        }

        None
    }
}

// ===== impl Pointers =====

impl<T> Pointers<T> {
    /// Create a new set of empty pointers
    pub fn new() -> Pointers<T> {
        Pointers {
            inner: UnsafeCell::new(PointersInner {
                prev: None,
                next: None,
                _pin: PhantomPinned,
            }),
        }
    }

    fn get_prev(&self) -> Option<NonNull<T>> {
        // SAFETY: prev is the first field in PointersInner, which is #[repr(C)].
        unsafe {
            let inner = self.inner.get();
            let prev = inner as *const Option<NonNull<T>>;
            ptr::read(prev)
        }
    }
    fn get_next(&self) -> Option<NonNull<T>> {
        // SAFETY: next is the second field in PointersInner, which is #[repr(C)].
        unsafe {
            let inner = self.inner.get();
            let prev = inner as *const Option<NonNull<T>>;
            let next = prev.add(1);
            ptr::read(next)
        }
    }

    fn set_prev(&mut self, value: Option<NonNull<T>>) {
        // SAFETY: prev is the first field in PointersInner, which is #[repr(C)].
        unsafe {
            let inner = self.inner.get();
            let prev = inner as *mut Option<NonNull<T>>;
            ptr::write(prev, value);
        }
    }
    fn set_next(&mut self, value: Option<NonNull<T>>) {
        // SAFETY: next is the second field in PointersInner, which is #[repr(C)].
        unsafe {
            let inner = self.inner.get();
            let prev = inner as *mut Option<NonNull<T>>;
            let next = prev.add(1);
            ptr::write(next, value);
        }
    }
}

impl<T> fmt::Debug for Pointers<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prev = self.get_prev();
        let next = self.get_next();
        f.debug_struct("Pointers")
            .field("prev", &prev)
            .field("next", &next)
            .finish()
    }
}
