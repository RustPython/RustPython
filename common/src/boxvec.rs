//! An unresizable vector backed by a `Box<[T]>`

use std::borrow::{Borrow, BorrowMut};
use std::mem::{self, MaybeUninit};
use std::ops::{Bound, Deref, DerefMut, RangeBounds};
use std::{alloc, cmp, fmt, ptr, slice};

pub struct BoxVec<T> {
    xs: Box<[MaybeUninit<T>]>,
    len: usize,
}

impl<T> Drop for BoxVec<T> {
    fn drop(&mut self) {
        self.clear();

        // MaybeUninit inhibits array's drop
    }
}

macro_rules! panic_oob {
    ($method_name:expr, $index:expr, $len:expr) => {
        panic!(
            concat!(
                "BoxVec::",
                $method_name,
                ": index {} is out of bounds in vector of length {}"
            ),
            $index, $len
        )
    };
}

fn capacity_overflow() -> ! {
    panic!("capacity overflow")
}

impl<T> BoxVec<T> {
    pub fn new(n: usize) -> BoxVec<T> {
        unsafe {
            let layout = match alloc::Layout::array::<T>(n) {
                Ok(l) => l,
                Err(_) => capacity_overflow(),
            };
            let ptr = if mem::size_of::<T>() == 0 {
                ptr::NonNull::<MaybeUninit<T>>::dangling().as_ptr()
            } else {
                let ptr = alloc::alloc(layout);
                if ptr.is_null() {
                    alloc::handle_alloc_error(layout)
                }
                ptr as *mut MaybeUninit<T>
            };
            let ptr = ptr::slice_from_raw_parts_mut(ptr, n);
            let xs = Box::from_raw(ptr);
            BoxVec { xs, len: 0 }
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.xs.len()
    }

    pub fn is_full(&self) -> bool {
        self.len() == self.capacity()
    }

    pub fn remaining_capacity(&self) -> usize {
        self.capacity() - self.len()
    }

    pub fn push(&mut self, element: T) {
        self.try_push(element).unwrap()
    }

    pub fn try_push(&mut self, element: T) -> Result<(), CapacityError<T>> {
        if self.len() < self.capacity() {
            unsafe {
                self.push_unchecked(element);
            }
            Ok(())
        } else {
            Err(CapacityError::new(element))
        }
    }

    /// # Safety
    /// Must ensure that self.len() < self.capacity()
    pub unsafe fn push_unchecked(&mut self, element: T) {
        let len = self.len();
        debug_assert!(len < self.capacity());
        ptr::write(self.get_unchecked_ptr(len), element);
        self.set_len(len + 1);
    }

    /// Get pointer to where element at `index` would be
    unsafe fn get_unchecked_ptr(&mut self, index: usize) -> *mut T {
        self.xs.as_mut_ptr().add(index).cast()
    }

    pub fn insert(&mut self, index: usize, element: T) {
        self.try_insert(index, element).unwrap()
    }

    pub fn try_insert(&mut self, index: usize, element: T) -> Result<(), CapacityError<T>> {
        if index > self.len() {
            panic_oob!("try_insert", index, self.len())
        }
        if self.len() == self.capacity() {
            return Err(CapacityError::new(element));
        }
        let len = self.len();

        // follows is just like Vec<T>
        unsafe {
            // infallible
            // The spot to put the new value
            {
                let p: *mut _ = self.get_unchecked_ptr(index);
                // Shift everything over to make space. (Duplicating the
                // `index`th element into two consecutive places.)
                ptr::copy(p, p.offset(1), len - index);
                // Write it in, overwriting the first copy of the `index`th
                // element.
                ptr::write(p, element);
            }
            self.set_len(len + 1);
        }
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        unsafe {
            let new_len = self.len() - 1;
            self.set_len(new_len);
            Some(ptr::read(self.get_unchecked_ptr(new_len)))
        }
    }

    pub fn swap_remove(&mut self, index: usize) -> T {
        self.swap_pop(index)
            .unwrap_or_else(|| panic_oob!("swap_remove", index, self.len()))
    }

    pub fn swap_pop(&mut self, index: usize) -> Option<T> {
        let len = self.len();
        if index >= len {
            return None;
        }
        self.swap(index, len - 1);
        self.pop()
    }

    pub fn remove(&mut self, index: usize) -> T {
        self.pop_at(index)
            .unwrap_or_else(|| panic_oob!("remove", index, self.len()))
    }

    pub fn pop_at(&mut self, index: usize) -> Option<T> {
        if index >= self.len() {
            None
        } else {
            self.drain(index..index + 1).next()
        }
    }

    pub fn truncate(&mut self, new_len: usize) {
        unsafe {
            if new_len < self.len() {
                let tail: *mut [_] = &mut self[new_len..];
                self.len = new_len;
                ptr::drop_in_place(tail);
            }
        }
    }

    /// Remove all elements in the vector.
    pub fn clear(&mut self) {
        self.truncate(0)
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// In other words, remove all elements `e` such that `f(&mut e)` returns false.
    /// This method operates in place and preserves the order of the retained
    /// elements.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut T) -> bool,
    {
        let len = self.len();
        let mut del = 0;
        {
            let v = &mut **self;

            for i in 0..len {
                if !f(&mut v[i]) {
                    del += 1;
                } else if del > 0 {
                    v.swap(i - del, i);
                }
            }
        }
        if del > 0 {
            self.drain(len - del..);
        }
    }

    /// Set the vector’s length without dropping or moving out elements
    ///
    /// This method is `unsafe` because it changes the notion of the
    /// number of “valid” elements in the vector. Use with care.
    ///
    /// This method uses *debug assertions* to check that `length` is
    /// not greater than the capacity.
    ///
    /// # Safety
    /// Must ensure that length <= self.capacity()
    pub unsafe fn set_len(&mut self, length: usize) {
        debug_assert!(length <= self.capacity());
        self.len = length;
    }

    /// Copy and appends all elements in a slice to the `BoxVec`.
    ///
    /// # Errors
    ///
    /// This method will return an error if the capacity left (see
    /// [`remaining_capacity`]) is smaller then the length of the provided
    /// slice.
    ///
    /// [`remaining_capacity`]: #method.remaining_capacity
    pub fn try_extend_from_slice(&mut self, other: &[T]) -> Result<(), CapacityError>
    where
        T: Copy,
    {
        if self.remaining_capacity() < other.len() {
            return Err(CapacityError::new(()));
        }

        let self_len = self.len();
        let other_len = other.len();

        unsafe {
            let dst = self.as_mut_ptr().add(self_len);
            ptr::copy_nonoverlapping(other.as_ptr(), dst, other_len);
            self.set_len(self_len + other_len);
        }
        Ok(())
    }

    /// Create a draining iterator that removes the specified range in the vector
    /// and yields the removed items from start to end. The element range is
    /// removed even if the iterator is not consumed until the end.
    ///
    /// Note: It is unspecified how many elements are removed from the vector,
    /// if the `Drain` value is leaked.
    ///
    /// **Panics** if the starting point is greater than the end point or if
    /// the end point is greater than the length of the vector.
    pub fn drain<R>(&mut self, range: R) -> Drain<T>
    where
        R: RangeBounds<usize>,
    {
        // Memory safety
        //
        // When the Drain is first created, it shortens the length of
        // the source vector to make sure no uninitialized or moved-from elements
        // are accessible at all if the Drain's destructor never gets to run.
        //
        // Drain will ptr::read out the values to remove.
        // When finished, remaining tail of the vec is copied back to cover
        // the hole, and the vector length is restored to the new length.
        //
        let len = self.len();
        let start = match range.start_bound() {
            Bound::Unbounded => 0,
            Bound::Included(&i) => i,
            Bound::Excluded(&i) => i.saturating_add(1),
        };
        let end = match range.end_bound() {
            Bound::Excluded(&j) => j,
            Bound::Included(&j) => j.saturating_add(1),
            Bound::Unbounded => len,
        };
        self.drain_range(start, end)
    }

    fn drain_range(&mut self, start: usize, end: usize) -> Drain<T> {
        let len = self.len();

        // bounds check happens here (before length is changed!)
        let range_slice: *const _ = &self[start..end];

        // Calling `set_len` creates a fresh and thus unique mutable references, making all
        // older aliases we created invalid. So we cannot call that function.
        self.len = start;

        unsafe {
            Drain {
                tail_start: end,
                tail_len: len - end,
                iter: (*range_slice).iter(),
                vec: ptr::NonNull::from(self),
            }
        }
    }

    /// Return a slice containing all elements of the vector.
    pub fn as_slice(&self) -> &[T] {
        self
    }

    /// Return a mutable slice containing all elements of the vector.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        self
    }

    /// Return a raw pointer to the vector's buffer.
    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.xs.as_ptr().cast()
    }

    /// Return a raw mutable pointer to the vector's buffer.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.xs.as_mut_ptr().cast()
    }
}

impl<T> Deref for BoxVec<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        unsafe { slice::from_raw_parts(self.as_ptr(), self.len()) }
    }
}

impl<T> DerefMut for BoxVec<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        let len = self.len();
        unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), len) }
    }
}

/// Iterate the `BoxVec` with references to each element.
impl<'a, T> IntoIterator for &'a BoxVec<T> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterate the `BoxVec` with mutable references to each element.
impl<'a, T> IntoIterator for &'a mut BoxVec<T> {
    type Item = &'a mut T;
    type IntoIter = slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Iterate the `BoxVec` with each element by value.
///
/// The vector is consumed by this operation.
impl<T> IntoIterator for BoxVec<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;
    fn into_iter(self) -> IntoIter<T> {
        IntoIter { index: 0, v: self }
    }
}

/// By-value iterator for `BoxVec`.
pub struct IntoIter<T> {
    index: usize,
    v: BoxVec<T>,
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.index == self.v.len {
            None
        } else {
            unsafe {
                let index = self.index;
                self.index += 1;
                Some(ptr::read(self.v.get_unchecked_ptr(index)))
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.v.len() - self.index;
        (len, Some(len))
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        if self.index == self.v.len {
            None
        } else {
            unsafe {
                let new_len = self.v.len() - 1;
                self.v.set_len(new_len);
                Some(ptr::read(self.v.get_unchecked_ptr(new_len)))
            }
        }
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

impl<T> Drop for IntoIter<T> {
    fn drop(&mut self) {
        // panic safety: Set length to 0 before dropping elements.
        let index = self.index;
        let len = self.v.len();
        unsafe {
            self.v.set_len(0);
            let elements = slice::from_raw_parts_mut(self.v.get_unchecked_ptr(index), len - index);
            ptr::drop_in_place(elements);
        }
    }
}

impl<T> fmt::Debug for IntoIter<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(&self.v[self.index..]).finish()
    }
}

/// A draining iterator for `BoxVec`.
pub struct Drain<'a, T> {
    /// Index of tail to preserve
    tail_start: usize,
    /// Length of tail
    tail_len: usize,
    /// Current remaining range to remove
    iter: slice::Iter<'a, T>,
    vec: ptr::NonNull<BoxVec<T>>,
}

unsafe impl<'a, T: Sync> Sync for Drain<'a, T> {}
unsafe impl<'a, T: Send> Send for Drain<'a, T> {}

impl<T> Iterator for Drain<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter
            .next()
            .map(|elt| unsafe { ptr::read(elt as *const _) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T> DoubleEndedIterator for Drain<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.iter
            .next_back()
            .map(|elt| unsafe { ptr::read(elt as *const _) })
    }
}

impl<T> ExactSizeIterator for Drain<'_, T> {}

impl<'a, T> Drain<'a, T> {
    pub fn as_slice(&self) -> &'a [T] {
        self.iter.as_slice()
    }
}

impl<T> Drop for Drain<'_, T> {
    fn drop(&mut self) {
        // len is currently 0 so panicking while dropping will not cause a double drop.

        // exhaust self first
        while let Some(_) = self.next() {}

        if self.tail_len > 0 {
            unsafe {
                let source_vec = self.vec.as_mut();
                // memmove back untouched tail, update to new length
                let start = source_vec.len();
                let tail = self.tail_start;
                let src = source_vec.as_ptr().add(tail);
                let dst = source_vec.as_mut_ptr().add(start);
                ptr::copy(src, dst, self.tail_len);
                source_vec.set_len(start + self.tail_len);
            }
        }
    }
}

struct ScopeExitGuard<T, Data, F>
where
    F: FnMut(&Data, &mut T),
{
    value: T,
    data: Data,
    f: F,
}

impl<T, Data, F> Drop for ScopeExitGuard<T, Data, F>
where
    F: FnMut(&Data, &mut T),
{
    fn drop(&mut self) {
        (self.f)(&self.data, &mut self.value)
    }
}

/// Extend the `BoxVec` with an iterator.
///
/// Does not extract more items than there is space for. No error
/// occurs if there are more iterator elements.
impl<T> Extend<T> for BoxVec<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        let take = self.capacity() - self.len();
        unsafe {
            let len = self.len();
            let mut ptr = raw_ptr_add(self.as_mut_ptr(), len);
            let end_ptr = raw_ptr_add(ptr, take);
            // Keep the length in a separate variable, write it back on scope
            // exit. To help the compiler with alias analysis and stuff.
            // We update the length to handle panic in the iteration of the
            // user's iterator, without dropping any elements on the floor.
            let mut guard = ScopeExitGuard {
                value: &mut self.len,
                data: len,
                f: move |&len, self_len| {
                    **self_len = len;
                },
            };
            let mut iter = iter.into_iter();
            loop {
                if ptr == end_ptr {
                    break;
                }
                if let Some(elt) = iter.next() {
                    raw_ptr_write(ptr, elt);
                    ptr = raw_ptr_add(ptr, 1);
                    guard.data += 1;
                } else {
                    break;
                }
            }
        }
    }
}

/// Rawptr add but uses arithmetic distance for ZST
unsafe fn raw_ptr_add<T>(ptr: *mut T, offset: usize) -> *mut T {
    if mem::size_of::<T>() == 0 {
        // Special case for ZST
        (ptr as usize).wrapping_add(offset) as _
    } else {
        ptr.add(offset)
    }
}

unsafe fn raw_ptr_write<T>(ptr: *mut T, value: T) {
    if mem::size_of::<T>() == 0 {
        /* nothing */
    } else {
        ptr::write(ptr, value)
    }
}

impl<T> Clone for BoxVec<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        let mut new = BoxVec::new(self.capacity());
        new.extend(self.iter().cloned());
        new
    }

    fn clone_from(&mut self, rhs: &Self) {
        // recursive case for the common prefix
        let prefix = cmp::min(self.len(), rhs.len());
        self[..prefix].clone_from_slice(&rhs[..prefix]);

        if prefix < self.len() {
            // rhs was shorter
            for _ in 0..self.len() - prefix {
                self.pop();
            }
        } else {
            let rhs_elems = rhs[self.len()..].iter().cloned();
            self.extend(rhs_elems);
        }
    }
}

impl<T> PartialEq for BoxVec<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T> PartialEq<[T]> for BoxVec<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &[T]) -> bool {
        **self == *other
    }
}

impl<T> Eq for BoxVec<T> where T: Eq {}

impl<T> Borrow<[T]> for BoxVec<T> {
    fn borrow(&self) -> &[T] {
        self
    }
}

impl<T> BorrowMut<[T]> for BoxVec<T> {
    fn borrow_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T> AsRef<[T]> for BoxVec<T> {
    fn as_ref(&self) -> &[T] {
        self
    }
}

impl<T> AsMut<[T]> for BoxVec<T> {
    fn as_mut(&mut self) -> &mut [T] {
        self
    }
}

impl<T> fmt::Debug for BoxVec<T>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        (**self).fmt(f)
    }
}

/// Error value indicating insufficient capacity
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub struct CapacityError<T = ()> {
    element: T,
}

impl<T> CapacityError<T> {
    /// Create a new `CapacityError` from `element`.
    pub fn new(element: T) -> CapacityError<T> {
        CapacityError { element }
    }

    /// Extract the overflowing element
    pub fn element(self) -> T {
        self.element
    }

    /// Convert into a `CapacityError` that does not carry an element.
    pub fn simplify(self) -> CapacityError {
        CapacityError { element: () }
    }
}

const CAPERROR: &str = "insufficient capacity";

impl<T> std::error::Error for CapacityError<T> {}

impl<T> fmt::Display for CapacityError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", CAPERROR)
    }
}

impl<T> fmt::Debug for CapacityError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "capacity error: {}", CAPERROR)
    }
}
